//! Black-box contracts for the installable command.

#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet"))]

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn cli_writes_reviewed_output_through_the_public_conversion_path() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies");
    let output = TemporaryOutput::new("exact");
    let compose = fixture.join("compose.yaml");
    let result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "dependencies",
            "--podman-minimum-version",
            "5.4.0",
            "--podman-maximum-version",
            "6.0.2",
            "--loss-policy",
            "exact",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    for name in ["database.container", "cache.container", "web.container"] {
        assert_eq!(fs::read(output.path().join(name))?, fs::read(fixture.join(name))?);
    }
    Ok(())
}

#[test]
fn cli_refuses_to_overwrite_an_existing_output_directory() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies");
    let output = TemporaryOutput::new("existing");
    let compose = fixture.join("compose.yaml");
    fs::create_dir_all(output.path())?;
    let result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "dependencies",
            "--podman-maximum-version",
            "6.0.2",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("output directory already exists"));
    assert!(fs::read_dir(output.path())?.next().is_none());
    Ok(())
}

#[test]
fn cli_blocks_partial_output_before_creating_the_directory() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-core");
    let output = TemporaryOutput::new("blocked");
    let compose = fixture.join("compose.yaml");
    let override_file = fixture.join("compose.override.yaml");
    let result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--file",
            path_text(&override_file)?,
            "--project-name",
            "core",
            "--podman-maximum-version",
            "6.0.2",
            "--loss-policy",
            "exact",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("output blocked by the selected loss policy"));
    assert!(!output.path().exists());
    Ok(())
}

fn boxferry_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_boxferry"))
}

fn fixture_directory(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/conversion")
        .join(name)
}

fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| format!("test path is not UTF-8: {}", path.display()).into())
}

struct TemporaryOutput {
    path: PathBuf,
}

impl TemporaryOutput {
    fn new(label: &str) -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            path: std::env::temp_dir().join(format!("boxferry-cli-{}-{label}-{id}", std::process::id())),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if self.path.is_dir() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
