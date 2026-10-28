//! Build inert, volume-only consumer artifacts from six embedded authored fixtures.
//!
//! This validation-only example neither applies requests nor proves runtime acceptance or
//! candidate admission. The receipt argument binds the output to separately reviewed evidence.
//! Native context remains in each unmodified complete artifact; the manifest claims volume scope
//! only. A failure may leave private partial output, but never a success manifest written early.
//! Output paths must have no symlink components. This precheck is not an atomic ancestry proof;
//! the inspected directory's device/inode binding and pinned descriptor still protect writes
//! from a later rename/replacement of the final directory path.

#[cfg(target_os = "linux")]
#[path = "../tests/support/docker_volume_fixtures.rs"]
mod fixtures;

#[cfg(target_os = "linux")]
mod producer {
    use std::{
        collections::BTreeSet,
        fs::{self, DirBuilder, File, OpenOptions},
        io::Write,
        os::{
            fd::AsRawFd,
            unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
        path::{Component, Path, PathBuf},
    };

    use serde_json::{Value, json};

    use super::fixtures;

    const LANES: [&str; 4] = [
        "debian11-rootful",
        "debian11-rootless",
        "upstream-rootful",
        "upstream-rootless",
    ];

    #[derive(Debug, PartialEq, Eq)]
    pub enum Failure {
        Arguments,
        Profile,
        Fixture,
        Manifest,
        OutputDirectory,
        OutputWrite,
    }

    impl Failure {
        pub const fn code(&self) -> &'static str {
            match self {
                Self::Arguments => "arguments-rejected",
                Self::Profile => "reviewed-profile-unavailable",
                Self::Fixture => "volume-fixture-rejected",
                Self::Manifest => "manifest-rejected",
                Self::OutputDirectory => "private-output-directory-rejected",
                Self::OutputWrite => "private-output-write-failed",
            }
        }
    }

    #[derive(Debug)]
    struct Arguments {
        lane: String,
        run: String,
        prefix: String,
        receipt: String,
        output: PathBuf,
    }

    fn safe_token(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 64
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }

    fn sha256_token(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn parse(arguments: &[String]) -> Result<Arguments, Failure> {
        if arguments.len() != 10 || arguments.iter().any(|argument| argument.len() > 4096) {
            return Err(Failure::Arguments);
        }
        let mut seen = BTreeSet::new();
        let (mut lane, mut run, mut prefix, mut receipt, mut output) = (None, None, None, None, None);
        for pair in arguments.as_chunks::<2>().0 {
            if !seen.insert(pair[0].as_str()) {
                return Err(Failure::Arguments);
            }
            let value = pair[1].clone();
            match pair[0].as_str() {
                "--lane" if LANES.contains(&value.as_str()) => lane = Some(value),
                "--run" if safe_token(&value) => run = Some(value),
                "--prefix" if safe_token(&value) => prefix = Some(value),
                "--receipt-sha256" if sha256_token(&value) => receipt = Some(value),
                "--output-directory" => {
                    if value.contains('\0') {
                        return Err(Failure::Arguments);
                    }
                    let path = PathBuf::from(value);
                    if !path.is_absolute()
                        || path.components().count() < 2
                        || path
                            .components()
                            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
                    {
                        return Err(Failure::Arguments);
                    }
                    output = Some(path);
                }
                _ => return Err(Failure::Arguments),
            }
        }
        Ok(Arguments {
            lane: lane.ok_or(Failure::Arguments)?,
            run: run.ok_or(Failure::Arguments)?,
            prefix: prefix.ok_or(Failure::Arguments)?,
            receipt: receipt.ok_or(Failure::Arguments)?,
            output: output.ok_or(Failure::Arguments)?,
        })
    }

    struct PrivateDirectory {
        handle: File,
        uid: u32,
    }

    fn reject_symlink_ancestors(path: &Path) -> Result<(), Failure> {
        if !path.is_absolute() {
            return Err(Failure::OutputDirectory);
        }
        let mut ancestor = PathBuf::new();
        for component in path.components() {
            if !matches!(component, Component::RootDir | Component::Normal(_)) {
                return Err(Failure::OutputDirectory);
            }
            ancestor.push(component.as_os_str());
            match fs::symlink_metadata(&ancestor) {
                Ok(metadata) if metadata.is_dir() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && ancestor == path => {}
                _ => return Err(Failure::OutputDirectory),
            }
        }
        Ok(())
    }

    impl PrivateDirectory {
        fn open(path: &Path) -> Result<Self, Failure> {
            reject_symlink_ancestors(path)?;
            let uid = fs::metadata("/proc/self").map_err(|_| Failure::OutputDirectory)?.uid();
            let before = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    DirBuilder::new()
                        .mode(0o700)
                        .create(path)
                        .map_err(|_| Failure::OutputDirectory)?;
                    fs::symlink_metadata(path).map_err(|_| Failure::OutputDirectory)?
                }
                Err(_) => return Err(Failure::OutputDirectory),
            };
            if !before.is_dir() || before.uid() != uid || before.mode() & 0o7777 != 0o700 {
                return Err(Failure::OutputDirectory);
            }
            let handle = File::open(path).map_err(|_| Failure::OutputDirectory)?;
            let opened = handle.metadata().map_err(|_| Failure::OutputDirectory)?;
            if opened.dev() != before.dev() || opened.ino() != before.ino() {
                return Err(Failure::OutputDirectory);
            }
            let directory = Self { handle, uid };
            directory.validate()?;
            if fs::read_dir(directory.pinned_path())
                .map_err(|_| Failure::OutputDirectory)?
                .next()
                .is_some()
            {
                return Err(Failure::OutputDirectory);
            }
            Ok(directory)
        }

        fn pinned_path(&self) -> PathBuf {
            // Keep the inspected directory open: subsequent filename creation does not re-resolve
            // the caller's path if that path is exchanged or renamed after validation.
            PathBuf::from(format!("/proc/self/fd/{}", self.handle.as_raw_fd()))
        }

        fn validate(&self) -> Result<(), Failure> {
            let metadata = self.handle.metadata().map_err(|_| Failure::OutputDirectory)?;
            if !metadata.is_dir() || metadata.uid() != self.uid || metadata.mode() & 0o7777 != 0o700 {
                return Err(Failure::OutputDirectory);
            }
            Ok(())
        }

        fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), Failure> {
            self.validate()?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(self.pinned_path().join(name))
                .map_err(|_| Failure::OutputWrite)?;
            let metadata = file.metadata().map_err(|_| Failure::OutputWrite)?;
            if !metadata.is_file() || metadata.uid() != self.uid || metadata.mode() & 0o7777 != 0o600 {
                return Err(Failure::OutputWrite);
            }
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| Failure::OutputWrite)?;
            self.validate()
        }
    }

    fn manifest(arguments: &Arguments, rows: &[Value]) -> Result<Vec<u8>, Failure> {
        if rows.len() != 6 {
            return Err(Failure::Manifest);
        }
        let value = json!({
            "schema": 1,
            "scope": "volume-only",
            "lane": arguments.lane,
            "run": arguments.run,
            "prefix": arguments.prefix,
            "candidate_receipt_sha256": arguments.receipt,
            "fixtures": rows,
        });
        let mut bytes = serde_json::to_vec_pretty(&value).map_err(|_| Failure::Manifest)?;
        bytes.push(b'\n');
        if bytes.len() > 16_384 {
            return Err(Failure::Manifest);
        }
        Ok(bytes)
    }

    pub fn run(arguments: &[String]) -> Result<(), Failure> {
        let arguments = parse(arguments)?;
        let profile = fixtures::selected_profile(&arguments.lane).map_err(|_| Failure::Profile)?;
        let mut artifacts = Vec::with_capacity(6);
        let mut rows = Vec::with_capacity(6);
        for id in [
            "forgejo",
            "nextcloud",
            "paperless-ngx",
            "immich",
            "observability",
            "supabase",
        ] {
            let fixture = fixtures::fixture(id).map_err(|_| Failure::Fixture)?;
            let bytes = fixtures::render_fixture(fixture, &arguments.prefix, &arguments.run, &profile)
                .map_err(|_| Failure::Fixture)?;
            let name = format!("{}-volumes.json", fixture.id);
            rows.push(json!({
                "id": fixture.id,
                "source_sha256": fixtures::sha256(fixture.template.as_bytes()),
                "artifact": name,
                "artifact_sha256": fixtures::sha256(&bytes),
                "volume_count": fixture.volume_count,
            }));
            artifacts.push((name, bytes));
        }
        // All native artifacts and the complete manifest are ready before filesystem mutation.
        let manifest = manifest(&arguments, &rows)?;
        let directory = PrivateDirectory::open(&arguments.output)?;
        for (name, bytes) in artifacts {
            directory.write_new(&name, &bytes)?;
        }
        directory.write_new("manifest.json", &manifest)
    }

    #[cfg(test)]
    mod tests {
        use std::{
            fs::{self, DirBuilder},
            os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink},
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };

        use super::{Failure, PrivateDirectory, manifest, parse, reject_symlink_ancestors, safe_token, sha256_token};
        use serde_json::json;

        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

        struct Scratch {
            path: PathBuf,
            device: u64,
            inode: u64,
        }

        impl Scratch {
            fn new() -> Result<Self, Failure> {
                for _ in 0..64 {
                    let path = PathBuf::from("/tmp").join(format!(
                        "boxferry-volume-producer-{}-{}",
                        std::process::id(),
                        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
                    ));
                    match DirBuilder::new().mode(0o700).create(&path) {
                        Ok(()) => {
                            let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::OutputDirectory)?;
                            return Ok(Self {
                                path,
                                device: metadata.dev(),
                                inode: metadata.ino(),
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(_) => return Err(Failure::OutputDirectory),
                    }
                }
                Err(Failure::OutputDirectory)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                // Exact known test entries only: no recursive deletion, symlink following, or
                // adoption of another directory after a replacement of the scratch path.
                let Ok(metadata) = fs::symlink_metadata(&self.path) else {
                    return;
                };
                if !metadata.is_dir() || metadata.dev() != self.device || metadata.ino() != self.inode {
                    return;
                }
                for name in [
                    "forgejo-volumes.json",
                    "nextcloud-volumes.json",
                    "artifacts/existing",
                    "original/forgejo-volumes.json",
                    "renamed/forgejo-volumes.json",
                    "alias",
                    "linked-parent",
                    "final-link",
                ] {
                    let entry = self.path.join(name);
                    if entry
                        .parent()
                        .is_some_and(|parent| reject_symlink_ancestors(parent).is_ok())
                    {
                        let _ = fs::remove_file(entry);
                    }
                }
                for name in [
                    "real-parent/artifacts",
                    "real-parent",
                    "artifacts",
                    "original",
                    "renamed",
                ] {
                    let entry = self.path.join(name);
                    if entry
                        .parent()
                        .is_some_and(|parent| reject_symlink_ancestors(parent).is_ok())
                    {
                        let _ = fs::remove_dir(entry);
                    }
                }
                let _ = fs::remove_dir(&self.path);
            }
        }

        fn arguments() -> Vec<String> {
            [
                "--lane",
                "upstream-rootless",
                "--run",
                "offline-run",
                "--prefix",
                "offline",
                "--receipt-sha256",
                &"a".repeat(64),
                "--output-directory",
                "/tmp/private-volume-output",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        }

        #[test]
        fn arguments_are_closed_bounded_and_do_not_authorize_unsafe_tokens() {
            assert!(parse(&arguments()).is_ok());
            for token in ["", "1run", "Run", "run_name", "run/name", "run\n", &"a".repeat(65)] {
                assert!(!safe_token(token));
            }
            assert!(safe_token(&"a".repeat(64)));
            assert!(sha256_token(&"a".repeat(64)));
            for receipt in ["", &"A".repeat(64), &"g".repeat(64), &"a".repeat(63)] {
                assert!(!sha256_token(receipt));
            }
            for (index, value) in [
                (0, "--unknown"),
                (1, "other-lane"),
                (2, "--lane"),
                (9, "relative/path"),
                (9, "/"),
                (9, "/tmp/../private"),
            ] {
                let mut args = arguments();
                args[index] = value.to_owned();
                assert!(matches!(parse(&args), Err(Failure::Arguments)));
            }
            let mut extra = arguments();
            extra.push("--extra".to_owned());
            assert!(matches!(parse(&extra), Err(Failure::Arguments)));
        }

        #[test]
        fn manifest_has_only_the_assigned_volume_scope_fields() -> Result<(), Failure> {
            let args = parse(&arguments())?;
            let rows: Vec<_> = [
                ("forgejo", 2),
                ("nextcloud", 3),
                ("paperless-ngx", 6),
                ("immich", 4),
                ("observability", 5),
                ("supabase", 3),
            ]
            .into_iter()
            .map(|(id, count)| {
                json!({"id": id, "source_sha256": "b".repeat(64),
                    "artifact": format!("{id}-volumes.json"), "artifact_sha256": "c".repeat(64), "volume_count": count})
            })
            .collect();
            assert!(matches!(manifest(&args, &rows[..5]), Err(Failure::Manifest)));
            let raw = manifest(&args, &rows)?;
            let actual: serde_json::Value = serde_json::from_slice(&raw).map_err(|_| Failure::Manifest)?;
            assert_eq!(
                actual,
                json!({"schema": 1, "scope": "volume-only", "lane": "upstream-rootless",
                "run": "offline-run", "prefix": "offline", "candidate_receipt_sha256": "a".repeat(64), "fixtures": rows})
            );
            Ok(())
        }

        #[test]
        fn private_files_are_create_new_and_existing_symlinks_are_not_followed() -> Result<(), Failure> {
            let scratch = Scratch::new()?;
            let directory = PrivateDirectory::open(&scratch.path)?;
            directory.write_new("forgejo-volumes.json", b"original")?;
            let path = scratch.path.join("forgejo-volumes.json");
            let metadata = fs::metadata(&path).map_err(|_| Failure::OutputWrite)?;
            assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
            assert!(matches!(
                directory.write_new("forgejo-volumes.json", b"replacement"),
                Err(Failure::OutputWrite)
            ));
            let alias = scratch.path.join("nextcloud-volumes.json");
            symlink(&path, &alias).map_err(|_| Failure::OutputWrite)?;
            assert!(matches!(
                directory.write_new("nextcloud-volumes.json", b"replacement"),
                Err(Failure::OutputWrite)
            ));
            assert_eq!(fs::read(path).map_err(|_| Failure::OutputWrite)?, b"original");
            Ok(())
        }

        #[test]
        fn absent_empty_private_directories_are_allowed_but_nonempty_or_public_ones_are_not() -> Result<(), Failure> {
            let scratch = Scratch::new()?;
            let path = scratch.path.join("artifacts");
            let directory = PrivateDirectory::open(&path)?;
            assert_eq!(
                fs::metadata(&path)
                    .map_err(|_| Failure::OutputDirectory)?
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700
            );
            assert!(PrivateDirectory::open(&path).is_ok());
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|_| Failure::OutputDirectory)?;
            assert!(matches!(PrivateDirectory::open(&path), Err(Failure::OutputDirectory)));
            assert_eq!(
                fs::metadata(&path)
                    .map_err(|_| Failure::OutputDirectory)?
                    .permissions()
                    .mode()
                    & 0o7777,
                0o755
            );
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|_| Failure::OutputDirectory)?;
            directory.write_new("existing", b"retained")?;
            assert!(matches!(PrivateDirectory::open(&path), Err(Failure::OutputDirectory)));
            let alias = scratch.path.join("alias");
            symlink(&path, &alias).map_err(|_| Failure::OutputDirectory)?;
            assert!(matches!(PrivateDirectory::open(&alias), Err(Failure::OutputDirectory)));
            Ok(())
        }

        #[test]
        fn final_and_ancestor_symlinks_are_refused_even_when_their_targets_are_empty_private_directories()
        -> Result<(), Failure> {
            let scratch = Scratch::new()?;
            let parent = scratch.path.join("real-parent");
            let destination = parent.join("artifacts");
            DirBuilder::new()
                .mode(0o700)
                .create(&parent)
                .map_err(|_| Failure::OutputDirectory)?;
            DirBuilder::new()
                .mode(0o700)
                .create(&destination)
                .map_err(|_| Failure::OutputDirectory)?;
            let final_link = scratch.path.join("final-link");
            symlink(&destination, &final_link).map_err(|_| Failure::OutputDirectory)?;
            assert!(matches!(
                PrivateDirectory::open(&final_link),
                Err(Failure::OutputDirectory)
            ));
            let ancestor_link = scratch.path.join("linked-parent");
            symlink(&parent, &ancestor_link).map_err(|_| Failure::OutputDirectory)?;
            assert!(matches!(
                PrivateDirectory::open(&ancestor_link.join("artifacts")),
                Err(Failure::OutputDirectory)
            ));
            assert!(matches!(
                PrivateDirectory::open(&ancestor_link.join("absent")),
                Err(Failure::OutputDirectory)
            ));
            assert!(!parent.join("absent").exists());
            assert!(
                fs::read_dir(destination)
                    .map_err(|_| Failure::OutputDirectory)?
                    .next()
                    .is_none()
            );
            Ok(())
        }

        #[test]
        fn pinned_directory_writes_survive_rename_without_writing_into_the_replacement() -> Result<(), Failure> {
            let scratch = Scratch::new()?;
            let original = scratch.path.join("original");
            let renamed = scratch.path.join("renamed");
            let directory = PrivateDirectory::open(&original)?;
            fs::rename(&original, &renamed).map_err(|_| Failure::OutputDirectory)?;
            DirBuilder::new()
                .mode(0o700)
                .create(&original)
                .map_err(|_| Failure::OutputDirectory)?;
            directory.write_new("forgejo-volumes.json", b"pinned-original")?;
            assert_eq!(
                fs::read(renamed.join("forgejo-volumes.json")).map_err(|_| Failure::OutputWrite)?,
                b"pinned-original"
            );
            assert!(!original.join("forgejo-volumes.json").exists());
            assert!(
                fs::read_dir(original)
                    .map_err(|_| Failure::OutputDirectory)?
                    .next()
                    .is_none()
            );
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    // Native errors, authored values, and caller paths never reach the console, including panics.
    std::panic::set_hook(Box::new(|_| {
        eprintln!("docker-volume-fixture-rehearsal: internal-failure");
    }));
    let arguments = std::env::args_os()
        .skip(1)
        .take(11)
        .map(|value| {
            if value.as_encoded_bytes().len() > 4096 {
                return Err(producer::Failure::Arguments);
            }
            value.into_string().map_err(|_| producer::Failure::Arguments)
        })
        .collect::<Result<Vec<_>, _>>();
    let result = arguments.and_then(|arguments| producer::run(&arguments));
    match result {
        Ok(()) => {
            eprintln!("docker-volume-fixture-rehearsal: artifacts-written");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("docker-volume-fixture-rehearsal: {}", error.code());
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("docker-volume-fixture-rehearsal: unsupported-platform");
    std::process::ExitCode::FAILURE
}
