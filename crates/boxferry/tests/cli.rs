//! Black-box contracts for the installable command.

#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet"))]

use std::{
    error::Error,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
use zip::{CompressionMethod, ZipArchive};

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
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("BFC0018"));
    assert!(stderr.contains("boxferry convert --input-type compose --output-type quadlet"));
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
fn legacy_preprocessing_failures_keep_the_deprecation_and_policy_exit_contract() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("legacy-preprocessing-failures");
    fs::create_dir_all(project.path())?;
    let malformed = project.path().join("malformed.yaml");
    fs::write(&malformed, "services: [broken\n")?;
    let malformed_result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&malformed)?,
            "--project-name",
            "legacy",
            "--output-directory",
            path_text(&project.path().join("malformed-output"))?,
        ])
        .output()?;
    assert_eq!(malformed_result.status.code(), Some(2));
    let malformed_stderr = String::from_utf8_lossy(&malformed_result.stderr);
    assert_eq!(malformed_stderr.matches("BFC0018").count(), 1);
    assert!(malformed_stderr.contains("compose.yaml.unclosed-flow-sequence"));
    assert!(!malformed_stderr.contains("Compose preprocessing failed"));

    let invalid_profile = project.path().join("invalid-profile.yaml");
    fs::write(
        &invalid_profile,
        "services:\n  app:\n    image: example.invalid/app:1\n    profiles: [only]\n",
    )?;
    let profile_result = boxferry_command()
        .args([
            "compose-to-quadlet",
            "--file",
            path_text(&invalid_profile)?,
            "--project-name",
            "legacy",
            "--profile",
            "@invalid",
            "--output-directory",
            path_text(&project.path().join("profile-output"))?,
        ])
        .output()?;
    assert_eq!(profile_result.status.code(), Some(2));
    let profile_stderr = String::from_utf8_lossy(&profile_result.stderr);
    assert_eq!(profile_stderr.matches("BFC0018").count(), 1);
    assert!(profile_stderr.contains("compose.profiles.invalid-name"));
    assert!(profile_stderr.contains("active profile name does not follow the Compose profile grammar"));
    assert!(!profile_stderr.contains("Compose preprocessing failed"));
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

#[test]
fn generic_convert_preserves_mixed_input_occurrence_order_and_directory_priority() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("mixed-inputs");
    let output = TemporaryOutput::new("mixed-output");
    fs::create_dir_all(project.path())?;
    fs::write(
        project.path().join("compose.yaml"),
        "name: ordered\nservices:\n  web:\n    image: example.invalid/web:1\n",
    )?;
    fs::write(
        project.path().join("docker-compose.yaml"),
        "services:\n  ignored:\n    image: example.invalid/ignored:1\n",
    )?;
    let override_file = project.path().join("override.yaml");
    fs::write(&override_file, "services:\n  web:\n    command: [\"run\"]\n")?;
    let override_argument = format!("--input-file={}", path_text(&override_file)?);
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type=compose",
            "--output-type=quadlet",
            "--input-directory",
            path_text(project.path())?,
            override_argument.as_str(),
            "--podman-maximum-version",
            "6.0",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;

    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["inputs"].as_array().map(Vec::len), Some(2));
    assert_eq!(report["inputs"][0]["alias"], "<input-1>");
    assert_eq!(report["inputs"][1]["alias"], "<input-2>");
    assert_eq!(report["discovery"][0]["selected"], "<input-1>");
    assert!(!String::from_utf8_lossy(&result.stdout).contains(path_text(project.path())?));
    assert_eq!(report["resolved_versions"]["minimum"], "5.4.0");
    assert_eq!(report["resolved_versions"]["maximum"], "6.0.2");
    assert!(output.path().join("web.container").exists());
    Ok(())
}

#[test]
fn generic_discovery_reports_ignored_candidates_in_verbose_mode() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("discovery-priority");
    let output = TemporaryOutput::new("discovery-priority-output");
    fs::create_dir_all(project.path())?;
    fs::create_dir(project.path().join("compose.yaml"))?;
    fs::write(
        project.path().join("compose.yml"),
        "name: discovered\nservices:\n  app:\n    image: example.invalid/app:1\n",
    )?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--verbose",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("input:"));
    assert!(stdout.contains("ignored candidate:"));
    let report = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(report.status.success(), "{}", String::from_utf8_lossy(&report.stderr));
    let report: serde_json::Value = serde_json::from_slice(&report.stdout)?;
    assert_eq!(report["discovery"][0]["ignored"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[test]
fn capabilities_verbose_and_human_conversion_output_include_concise_summaries() -> Result<(), Box<dyn Error>> {
    let capabilities = boxferry_command().args(["capabilities", "--verbose"]).output()?;
    assert!(capabilities.status.success());
    let capabilities_stdout = String::from_utf8(capabilities.stdout)?;
    assert!(capabilities_stdout.contains("fidelity:"));
    assert!(capabilities_stdout.contains("fidelity boundary:"));
    let capabilities_json = boxferry_command()
        .args(["capabilities", "--console-format", "json"])
        .output()?;
    assert!(capabilities_json.status.success());
    assert_eq!(std::str::from_utf8(&capabilities_json.stdout)?.lines().count(), 1);
    let capabilities_json: serde_json::Value = serde_json::from_slice(&capabilities_json.stdout)?;
    let boundaries = &capabilities_json["routes"][0]["fidelity_boundaries"];
    assert_eq!(boundaries["exact"], "supported-compose-quadlet-intersection");
    assert_eq!(boundaries["approximate"], serde_json::json!(["pod-grouping"]));
    assert_eq!(
        boundaries["policy_controlled"],
        serde_json::json!(["unsupported-fields"])
    );

    let project = TemporaryOutput::new("human-conversion-summary");
    let output = TemporaryOutput::new("human-conversion-summary-output");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        "name: human-summary\nservices:\n  app:\n    image: example.invalid/app:1\n",
    )?;
    let conversion = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert!(
        conversion.status.success(),
        "{}",
        String::from_utf8_lossy(&conversion.stderr)
    );
    let stdout = String::from_utf8(conversion.stdout)?;
    assert!(stdout.contains("application: human-summary"));
    assert!(stdout.contains("stage: conversion complete"));
    assert!(stdout.contains("result: wrote 1 file(s)"));
    Ok(())
}

#[test]
fn generic_rejects_duplicate_and_multiple_stdin_before_writes() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let output = TemporaryOutput::new("duplicate-input");
    let duplicate = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--input-file",
            path_text(&fixture)?,
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate resolved input"));
    assert!(!output.path().exists());
    let stdin = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            "-",
            "--input-file",
            "-",
        ])
        .output()?;
    assert_eq!(stdin.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&stdin.stderr).contains("stdin may be supplied only once"));
    Ok(())
}

#[test]
fn generic_rejects_empty_discovery_unsafe_inputs_and_major_only_versions() -> Result<(), Box<dyn Error>> {
    let empty = TemporaryOutput::new("empty-discovery");
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    fs::create_dir_all(empty.path())?;
    let no_candidate = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(empty.path())?,
        ])
        .output()?;
    assert_eq!(no_candidate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&no_candidate.stderr).contains("no conventional Compose file"));
    let non_regular = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(empty.path())?,
        ])
        .output()?;
    assert_eq!(non_regular.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&non_regular.stderr).contains("regular non-symlink file"));
    let major_only = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--podman-minimum-version",
            "5",
        ])
        .output()?;
    assert_eq!(major_only.status.code(), Some(2));
    Ok(())
}

#[test]
fn generic_environment_files_and_explicit_inputs_have_documented_precedence_without_value_leaks()
-> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("environment-precedence");
    let output = TemporaryOutput::new("environment-precedence-output");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    let first = project.path().join("first.env");
    let second = project.path().join("second.env");
    fs::write(
        &compose,
        "name: precedence\nservices:\n  app:\n    image: example.invalid/app:${TAG}\n",
    )?;
    fs::write(&first, "TAG=first-secret\n")?;
    fs::write(&second, "TAG=second-secret\n")?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--interpolate",
            "--env-file",
            path_text(&first)?,
            "--env-file",
            path_text(&second)?,
            "--env=TAG=final-secret",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(fs::read_to_string(output.path().join("app.container"))?.contains("app:final-secret"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("final-secret"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("final-secret"));
    Ok(())
}

#[test]
fn generic_interpolation_values_are_redacted_in_failed_json_report_and_bundle_outputs() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("interpolation-failure-canaries");
    let output = TemporaryOutput::new("interpolation-failure-output");
    let report_directory = TemporaryOutput::new("interpolation-failure-report");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(output.path())?;
    fs::create_dir_all(report_directory.path())?;
    let compose = project.path().join("compose.yaml");
    let environment = project.path().join("values.env");
    let report = report_directory.path().join("result.json");
    let bundle = report_directory.path().join("result.zip");
    fs::write(
        &compose,
        "name: canaries\nservices:\n  app:\n    image: example.invalid/${FILE_VALUE}-${LITERAL_VALUE}-${PROCESS_VALUE}\n",
    )?;
    fs::write(&environment, "FILE_VALUE=file-value-canary\n")?;
    let result = boxferry_command()
        .env("BOXFERRY_TEST_PROCESS_VALUE", "process-value-canary")
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--interpolate",
            "--env-file",
            path_text(&environment)?,
            "--env=LITERAL_VALUE=literal-value-canary",
            "--env",
            "BOXFERRY_TEST_PROCESS_VALUE",
            "--output-directory",
            path_text(output.path())?,
            "--report-file",
            path_text(&report)?,
            "--generate-error-report",
            path_text(&bundle)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let json = String::from_utf8(result.stdout)?;
    let file = fs::read_to_string(&report)?;
    let mut archive = ZipArchive::new(Cursor::new(fs::read(&bundle)?))?;
    let mut bundled = String::new();
    for index in 0..archive.len() {
        std::io::Read::read_to_string(&mut archive.by_index(index)?, &mut bundled)?;
    }
    for canary in ["file-value-canary", "literal-value-canary", "process-value-canary"] {
        assert!(!json.contains(canary), "console leaked {canary}");
        assert!(!file.contains(canary), "report file leaked {canary}");
        assert!(!bundled.contains(canary), "support bundle leaked {canary}");
    }
    let value: serde_json::Value = serde_json::from_str(&json)?;
    assert_eq!(value["failed_stage"], "output-write");
    assert!(
        value["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFC0021"))
    );
    Ok(())
}

#[test]
fn output_write_failure_preserves_the_completed_conversion_report_everywhere() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("output-write-report-project");
    let output = TemporaryOutput::new("output-write-report-output");
    let report_directory = TemporaryOutput::new("output-write-report-files");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(output.path())?;
    fs::create_dir_all(report_directory.path())?;
    let compose = project.path().join("compose.yaml");
    let report = report_directory.path().join("result.json");
    let bundle = report_directory.path().join("result.zip");
    fs::write(
        &compose,
        "name: complete-report\nservices:\n  app:\n    image: example.invalid/app:1\n",
    )?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--output-directory",
            path_text(output.path())?,
            "--report-file",
            path_text(&report)?,
            "--generate-error-report",
            path_text(&bundle)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let console: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    let file: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report)?)?;
    let mut archive = ZipArchive::new(Cursor::new(fs::read(&bundle)?))?;
    let mut bundled = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("report.json")?, &mut bundled)?;
    let bundle: serde_json::Value = serde_json::from_str(&bundled)?;
    for value in [&console, &file, &bundle] {
        assert_eq!(value["status"], "failure");
        assert_eq!(value["exit_category"], "output-write");
        assert_eq!(value["failed_stage"], "output-write");
        assert_eq!(value["application"], "complete-report");
        assert_eq!(value["inputs"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["discovery"].as_array().map(Vec::len), Some(1));
        assert!(value["resolved_versions"]["minimum"].is_string());
        assert!(value["fidelity"]["exact"].is_u64());
        assert!(
            value["output_artifacts"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            value["diagnostics"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["code"] == "BFC0021"))
        );
    }
    Ok(())
}

#[test]
fn generic_report_and_default_pod_name_use_the_resolved_compose_name() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("fallback-name-must-not-win");
    let output = TemporaryOutput::new("resolved-compose-name-output");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        "name: imported-name\nservices:\n  app:\n    image: example.invalid/app:1\n",
    )?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--quadlet-grouping",
            "pod",
            "--loss-policy",
            "approximate",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["application"], "imported-name");
    assert!(output.path().join("imported-name.pod").exists());
    Ok(())
}

#[test]
fn generic_env_name_authorizes_only_that_process_value() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("process-environment");
    let output = TemporaryOutput::new("process-environment-output");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        "name: process-env\nservices:\n  app:\n    image: example.invalid/app:${BOXFERRY_TEST_EXPLICIT_TAG}\n",
    )?;
    let result = boxferry_command()
        .env("BOXFERRY_TEST_EXPLICIT_TAG", "process-secret")
        .env("TAG", "ambient-secret")
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--interpolate",
            "--env",
            "BOXFERRY_TEST_EXPLICIT_TAG",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(fs::read_to_string(output.path().join("app.container"))?.contains("app:process-secret"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("process-secret"));
    Ok(())
}

#[test]
fn generic_rejects_profile_and_presentation_conflicts() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let profile = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--profile",
            "production",
            "--all-profiles",
        ])
        .output()?;
    assert_eq!(profile.status.code(), Some(2));
    let mode = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--quiet",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(mode.status.code(), Some(2));
    Ok(())
}

#[test]
fn generic_grouping_validation_and_validate_mode_do_not_write() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let output = TemporaryOutput::new("validate-no-write");
    let invalid = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--pod-name",
            "only-in-pod",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("--pod-name requires"));
    let validation = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--podman-minimum-version",
            "5.4",
            "--podman-maximum-version",
            "6.0",
            "--loss-policy",
            "partial",
        ])
        .output()?;
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    assert!(!output.path().exists());
    Ok(())
}

#[test]
fn generic_pod_name_is_applied_only_to_explicit_pod_grouping() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-pod").join("compose.yaml");
    let output = TemporaryOutput::new("explicit-pod-name");
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--quadlet-grouping",
            "pod",
            "--pod-name",
            "custom-pod",
            "--loss-policy",
            "approximate",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        fs::read_to_string(output.path().join("custom-pod.pod"))?,
        "[Pod]\nPodName=custom-pod\nUserNS=keep-id\nAddHost=host.docker.internal:host-gateway\nPublishPort=8080:80/tcp\nPublishPort=9090:90/tcp\nNetwork=frontend.network\n"
    );
    Ok(())
}

#[test]
fn help_version_and_json_stream_contracts_remain_conventional() -> Result<(), Box<dyn Error>> {
    for arguments in [
        ["--help"].as_slice(),
        ["convert", "--help"].as_slice(),
        ["--version"].as_slice(),
    ] {
        let result = boxferry_command().args(arguments).output()?;
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert!(!result.stdout.is_empty());
    }
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let json = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--loss-policy",
            "partial",
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(json.status.success(), "{}", String::from_utf8_lossy(&json.stderr));
    assert!(json.stderr.is_empty());
    let _: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    assert_eq!(std::str::from_utf8(&json.stdout)?.lines().count(), 1);
    Ok(())
}

#[test]
fn generic_clap_json_and_stdin_failure_contracts_are_fail_closed() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let missing_output = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
        ])
        .output()?;
    assert_eq!(missing_output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_output.stderr).contains("--output-directory"));
    let stdin_later = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--input-file",
            "-",
        ])
        .output()?;
    assert_eq!(stdin_later.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&stdin_later.stderr).contains("--project-directory"));
    let existing = TemporaryOutput::new("json-write-failure");
    fs::create_dir_all(existing.path())?;
    let json = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--output-directory",
            path_text(existing.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    assert_eq!(report["status"], "failure");
    assert!(!String::from_utf8_lossy(&json.stdout).contains("success"));
    Ok(())
}

#[test]
fn generic_accepts_finite_short_and_exact_podman_selectors() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    for maximum in ["5.8", "6.0.2"] {
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
                "--podman-maximum-version",
                maximum,
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ])
            .output()?;
        assert!(
            result.status.success(),
            "{maximum}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty());
    }
    Ok(())
}

#[test]
fn generic_json_preprocessing_failure_is_one_redacted_document() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("json-preprocess-failure");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(&compose, "services: [not-valid\n")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "failure");
    assert_eq!(std::str::from_utf8(&result.stdout)?.lines().count(), 1);
    Ok(())
}

#[test]
fn report_file_matches_json_and_refuses_existing_paths() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let output = TemporaryOutput::new("report-file-output");
    let report = TemporaryOutput::new("report-file");
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--loss-policy",
            "partial",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
            "--report-file",
            path_text(report.path())?,
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(fs::read(report.path())?, result.stdout);
    let duplicate = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--report-file",
            path_text(report.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(duplicate.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&duplicate.stdout)?;
    assert_eq!(value["exit_category"], "report-write");
    Ok(())
}

#[test]
fn report_schema_declares_the_emitted_v1_shape() -> Result<(), Box<dyn Error>> {
    let schema: serde_json::Value = serde_json::from_str(&fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/conversion-report-v1.schema.json"),
    )?)?;
    let required = schema["required"].as_array().ok_or("top-level required")?;
    for field in [
        "schema_version",
        "boxferry_version",
        "status",
        "exit_category",
        "failed_stage",
        "source_type",
        "target_type",
        "application",
        "inputs",
        "discovery",
        "choices",
        "invocation",
        "host",
        "fidelity",
        "requested_versions",
        "resolved_versions",
        "diagnostics",
        "events",
        "output_artifacts",
        "review_required",
        "redaction",
        "truncations",
    ] {
        assert!(required.iter().any(|value| value == field), "missing {field}");
    }
    for definition in [
        "version_bounds",
        "input",
        "discovery",
        "choice",
        "invocation",
        "host",
        "fidelity",
        "span",
        "field",
        "diagnostic",
        "artifact",
        "redaction",
        "truncation",
    ] {
        let object = &schema["$defs"][definition];
        assert_eq!(object["additionalProperties"], false, "{definition}");
        assert!(object["required"].is_array(), "{definition}");
    }
    assert_eq!(
        schema["$defs"]["invocation"]["required"]
            .as_array()
            .ok_or("invocation required fields")?
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>(),
        Some(vec!["command_kind", "provided_option_names"])
    );
    Ok(())
}

#[test]
fn report_json_never_contains_seeded_input_or_interpolation_canaries() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("report-canaries-project");
    let output = TemporaryOutput::new("report-canaries-output");
    let report = TemporaryOutput::new("report-canaries");
    let bundle_directory = TemporaryOutput::new("report-canaries-bundle");
    if report.path().exists() {
        fs::remove_file(report.path())?;
    }
    fs::create_dir_all(bundle_directory.path())?;
    let bundle = bundle_directory.path().join("report.zip");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    let environment = project.path().join("canary.env");
    fs::write(
        &compose,
        concat!(
            "services:\n  app:\n    image: example.invalid/app:${EXPLICIT_CANARY}\n    environment:\n",
            "      URL: https://user:URL_CANARY@example.invalid/path\n",
            "      AUTHORIZATION: Authorization: Bearer HEADER_CANARY\n",
            "      PRIVATE_KEY: \"-----BEGIN PRIVATE KEY----- PEM_CANARY -----END PRIVATE KEY-----\"\n",
            "      TOKEN: PROTECTED_CANARY\n",
        ),
    )?;
    fs::write(&environment, "FROM_FILE=ENV_FILE_CANARY\n")?;
    let result = boxferry_command()
        .env("BOXFERRY_REPORT_PROCESS_CANARY", "PROCESS_CANARY")
        .args([
            "convert",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--interpolate",
            "--env-file",
            path_text(&environment)?,
            "--env=EXPLICIT_CANARY=EXPLICIT_CANARY",
            "--env",
            "BOXFERRY_REPORT_PROCESS_CANARY",
            "--loss-policy",
            "partial",
            "--output-directory",
            path_text(output.path())?,
            "--report-file",
            path_text(report.path())?,
            "--generate-error-report",
            path_text(&bundle)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    let json = String::from_utf8(result.stdout)?;
    let file = fs::read_to_string(report.path())?;
    let mut archive = ZipArchive::new(Cursor::new(fs::read(bundle)?))?;
    let mut bundled_contents = String::new();
    for index in 0..archive.len() {
        std::io::Read::read_to_string(&mut archive.by_index(index)?, &mut bundled_contents)?;
    }
    for canary in [
        "URL_CANARY",
        "HEADER_CANARY",
        "PEM_CANARY",
        "PROTECTED_CANARY",
        "ENV_FILE_CANARY",
        "EXPLICIT_CANARY",
        "PROCESS_CANARY",
    ] {
        assert!(!json.contains(canary), "console leaked {canary}");
        assert!(!file.contains(canary), "report file leaked {canary}");
        assert!(!bundled_contents.contains(canary), "support bundle leaked {canary}");
    }
    let value: serde_json::Value = serde_json::from_str(&json)?;
    assert_eq!(value["review_required"], true);
    assert_eq!(value["invocation"]["command_kind"], "convert");
    let option_names = value["invocation"]["provided_option_names"]
        .as_array()
        .ok_or("provided invocation names")?;
    for option in [
        "--output-directory",
        "--console-format",
        "--report-file",
        "--generate-error-report",
    ] {
        assert!(option_names.iter().any(|name| name == option), "missing {option}");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn support_bundle_refuses_a_symlink_parent() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-symlink");
    let target = TemporaryOutput::new("support-bundle-symlink-target");
    fs::create_dir_all(target.path())?;
    symlink(target.path(), directory.path())?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--generate-error-report",
            path_text(&directory.path().join("report.zip"))?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    assert!(!target.path().join("report.zip").exists());
    fs::remove_file(directory.path())?;

    let destination = target.path().join("existing-link.zip");
    let existing = target.path().join("existing.zip");
    fs::write(&existing, "existing archive")?;
    symlink(&existing, &destination)?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--generate-error-report",
            path_text(&destination)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&existing)?, "existing archive");
    Ok(())
}

#[test]
fn report_file_is_attempted_before_the_support_bundle() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-ordering");
    fs::create_dir_all(directory.path())?;
    let report = directory.path().join("report.json");
    let bundle = directory.path().join("report.zip");
    fs::write(&report, "existing")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--report-file",
            path_text(&report)?,
            "--generate-error-report",
            path_text(&bundle)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let mut archive = ZipArchive::new(Cursor::new(fs::read(bundle)?))?;
    let mut entry = archive.by_name("report.json")?;
    let mut bundled = String::new();
    std::io::Read::read_to_string(&mut entry, &mut bundled)?;
    assert!(
        serde_json::from_str::<serde_json::Value>(&bundled)?["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFC0020"))
    );
    Ok(())
}

#[test]
fn report_write_failure_preserves_a_primary_failure_category_and_stage() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("report-primary-failure");
    let report = TemporaryOutput::new("report-primary-existing");
    fs::create_dir_all(project.path())?;
    fs::write(project.path().join("compose.yaml"), "services: [broken\n")?;
    fs::write(report.path(), "already exists")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&project.path().join("compose.yaml"))?,
            "--report-file",
            path_text(report.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(value["failed_stage"], "compose-merge");
    assert_ne!(value["exit_category"], "report-write");
    assert!(
        value["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFC0020"))
    );
    Ok(())
}

#[test]
fn support_bundle_has_only_fixed_stored_entries_and_is_independent_of_presentation() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    for (label, presentation) in [
        ("normal", Vec::new()),
        ("quiet", vec!["--quiet"]),
        ("json", vec!["--console-format", "json"]),
        ("verbose", vec!["--verbose"]),
    ] {
        let directory = TemporaryOutput::new(&format!("support-bundle-{label}"));
        fs::create_dir_all(directory.path())?;
        let archive = directory.path().join("report.zip");
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
                "--loss-policy",
                "partial",
                "--generate-error-report",
                path_text(&archive)?,
            ])
            .args(presentation)
            .output()?;
        assert!(
            result.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut zip = ZipArchive::new(Cursor::new(fs::read(&archive)?))?;
        assert_eq!(zip.len(), 2);
        for (index, name) in ["README.md", "report.json"].iter().enumerate() {
            let mut entry = zip.by_index(index)?;
            assert_eq!(entry.name(), *name);
            assert_eq!(entry.compression(), CompressionMethod::Stored);
            let mut contents = String::new();
            std::io::Read::read_to_string(&mut entry, &mut contents)?;
            if *name == "README.md" {
                assert!(contents.contains("review_required: true"));
                assert!(contents.contains("Inspect both files before uploading"));
            } else {
                let report: serde_json::Value = serde_json::from_str(&contents)?;
                assert_eq!(report["review_required"], true);
                assert_eq!(report["invocation"]["command_kind"], "validate");
                let option_names = report["invocation"]["provided_option_names"]
                    .as_array()
                    .ok_or("provided option names")?;
                let has = |option| option_names.iter().any(|name| name == option);
                assert!(has("--generate-error-report"));
                assert_eq!(has("--verbose"), label == "verbose");
                assert_eq!(has("--quiet"), label == "quiet");
                assert_eq!(has("--console-format"), label == "json");
                assert!(has("--loss-policy"));
                assert!(!has("--podman-minimum-version"));
                assert!(!has("--podman-maximum-version"));
                assert!(report["host"]["os_family"].is_string());
                assert!(report["host"]["architecture"].is_string());
                assert!(report["fidelity"]["exact"].is_u64());
            }
        }
        assert!(
            fs::read_dir(directory.path())?
                .all(|entry| entry.is_ok_and(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")))
        );
    }
    Ok(())
}

#[test]
fn support_bundle_is_created_for_blocked_and_failed_conversions() -> Result<(), Box<dyn Error>> {
    let blocked_fixture = fixture_directory("compose-to-quadlet-core").join("compose.yaml");
    let failed_directory = TemporaryOutput::new("support-bundle-failure-source");
    fs::create_dir_all(failed_directory.path())?;
    let failed_fixture = failed_directory.path().join("broken.yaml");
    fs::write(&failed_fixture, "services: [broken\n")?;
    for (label, input, expected_status, expected_exit) in [
        ("blocked", blocked_fixture, "blocked", 2),
        ("failed", failed_fixture, "failure", 1),
    ] {
        let directory = TemporaryOutput::new(&format!("support-bundle-{label}-result"));
        fs::create_dir_all(directory.path())?;
        let archive = directory.path().join("report.zip");
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&input)?,
                "--generate-error-report",
                path_text(&archive)?,
                "--console-format",
                "json",
            ])
            .output()?;
        assert_eq!(result.status.code(), Some(expected_exit));
        let mut zip = ZipArchive::new(Cursor::new(fs::read(&archive)?))?;
        let mut entry = zip.by_name("report.json")?;
        let mut contents = String::new();
        std::io::Read::read_to_string(&mut entry, &mut contents)?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&contents)?["status"],
            expected_status
        );
    }
    Ok(())
}

#[test]
fn support_bundle_refuses_existing_paths_without_leaving_a_temporary_file() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-existing");
    fs::create_dir_all(directory.path())?;
    let archive = directory.path().join("report.zip");
    fs::write(&archive, "existing archive")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--generate-error-report",
            path_text(&archive)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&archive)?, "existing archive");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["exit_category"], "report-write");
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFC0022"))
    );
    assert!(
        fs::read_dir(directory.path())?
            .all(|entry| entry.is_ok_and(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")))
    );
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
        let path = std::env::temp_dir().join(format!("boxferry-cli-{}-{label}-{id}", std::process::id()));
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else if path.is_file() {
            let _ = fs::remove_file(&path);
        }
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if self.path.is_dir() {
            let _ = fs::remove_dir_all(&self.path);
        } else if self.path.is_file() {
            let _ = fs::remove_file(&self.path);
        }
    }
}
