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

#[test]
fn cli_interpolates_only_explicitly_authorized_values() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-interpolation");
    let output = TemporaryOutput::new("interpolation");
    let compose = fixture.join("compose.yaml");
    let result = boxferry_command()
        .env("TOKEN", "explicit-secret")
        .env("UNAUTHORIZED", "ambient-secret-must-not-be-used")
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "interpolation",
            "--interpolate",
            "--variable",
            "TAG=2.1",
            "--variable-from-environment",
            "TOKEN",
            "--podman-maximum-version",
            "6.0.2",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        fs::read(output.path().join("app.container"))?,
        fs::read(fixture.join("app.container"))?
    );
    assert!(!String::from_utf8_lossy(&result.stderr).contains("ambient-secret-must-not-be-used"));
    Ok(())
}

#[test]
fn cli_never_interpolates_ambient_values_without_opt_in() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-interpolation");
    let output = TemporaryOutput::new("no-ambient-interpolation");
    let compose = fixture.join("compose.yaml");
    let result = boxferry_command()
        .env("TAG", "ambient-tag-must-not-be-used")
        .env("TOKEN", "ambient-token-must-not-be-used")
        .env("RESTART_POLICY", "always")
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "interpolation",
            "--podman-maximum-version",
            "6.0.2",
            "--loss-policy",
            "partial",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!stderr.contains("ambient-tag-must-not-be-used"));
    assert!(!stderr.contains("ambient-token-must-not-be-used"));
    assert!(!output.path().exists());
    Ok(())
}

#[test]
fn cli_fails_closed_when_an_authorized_process_variable_is_missing() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-interpolation");
    let output = TemporaryOutput::new("missing-interpolation-variable");
    let compose = fixture.join("compose.yaml");
    let variable = "BOXFERRY_TEST_MISSING_INTERPOLATION_VALUE";
    let result = boxferry_command()
        .env_remove(variable)
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "interpolation",
            "--interpolate",
            "--variable",
            "TAG=2.1",
            "--variable-from-environment",
            variable,
            "--podman-maximum-version",
            "6.0.2",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains(variable));
    assert!(!output.path().exists());
    Ok(())
}

#[test]
fn cli_rejects_duplicate_interpolation_sources() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-interpolation");
    let output = TemporaryOutput::new("duplicate-interpolation-variable");
    let compose = fixture.join("compose.yaml");
    let result = boxferry_command()
        .env("TAG", "ignored")
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "interpolation",
            "--interpolate",
            "--variable",
            "TAG=2.1",
            "--variable-from-environment",
            "TAG",
            "--podman-maximum-version",
            "6.0.2",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("`TAG` was supplied more than once"));
    assert!(!output.path().exists());
    Ok(())
}

#[test]
fn cli_converts_environment_file_declarations_without_reading_missing_files() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("environment-file-project");
    let output = TemporaryOutput::new("environment-file-output");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "services:\n",
            "  web:\n",
            "    image: example.invalid/web:1\n",
            "    env_file:\n",
            "      - ./missing-but-declared.env\n",
        ),
    )?;

    let result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&compose)?,
            "--project-name",
            "environment-files",
            "--podman-maximum-version",
            "6.0.2",
            "--loss-policy",
            "approximate",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;

    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let container = fs::read_to_string(output.path().join("web.container"))?;
    assert_eq!(
        container,
        format!(
            "[Container]\nImage=example.invalid/web:1\nEnvironmentFile={}/missing-but-declared.env\n",
            path_text(project.path())?,
        )
    );
    assert!(!project.path().join("missing-but-declared.env").exists());
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
