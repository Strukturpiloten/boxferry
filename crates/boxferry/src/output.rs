//! Shared create-new artifact writes with invocation-owned cleanup.
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Borrowed bytes of one named output artifact. Debug never prints contents.
#[derive(Clone, Copy)]
pub struct OutputFile<'a> {
    name: &'a str,
    bytes: &'a [u8],
}
impl<'a> OutputFile<'a> {
    /// Describes an artifact. The complete set is validated before writing.
    #[must_use]
    pub const fn new(name: &'a str, bytes: &'a [u8]) -> Self {
        Self { name, bytes }
    }
}
impl std::fmt::Debug for OutputFile<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputFile")
            .field("name", &self.name)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Writes an entire validated artifact set into an absent or empty non-symlink directory.
///
/// Every file is create-new. On failure only invocation-created files are removed, and the
/// directory is removed only if this invocation created it. Existing empty directories retain
/// their permissions. On Unix, `private` gives new directories mode 0700 and files mode 0600.
/// This path-based writer does not claim protection against hostile directory replacement races.
///
/// # Errors
/// Rejects unsafe/duplicate basenames, nonempty/symlink destinations and I/O failures.
pub fn write_new_files(directory: &Path, files: &[OutputFile<'_>], private: bool) -> io::Result<()> {
    let mut names = BTreeSet::new();
    for file in files {
        if !safe_basename(file.name) || !names.insert(file.name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "output artifact names must be unique safe basenames",
            ));
        }
    }
    let created_directory = prepare(directory, private)?;
    let mut created = Vec::<PathBuf>::with_capacity(files.len());
    for file in files {
        let path = directory.join(file.name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            options.mode(0o600);
        }
        let result = options.open(&path).and_then(|mut destination| {
            created.push(path);
            destination.write_all(file.bytes)
        });
        if let Err(error) = result {
            for path in created {
                let _ = fs::remove_file(path);
            }
            if created_directory {
                let _ = fs::remove_dir(directory);
            }
            return Err(error);
        }
    }
    Ok(())
}
pub(crate) fn safe_basename(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}
fn prepare(directory: &Path, private: bool) -> io::Result<bool> {
    #[cfg(unix)]
    let creation = if private {
        fs::DirBuilder::new().mode(0o700).create(directory)
    } else {
        fs::create_dir(directory)
    };
    #[cfg(not(unix))]
    let creation = {
        let _ = private;
        fs::create_dir(directory)
    };
    match creation {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(directory)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output path is not a non-symlink directory",
                ));
            }
            if fs::read_dir(directory)?.next().transpose()?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "output directory is not empty",
                ));
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
