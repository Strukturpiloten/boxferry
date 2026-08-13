//! Black-box contracts for the installable command.

#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet"))]

use std::fmt::Write as _;
use std::{
    error::Error,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
use zip::{CompressionMethod, ZipArchive};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn compose_import_failure_preserves_every_diagnostic_in_human_and_json_output() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("compose-import-diagnostics");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "name: diagnostic-example\nservices:\n",
            "  first:\n    image: example.invalid/first:1\n    volumes: ['${FIRST}:/data']\n",
            "  second:\n    image: example.invalid/second:1\n    volumes: ['${SECOND}:/data']\n",
            "  third:\n    image: example.invalid/third:1\n    shm_size: invalid\n",
        ),
    )?;
    let common = [
        "validate",
        "--input-type",
        "compose",
        "--input-file",
        path_text(&compose)?,
        "--output-type",
        "quadlet",
    ];

    let human = boxferry_command().args(common).output()?;
    assert_eq!(human.status.code(), Some(1));
    let stdout = String::from_utf8(human.stdout)?;
    let stderr = String::from_utf8(human.stderr)?;
    assert!(stdout.contains("stage: conversion failed"), "{stdout}");
    assert_eq!(
        stderr.matches("BFC0105 compose-unresolved-variable [warning]").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.contains("(2 findings)"), "{stderr}");
    assert!(stderr.contains("Use --interpolate"), "{stderr}");
    assert!(stderr.contains("variable: FIRST"), "{stderr}");
    assert!(stderr.contains("variable: SECOND"), "{stderr}");
    assert_eq!(
        stderr.matches("BFC0005 compose-value-invalid [error]").count(),
        1,
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("BFC0198 compose-native-warning [warning]").count(),
        1,
        "{stderr}"
    );
    assert!(!stderr.contains("source rule:"), "{stderr}");
    for subject in [
        "services.first.volumes[0]",
        "services.second.volumes[0]",
        "services.third.shm_size",
    ] {
        assert!(stderr.contains(subject), "missing {subject} in {stderr}");
    }
    assert!(!stderr.contains("source import failed with"), "{stderr}");

    let json = boxferry_command()
        .args(common)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    assert_eq!(report["failed_stage"], "conversion");
    let diagnostics = report["diagnostics"].as_array().ok_or("missing diagnostics")?;
    assert_eq!(diagnostics.len(), 5);
    assert_eq!(diagnostics[0]["code"], "BFC0005");
    assert_eq!(diagnostics[1]["code"], "BFC0105");
    assert_eq!(diagnostics[2]["code"], "BFC0105");
    assert_eq!(diagnostics[3]["code"], "BFC0198");
    assert_eq!(diagnostics[4]["code"], "BFC0198");
    assert_eq!(
        diagnostics[3]["source_code"],
        "compose.shm-size.provider-dependent-string"
    );
    assert_eq!(diagnostics[3]["native_finding"]["source_format"], "compose");
    assert_eq!(diagnostics[3]["native_finding"]["producer"], "compose-lens");
    assert_eq!(diagnostics[3]["native_finding"]["stage"], "load");
    assert_eq!(diagnostics[3]["native_finding"]["labels"][0]["kind"], "primary");
    Ok(())
}

#[test]
fn unresolved_compose_environment_variables_are_paired_with_target_findings() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("unresolved-compose-environment");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "name: unresolved-environment\nservices:\n  database:\n",
            "    image: example.invalid/database:1\n",
            "    environment:\n",
            "      POSTGRES_PASSWORD: ${DB_PASSWORD}\n",
            "      POSTGRES_USER: ${DB_USERNAME}\n",
            "      POSTGRES_DB: ${DB_DATABASE_NAME}\n",
        ),
    )?;
    let common = [
        "validate",
        "--input-type",
        "compose",
        "--input-file",
        path_text(&compose)?,
        "--output-type",
        "quadlet",
    ];

    let exact = boxferry_command().args(common).output()?;
    assert_eq!(exact.status.code(), Some(2));
    let stderr = String::from_utf8(exact.stderr)?;
    assert!(
        stderr.contains("BFC0105 compose-unresolved-variable [warning] (3 findings)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("BFQ0003 quadlet-output-unsupported [warning] (3 findings)"),
        "{stderr}"
    );
    for variable in ["DB_PASSWORD", "DB_USERNAME", "DB_DATABASE_NAME"] {
        assert!(stderr.contains(&format!("variable: {variable}")), "{stderr}");
    }
    assert!(stderr.contains("Use --interpolate"), "{stderr}");

    let partial = boxferry_command()
        .args(common)
        .args(["--loss-policy", "partial"])
        .output()?;
    assert!(partial.status.success(), "{}", String::from_utf8_lossy(&partial.stderr));
    let partial_stderr = String::from_utf8(partial.stderr)?;
    assert!(partial_stderr.contains("BFC0105 compose-unresolved-variable [warning]"));
    assert!(partial_stderr.contains("BFQ0003 quadlet-output-unsupported [warning]"));
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
    assert!(report["fix_first"].is_null());
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
fn generic_convert_handles_a_large_repository_owned_offline_scenario() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("large-offline-project");
    let output = TemporaryOutput::new("large-offline-output");
    fs::create_dir_all(project.path())?;
    let mut compose = String::from("name: large-offline\nservices:\n");
    for service in 0..48 {
        writeln!(
            compose,
            "  service-{service}:\n    image: example.invalid/service-{service}:1\n    environment:\n      ROLE: service-{service}"
        )?;
    }
    fs::write(project.path().join("compose.yaml"), compose)?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type=compose",
            "--output-type=quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(result.stderr.is_empty());
    let stdout = String::from_utf8(result.stdout)?;
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "route: compose -> quadlet",
            format!("input: {}", path_text(&project.path().join("compose.yaml"))?).as_str(),
            "application: large-offline",
            "stage: conversion complete",
            "",
            "boxferry: command succeeded; wrote 48 file(s) to output directory",
        ]
    );

    let mut generated = fs::read_dir(output.path())?
        .map(|entry| {
            entry
                .map_err(|error| error.to_string())?
                .file_name()
                .into_string()
                .map_err(|name| format!("non-UTF-8 artifact name: {name:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected = (0..48)
        .map(|service| format!("service-{service}.container"))
        .collect::<Vec<_>>();
    generated.sort();
    expected.sort();
    assert_eq!(generated, expected);

    for service in 0..48 {
        let document = fs::read_to_string(output.path().join(format!("service-{service}.container")))?;
        assert!(
            document.contains(&format!("Image=example.invalid/service-{service}:1")),
            "service-{service} image intent is missing"
        );
        assert!(
            document.contains(&format!("Environment=ROLE=service-{service}")),
            "service-{service} environment intent is missing"
        );
    }
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
    assert_eq!(capabilities_json["routes"].as_array().map(Vec::len), Some(2));
    let boundaries = &capabilities_json["routes"][0]["fidelity_boundaries"];
    assert_eq!(boundaries["exact"], "supported-compose-quadlet-intersection");
    assert_eq!(boundaries["approximate"], serde_json::json!(["pod-grouping"]));
    assert_eq!(
        boundaries["policy_controlled"],
        serde_json::json!(["unsupported-fields"])
    );
    assert_eq!(capabilities_json["routes"][1]["input_type"], "quadlet");
    assert_eq!(capabilities_json["routes"][1]["output_type"], "compose");
    assert_eq!(
        capabilities_json["routes"][1]["target_selector"],
        "compose-specification-rolling"
    );
    assert_eq!(capabilities_json["routes"][1]["requested_version"], "rolling");
    assert_eq!(capabilities_json["routes"][1]["resolved_version"], "rolling");
    assert!(
        capabilities_json["routes"][1]
            .get("accepted_compose_providers")
            .is_none()
    );
    assert!(
        capabilities_json["routes"][1]
            .get("exact_provider_version_required")
            .is_none()
    );
    assert!(capabilities_json["routes"][1].get("podman_minimum").is_none());
    assert_eq!(
        capabilities_json["routes"][1]["fidelity_boundaries"]["approximate"],
        serde_json::json!(["environment-file-reconstruction"])
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
    assert_eq!(
        stdout.lines().next_back(),
        Some("boxferry: command succeeded; wrote 1 file(s) to output directory")
    );
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

fn write_interpolation_diagnostic_fixture(project: &Path) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(project)?;
    let compose = project.join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "name: interpolation-diagnostics\nservices:\n",
            "  application:\n",
            "    image: example.invalid/application:${IMMICH_VERSION-latest}\n",
            "    environment:\n",
            "      DB_PASSWORD: ${DB_PASSWORD}\n",
            "      DB_USERNAME: ${DB_USERNAME}\n",
            "    volumes:\n",
            "      - ${UPLOAD_LOCATION}:/data\n",
            "  database:\n",
            "    image: example.invalid/database:1\n",
            "    environment:\n",
            "      DB_DATABASE_NAME: ${DB_DATABASE_NAME}\n",
            "    volumes:\n",
            "      - ${DB_DATA_LOCATION}:/database\n",
        ),
    )?;
    Ok(compose)
}

const INTERPOLATION_VARIABLES: [&str; 5] = [
    "UPLOAD_LOCATION",
    "DB_DATA_LOCATION",
    "DB_PASSWORD",
    "DB_USERNAME",
    "DB_DATABASE_NAME",
];

#[test]
fn empty_interpolation_environment_preserves_native_warnings_before_import_errors() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("empty-interpolation-environment");
    let compose = write_interpolation_diagnostic_fixture(project.path())?;
    let common = [
        "validate",
        "--input-type",
        "compose",
        "--input-file",
        path_text(&compose)?,
        "--output-type",
        "quadlet",
        "--interpolate",
    ];
    let mut command = boxferry_command();
    command.args(common);
    for variable in INTERPOLATION_VARIABLES {
        command.env_remove(variable);
    }
    let result = command.output()?;

    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr)?;
    for variable in INTERPOLATION_VARIABLES {
        assert!(stderr.contains(&format!("variable: {variable}")), "{stderr}");
    }
    assert!(stderr.contains("(5 findings)"), "{stderr}");
    assert_eq!(
        stderr.matches("A Compose interpolation variable is not set.").count(),
        1,
        "{stderr}"
    );
    assert!(!stderr.contains("source rule:"), "{stderr}");
    assert!(stderr.contains("BFC0001 compose-model-invalid [error]"), "{stderr}");
    assert_eq!(
        stderr
            .matches("help: Provide the missing value with --env-file PATH or --env NAME=VALUE.")
            .count(),
        1,
        "{stderr}"
    );

    let mut json_command = boxferry_command();
    json_command.args(common).args(["--console-format", "json"]);
    for variable in INTERPOLATION_VARIABLES {
        json_command.env_remove(variable);
    }
    let json = json_command.output()?;
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    let diagnostics = report["diagnostics"].as_array().ok_or("missing diagnostics")?;
    for variable in INTERPOLATION_VARIABLES {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["summary"] == format!("interpolation variable `{variable}` is not set"))
        );
    }
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFC0001"));
    assert!(!String::from_utf8(json.stdout)?.contains("IMMICH_VERSION"));
    Ok(())
}

#[test]
fn partial_interpolation_environment_is_complete_in_human_and_json_diagnostics_without_value_leaks()
-> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("partial-interpolation-environment");
    let compose = write_interpolation_diagnostic_fixture(project.path())?;
    let environment = project.path().join("partial.env");
    fs::write(
        &environment,
        "UPLOAD_LOCATION=/provided-upload-value-canary\nDB_DATA_LOCATION=/provided-database-value-canary\n",
    )?;
    let common = [
        "validate",
        "--input-type",
        "compose",
        "--input-file",
        path_text(&compose)?,
        "--output-type",
        "quadlet",
        "--interpolate",
        "--env-file",
        path_text(&environment)?,
    ];

    let mut human_command = boxferry_command();
    human_command.args(common);
    for variable in INTERPOLATION_VARIABLES {
        human_command.env_remove(variable);
    }
    let human = human_command.output()?;
    assert!(human.status.success(), "{}", String::from_utf8_lossy(&human.stderr));
    let human_stdout = String::from_utf8(human.stdout)?;
    let human_stderr = String::from_utf8(human.stderr)?;
    for variable in ["DB_PASSWORD", "DB_USERNAME", "DB_DATABASE_NAME"] {
        assert!(human_stderr.contains(&format!("variable: {variable}")));
    }
    for variable in ["UPLOAD_LOCATION", "DB_DATA_LOCATION"] {
        assert!(!human_stderr.contains(&format!("variable: {variable}")));
    }
    assert!(human_stderr.contains("(3 findings)"));
    assert_eq!(
        human_stderr
            .matches("help: Provide the missing value with --env-file PATH or --env NAME=VALUE.")
            .count(),
        1
    );
    for canary in ["provided-upload-value-canary", "provided-database-value-canary"] {
        assert!(!human_stdout.contains(canary));
        assert!(!human_stderr.contains(canary));
    }

    let mut json_command = boxferry_command();
    json_command.args(common).args(["--console-format", "json"]);
    for variable in INTERPOLATION_VARIABLES {
        json_command.env_remove(variable);
    }
    let json = json_command.output()?;
    assert!(json.status.success(), "{}", String::from_utf8_lossy(&json.stderr));
    assert!(json.stderr.is_empty());
    let encoded = String::from_utf8(json.stdout)?;
    let report: serde_json::Value = serde_json::from_str(&encoded)?;
    let diagnostics = report["diagnostics"].as_array().ok_or("missing diagnostics")?;
    for variable in ["DB_PASSWORD", "DB_USERNAME", "DB_DATABASE_NAME"] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["summary"] == format!("interpolation variable `{variable}` is not set"))
        );
    }
    for canary in ["provided-upload-value-canary", "provided-database-value-canary"] {
        assert!(!encoded.contains(canary));
    }
    Ok(())
}

#[test]
fn repeated_human_rules_hoist_shared_context_and_list_only_finding_evidence() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("grouped-human-diagnostics");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "name: grouped-diagnostics\nservices:\n",
            "  first:\n",
            "    image: example.invalid/first:1\n",
            "    restart: unless-stopped\n",
            "    environment:\n      VALUE: ${FIRST_VALUE}\n",
            "  second:\n",
            "    image: example.invalid/second:1\n",
            "    restart: unless-stopped\n",
            "    environment:\n      VALUE: ${SECOND_VALUE}\n",
        ),
    )?;
    let output = boxferry_command()
        .env_remove("FIRST_VALUE")
        .env_remove("SECOND_VALUE")
        .args([
            "validate",
            "--input-type",
            "compose",
            "--input-file",
            path_text(&compose)?,
            "--output-type",
            "quadlet",
            "--interpolate",
            "--loss-policy",
            "approximate",
        ])
        .output()?;

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("BFC0101 compose-unset-variable [warning] (2 findings)"));
    assert_eq!(
        stderr.matches("A Compose interpolation variable is not set.").count(),
        1
    );
    assert!(stderr.contains("1. variable: FIRST_VALUE"));
    assert!(stderr.contains("2. variable: SECOND_VALUE"));
    assert!(!stderr.contains("interpolation variable `"));
    assert!(!stderr.contains("source rule:"));

    assert!(stderr.contains("BFQ0009 quadlet-restart-policy-approximation [warning] (2 findings)"));
    assert_eq!(
        stderr
            .matches("container restart behavior is approximated by the systemd service manager")
            .count(),
        1
    );
    assert_eq!(stderr.matches("\n  reason:").count(), 1, "{stderr}");
    assert!(stderr.contains("1. subject: services.first.restart_policy"));
    assert!(stderr.contains("2. subject: services.second.restart_policy"));
    Ok(())
}

#[test]
fn generic_interpolation_values_are_redacted_in_failed_json_report_and_bundle_outputs() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("interpolation-failure-canaries");
    let output = TemporaryOutput::new("interpolation-failure-output");
    let report_directory = TemporaryOutput::new("interpolation-failure-report");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(output.path())?;
    fs::write(output.path().join("sentinel"), "keep")?;
    fs::create_dir_all(report_directory.path())?;
    let compose = project.path().join("compose.yaml");
    let environment = project.path().join("values.env");
    let report = report_directory.path().join("result.json");
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
            "--error-report-directory",
            path_text(report_directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let json = String::from_utf8(result.stdout)?;
    let file = fs::read_to_string(&report)?;
    let bundle = generated_error_report(report_directory.path())?;
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
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFO2001"))
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
    fs::write(output.path().join("sentinel"), "keep")?;
    fs::create_dir_all(report_directory.path())?;
    let compose = project.path().join("compose.yaml");
    let report = report_directory.path().join("result.json");
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
            "--error-report-directory",
            path_text(report_directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let console: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    let file: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report)?)?;
    let bundle_path = generated_error_report(report_directory.path())?;
    let mut archive = ZipArchive::new(Cursor::new(fs::read(&bundle_path)?))?;
    let mut bundled = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("report.json")?, &mut bundled)?;
    let bundle: serde_json::Value = serde_json::from_str(&bundled)?;
    for value in [&console, &file, &bundle] {
        assert_eq!(value["status"], "failure");
        assert_eq!(value["exit_category"], "output-write");
        assert_eq!(value["failed_stage"], "output-write");
        assert_eq!(value["primary_diagnostic_code"], "BFO2001");
        assert_eq!(value["fix_first"]["code"], "BFO2001");
        assert_eq!(value["fix_first"]["name"], "output-directory-not-empty");
        assert!(value["fix_first"]["help"].is_string());
        assert!(value["fix_first"]["next_step"].is_string());
        assert!(
            value["failure_summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("output-directory-not-empty"))
        );
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
                .is_some_and(|items| items.iter().any(|item| item["code"] == "BFO2001"))
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
    for presentation in [
        ["--verbose", "--quiet", ""],
        ["--verbose", "--console-format", "json"],
        ["--quiet", "--console-format", "json"],
    ] {
        let mode = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
            ])
            .args(presentation.into_iter().filter(|value| !value.is_empty()))
            .output()?;
        assert_eq!(mode.status.code(), Some(2), "{presentation:?}");
    }
    Ok(())
}

#[test]
fn compose_profile_and_project_directory_options_have_successful_cli_paths() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("compose-profile-project-directory");
    let inputs = project.path().join("inputs");
    fs::create_dir_all(&inputs)?;
    let compose = inputs.join("compose.yaml");
    fs::write(
        &compose,
        concat!(
            "name: option-paths\nservices:\n",
            "  base:\n    image: example.invalid/base:1\n    volumes: ['./data:/data']\n",
            "  optional:\n    image: example.invalid/optional:1\n    profiles: [optional]\n",
        ),
    )?;
    let run = |selection: &[&str]| -> Result<serde_json::Value, Box<dyn Error>> {
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&compose)?,
                "--project-directory",
                path_text(project.path())?,
                "--console-format",
                "json",
            ])
            .args(selection)
            .output()?;
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        Ok(serde_json::from_slice(&result.stdout)?)
    };

    let default = run(&[])?;
    assert!(default["choices"].as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|choice| choice["name"] == "profiles" && choice["value"] == "")
    }));
    let profile = run(&["--profile", "optional"])?;
    assert!(profile["choices"].as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|choice| choice["name"] == "profiles" && choice["value"] == "optional")
    }));
    let all = run(&["--all-profiles"])?;
    assert!(all["choices"].as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|choice| choice["name"] == "profiles" && choice["value"] == "all")
    }));
    Ok(())
}

fn write_loss_policy_fixture(project: &Path, service_fields: &str) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(project)?;
    let compose = project.join("compose.yaml");
    fs::write(
        &compose,
        format!(
            "name: policy-matrix\nservices:\n  application:\n    image: example.invalid/application:1\n{service_fields}"
        ),
    )?;
    Ok(compose)
}

fn write_unresolved_image_fixture(project: &Path) -> Result<PathBuf, Box<dyn Error>> {
    fs::create_dir_all(project)?;
    let compose = project.join("compose.yaml");
    fs::write(
        &compose,
        "name: variables\nservices:\n  application:\n    image: example.invalid/application:${IMAGE_VERSION:-latest}\n",
    )?;
    Ok(compose)
}

fn validate_with_loss_policy(input: &Path, policy: &str) -> Result<Output, Box<dyn Error>> {
    Ok(boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(input)?,
            "--loss-policy",
            policy,
        ])
        .output()?)
}

#[test]
fn every_loss_policy_value_has_positive_and_negative_cli_behavior() -> Result<(), Box<dyn Error>> {
    let exact_project = TemporaryOutput::new("loss-policy-exact");
    let approximate_project = TemporaryOutput::new("loss-policy-approximate");
    let partial_project = TemporaryOutput::new("loss-policy-partial");
    let invalid_project = TemporaryOutput::new("loss-policy-invalid");
    let exact = write_loss_policy_fixture(exact_project.path(), "")?;
    let approximate = write_loss_policy_fixture(approximate_project.path(), "    restart: unless-stopped\n")?;
    let partial = write_loss_policy_fixture(partial_project.path(), "    ports: ['8000-8002:80-82']\n")?;
    let invalid = write_loss_policy_fixture(invalid_project.path(), "    shm_size: invalid\n")?;

    let exact_success = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&exact)?,
            "--quadlet-grouping",
            "separate",
            "--output-layout",
            "files",
            "--loss-policy",
            "exact",
        ])
        .output()?;
    assert!(
        exact_success.status.success(),
        "{}",
        String::from_utf8_lossy(&exact_success.stderr)
    );

    let exact_blocks_approximation = validate_with_loss_policy(&approximate, "exact")?;
    assert_eq!(exact_blocks_approximation.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&exact_blocks_approximation.stderr)
            .contains("BFQ0009 quadlet-restart-policy-approximation [warning]")
    );
    assert_eq!(
        String::from_utf8(exact_blocks_approximation.stdout)?
            .lines()
            .next_back(),
        Some(
            "boxferry: command blocked by the selected loss policy: BFQ0009 quadlet-restart-policy-approximation — container restart behavior is approximated by the systemd service manager"
        )
    );

    let approximate_success = validate_with_loss_policy(&approximate, "approximate")?;
    assert!(
        approximate_success.status.success(),
        "{}",
        String::from_utf8_lossy(&approximate_success.stderr)
    );
    assert!(
        String::from_utf8_lossy(&approximate_success.stderr)
            .contains("BFQ0009 quadlet-restart-policy-approximation [warning]")
    );

    let approximate_blocks_partial = validate_with_loss_policy(&partial, "approximate")?;
    assert_eq!(approximate_blocks_partial.status.code(), Some(2));
    let approximate_blocks_partial_stderr = String::from_utf8_lossy(&approximate_blocks_partial.stderr);
    assert!(approximate_blocks_partial_stderr.contains("BFC0004 compose-intent-unsupported [warning]"));
    assert!(!approximate_blocks_partial_stderr.contains("if the Compose input contains ${...}"));

    let partial_success = validate_with_loss_policy(&partial, "partial")?;
    assert!(
        partial_success.status.success(),
        "{}",
        String::from_utf8_lossy(&partial_success.stderr)
    );
    assert!(String::from_utf8_lossy(&partial_success.stderr).contains("BFC0004 compose-intent-unsupported [warning]"));

    let partial_rejects_invalid = validate_with_loss_policy(&invalid, "partial")?;
    assert_eq!(partial_rejects_invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&partial_rejects_invalid.stderr).contains("BFC0005 compose-value-invalid [error]"));
    Ok(())
}

#[test]
fn loss_policy_never_authorizes_unresolved_image_variables_and_help_resolves_them() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("loss-policy-unresolved-image");
    let compose = write_unresolved_image_fixture(project.path())?;

    for policy in ["exact", "approximate", "partial"] {
        let result = validate_with_loss_policy(&compose, policy)?;
        assert_eq!(result.status.code(), Some(1), "{policy}");
        let stdout = String::from_utf8(result.stdout)?;
        let stderr = String::from_utf8(result.stderr)?;
        assert!(
            stderr.contains("BFQ0014 quadlet-unresolved-source-variable [error]"),
            "{stderr}"
        );
        assert!(
            stderr.contains("BFC0105 compose-unresolved-variable [warning]"),
            "{stderr}"
        );
        assert!(stderr.contains("subject: services.application.image"), "{stderr}");
        assert!(stderr.contains("variable: IMAGE_VERSION"), "{stderr}");
        assert!(stderr.contains("use --interpolate"), "{stderr}");
        assert!(
            stderr.contains(&format!(
                "invalid findings always block output and cannot be authorized by --loss-policy {policy}"
            )),
            "{stderr}"
        );
        assert!(stdout.contains("stage: conversion failed"), "{stdout}");
        assert!(
            stdout.lines().next_back().is_some_and(|line| {
                line.starts_with(
                    "boxferry: command failed during conversion: BFQ0014 quadlet-unresolved-source-variable",
                )
            }),
            "{stdout}"
        );
        assert!(!stdout.contains("blocked by the selected loss policy"), "{stdout}");
        let fix_first = stderr.find("fix first:").ok_or("fix-first section")?;
        let last_diagnostic = stderr
            .rfind("explain: boxferry explain BFQ0014")
            .ok_or("last diagnostic")?;
        assert!(fix_first > last_diagnostic, "{stderr}");
        assert!(
            stderr[fix_first..].contains("BFQ0014 quadlet-unresolved-source-variable"),
            "{stderr}"
        );
        assert!(
            stderr[fix_first..].contains("remaining findings may disappear or change"),
            "{stderr}"
        );
    }

    let resolved = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--interpolate",
            "--loss-policy",
            "partial",
        ])
        .output()?;
    assert!(
        resolved.status.success(),
        "{}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    assert!(!String::from_utf8_lossy(&resolved.stderr).contains("BFQ0014"));
    assert!(!String::from_utf8_lossy(&resolved.stderr).contains("BFC0105"));
    Ok(())
}

#[test]
fn json_reports_structured_fix_first_guidance_and_paired_source_target_findings() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("json-unresolved-image-guidance");
    let compose = write_unresolved_image_fixture(project.path())?;
    let json = boxferry_command()
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
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    assert_eq!(report["primary_diagnostic_code"], "BFQ0014");
    assert_eq!(report["fix_first"]["code"], "BFQ0014");
    assert_eq!(report["fix_first"]["name"], "quadlet-unresolved-source-variable");
    assert!(
        report["fix_first"]["help"]
            .as_str()
            .is_some_and(|help| help.contains("--interpolate"))
    );
    assert!(
        report["fix_first"]["next_step"]
            .as_str()
            .is_some_and(|step| step.contains("remaining findings may disappear or change"))
    );
    let diagnostics = report["diagnostics"].as_array().ok_or("diagnostics")?;
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "BFC0105"
            && diagnostic["fields"].as_array().is_some_and(|fields| {
                fields
                    .iter()
                    .any(|field| field["name"] == "variable" && field["value"] == "IMAGE_VERSION")
            })
    }));
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFQ0014"));
    Ok(())
}

#[test]
fn output_directory_accepts_absent_and_empty_but_rejects_nonempty_and_dotfile_content() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("approximate-output-collision-project");
    let existing_output = TemporaryOutput::new("approximate-output-collision-existing");
    let dotfile_output = TemporaryOutput::new("approximate-output-collision-dotfile");
    let child_directory_output = TemporaryOutput::new("approximate-output-collision-child-directory");
    let empty_output = TemporaryOutput::new("approximate-output-existing-empty");
    let fresh_output = TemporaryOutput::new("approximate-output-collision-fresh");
    let file_output = TemporaryOutput::new("approximate-output-regular-file");
    let compose = write_loss_policy_fixture(project.path(), "    restart: unless-stopped\n")?;
    fs::create_dir_all(existing_output.path())?;
    fs::write(existing_output.path().join("sentinel"), "keep")?;
    fs::create_dir_all(dotfile_output.path())?;
    fs::write(dotfile_output.path().join(".keep"), "keep")?;
    fs::create_dir_all(child_directory_output.path().join("nested"))?;
    fs::create_dir_all(empty_output.path())?;
    fs::write(file_output.path(), "keep")?;
    let run = |output: &Path| -> Result<Output, Box<dyn Error>> {
        Ok(boxferry_command()
            .args([
                "convert",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&compose)?,
                "--loss-policy",
                "approximate",
                "--output-directory",
                path_text(output)?,
            ])
            .output()?)
    };

    let collision: Output = run(existing_output.path())?;
    assert_eq!(collision.status.code(), Some(1));
    let collision_stdout = String::from_utf8(collision.stdout)?;
    let collision_stderr = String::from_utf8(collision.stderr)?;
    assert!(collision_stdout.contains("stage: output write failed"));
    assert_eq!(
        collision_stdout.lines().next_back(),
        Some(
            "boxferry: command failed during output write: BFO2001 output-directory-not-empty — The selected output directory is not empty."
        )
    );
    assert!(collision_stderr.contains("BFQ0009 quadlet-restart-policy-approximation [warning]"));
    assert!(collision_stderr.contains("BFO2001 output-directory-not-empty [error]"));
    assert!(collision_stderr.contains("help: Empty the selected --output-directory or choose a new path."));
    assert_eq!(fs::read_to_string(existing_output.path().join("sentinel"))?, "keep");

    let dotfile: Output = run(dotfile_output.path())?;
    assert_eq!(dotfile.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&dotfile.stderr).contains("output directory is not empty"));
    assert_eq!(fs::read_to_string(dotfile_output.path().join(".keep"))?, "keep");

    let child_directory: Output = run(child_directory_output.path())?;
    assert_eq!(child_directory.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&child_directory.stderr).contains("output directory is not empty"));
    assert!(child_directory_output.path().join("nested").is_dir());

    let regular_file: Output = run(file_output.path())?;
    assert_eq!(regular_file.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&regular_file.stderr).contains("not a non-symlink directory"));
    assert_eq!(fs::read_to_string(file_output.path())?, "keep");

    let existing_empty: Output = run(empty_output.path())?;
    assert!(
        existing_empty.status.success(),
        "{}",
        String::from_utf8_lossy(&existing_empty.stderr)
    );
    assert!(empty_output.path().join("application.container").exists());

    let success: Output = run(fresh_output.path())?;
    assert!(success.status.success(), "{}", String::from_utf8_lossy(&success.stderr));
    assert!(
        String::from_utf8_lossy(&success.stderr).contains("BFQ0009 quadlet-restart-policy-approximation [warning]")
    );
    assert!(!String::from_utf8_lossy(&success.stderr).contains("BFO2001"));
    assert!(fresh_output.path().join("application.container").exists());
    assert_eq!(
        String::from_utf8(success.stdout)?.lines().next_back(),
        Some("boxferry: command succeeded; wrote 1 file(s) to output directory")
    );
    Ok(())
}

#[test]
fn human_sections_are_ordered_and_end_with_success_or_failure() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("human-section-order-project");
    let output = TemporaryOutput::new("human-section-order-output");
    let success_transcript = TemporaryOutput::new("human-section-order-success");
    let failure_transcript = TemporaryOutput::new("human-section-order-failure");
    let compose = write_loss_policy_fixture(
        project.path(),
        concat!(
            "    restart: unless-stopped\n",
            "    environment:\n",
            "      MISSING: ${BOXFERRY_TRANSCRIPT_MISSING}\n",
        ),
    )?;
    let command_arguments = [
        "convert",
        "--input-type",
        "compose",
        "--output-type",
        "quadlet",
        "--input-file",
        path_text(&compose)?,
        "--interpolate",
        "--loss-policy",
        "approximate",
        "--output-directory",
        path_text(output.path())?,
    ];
    let run = |transcript: &Path| -> Result<std::process::ExitStatus, Box<dyn Error>> {
        let file = fs::File::create(transcript)?;
        Ok(boxferry_command()
            .env_remove("BOXFERRY_TRANSCRIPT_MISSING")
            .args(command_arguments)
            .stdout(std::process::Stdio::from(file.try_clone()?))
            .stderr(std::process::Stdio::from(file))
            .status()?)
    };

    assert!(run(success_transcript.path())?.success());
    let success = fs::read_to_string(success_transcript.path())?;
    let stage = success.find("stage: conversion complete").ok_or("success stage")?;
    let interpolation = success
        .find("BFC0101 compose-unset-variable [warning]")
        .ok_or("interpolation warning")?;
    let interpolation_help = success
        .find("help: Provide the missing value with --env-file PATH or --env NAME=VALUE.")
        .ok_or("interpolation help")?;
    let approximation = success
        .find("BFQ0009 quadlet-restart-policy-approximation [warning]")
        .ok_or("approximation warning")?;
    let final_line = "boxferry: command succeeded; wrote 1 file(s) to output directory";
    let final_status = success.find(final_line).ok_or("success status")?;
    assert!(
        stage < interpolation
            && interpolation < interpolation_help
            && interpolation_help < approximation
            && approximation < final_status
    );
    assert!(
        success.contains(
            "stage: conversion complete\n\npolicy: --loss-policy approximate authorized output; non-exact findings remain visible\n\nBFC0101"
        ),
        "{success}"
    );
    assert!(success.contains("boxferry explain BFC0101\n\nBFQ0009"), "{success}");
    assert_eq!(success.lines().next_back(), Some(final_line));

    assert!(!run(failure_transcript.path())?.success());
    let failure = fs::read_to_string(failure_transcript.path())?;
    let output_error = failure
        .find("BFO2001 output-directory-not-empty [error]")
        .ok_or("output error")?;
    let output_help = failure
        .find("help: Empty the selected --output-directory or choose a new path.")
        .ok_or("output help")?;
    let failure_line = "boxferry: command failed during output write: BFO2001 output-directory-not-empty — The selected output directory is not empty.";
    let final_failure = failure.find(failure_line).ok_or("failure status")?;
    assert!(output_error < output_help && output_help < final_failure);
    assert_eq!(failure.lines().next_back(), Some(failure_line));
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
        ["-h"].as_slice(),
        ["convert", "--help"].as_slice(),
        ["convert", "-h"].as_slice(),
        ["--version"].as_slice(),
        ["help"].as_slice(),
        ["help", "convert"].as_slice(),
        ["version"].as_slice(),
    ] {
        let result = boxferry_command().args(arguments).output()?;
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert!(!result.stdout.is_empty());
    }
    let help = boxferry_command().args(["--help"]).output()?;
    assert!(!String::from_utf8(help.stdout)?.contains("compose-to-quadlet"));
    let unknown_help = boxferry_command().args(["help", "unknown"]).output()?;
    assert_eq!(unknown_help.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown_help.stderr).contains("unknown command"));
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
fn rules_and_explain_expose_one_sorted_machine_readable_catalogue() -> Result<(), Box<dyn Error>> {
    let human = boxferry_command().args(["rules"]).output()?;
    assert!(human.status.success());
    assert!(human.stderr.is_empty());
    let human = String::from_utf8(human.stdout)?;
    let lines = human.lines().collect::<Vec<_>>();
    assert!(lines.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(human.contains("BFC0101 compose-unset-variable [warning]"));
    assert!(human.contains("BFQ0009 quadlet-restart-policy-approximation [warning]"));
    assert!(human.contains("BFO2001 output-directory-not-empty [error]"));

    for rule in ["BFQ0009", "quadlet-restart-policy-approximation"] {
        let explained = boxferry_command().args(["explain", rule]).output()?;
        assert!(explained.status.success());
        let explained = String::from_utf8(explained.stdout)?;
        assert!(explained.contains("BFQ0009 quadlet-restart-policy-approximation"));
        assert!(explained.contains("owner: Quadlet adapter"));
        assert!(explained.contains("help:"));
    }

    let json = boxferry_command()
        .args(["rules", "--console-format", "json"])
        .output()?;
    assert!(json.status.success());
    assert!(json.stderr.is_empty());
    let catalogue: serde_json::Value = serde_json::from_slice(&json.stdout)?;
    let rules = catalogue["rules"].as_array().ok_or("rules array")?;
    assert!(rules.iter().any(|rule| {
        rule["code"] == "BFQ0009" && rule["name"] == "quadlet-restart-policy-approximation" && rule["help"].is_string()
    }));

    let unknown = boxferry_command().args(["explain", "NOT-A-RULE"]).output()?;
    assert_eq!(unknown.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("run `boxferry rules`"));
    Ok(())
}

#[test]
fn generic_input_and_interpolation_argument_failures_are_fail_closed() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let missing_input = boxferry_command()
        .args(["validate", "--input-type", "compose", "--output-type", "quadlet"])
        .output()?;
    assert_eq!(missing_input.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_input.stderr).contains("at least one --input-file"));
    let env_file_without_interpolation = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--env-file",
            "values.env",
        ])
        .output()?;
    assert_eq!(env_file_without_interpolation.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&env_file_without_interpolation.stderr).contains("--interpolate"));
    for environment in ["", "1INVALID", "INVALID-NAME"] {
        let invalid_environment = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
                "--interpolate",
                &format!("--env={environment}"),
            ])
            .output()?;
        assert_eq!(invalid_environment.status.code(), Some(2), "{environment:?}");
    }
    Ok(())
}

#[test]
fn generic_clap_json_and_stdin_failure_contracts_are_fail_closed() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let unexpected_error_report_value = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--generate-error-report",
            "report.zip",
        ])
        .output()?;
    assert_eq!(unexpected_error_report_value.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unexpected_error_report_value.stderr).contains("unexpected argument"));
    let directory_without_error_report = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--error-report-directory",
            ".",
        ])
        .output()?;
    assert_eq!(directory_without_error_report.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&directory_without_error_report.stderr).contains("--generate-error-report"));
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
    fs::write(existing.path().join("sentinel"), "keep")?;
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
    for (minimum, maximum) in [("5.4", "5.8"), ("5.4.0", "6.0.2")] {
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
                "--podman-minimum-version",
                minimum,
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
            "{minimum} through {maximum}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty());
    }
    for (minimum, maximum) in [("5.3", "6.0"), ("5.4", "6.1"), ("6.0", "5.4")] {
        let result = boxferry_command()
            .args([
                "validate",
                "--input-type",
                "compose",
                "--output-type",
                "quadlet",
                "--input-file",
                path_text(&fixture)?,
                "--podman-minimum-version",
                minimum,
                "--podman-maximum-version",
                maximum,
                "--console-format",
                "json",
            ])
            .output()?;
        assert_eq!(result.status.code(), Some(1), "{minimum} through {maximum}");
        assert!(result.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert!(
            report["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFO1003"))
        );
    }
    Ok(())
}

#[test]
fn generic_json_preprocessing_failure_is_one_redacted_document() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("json-preprocess-failure");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    fs::write(&compose, "services: [not-valid\n")?;
    fs::write(project.path().join("compose.yml"), "services: {}\n")?;
    let result = boxferry_command()
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
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "failure");
    assert_eq!(report["inputs"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["inputs"][0]["alias"], "<input-1>");
    assert_eq!(report["discovery"].as_array().map(Vec::len), Some(1));
    assert_eq!(std::str::from_utf8(&result.stdout)?.lines().count(), 1);
    Ok(())
}

#[test]
fn compose_post_discovery_failures_preserve_context_without_serializing_paths() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("post-resolution-input-canary");
    fs::create_dir_all(project.path())?;
    let compose = project.path().join("compose.yaml");
    let environment = project.path().join("post-resolution-environment-canary.env");
    fs::write(
        &compose,
        "name: post-resolution\nservices:\n  app:\n    image: example.invalid/app:1\n",
    )?;
    fs::write(&environment, "not a valid environment assignment\n")?;

    let target_failure = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--podman-minimum-version",
            "5.9",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(target_failure.status.code(), Some(1));
    assert!(target_failure.stderr.is_empty());
    let target_report: serde_json::Value = serde_json::from_slice(&target_failure.stdout)?;
    assert_eq!(target_report["failed_stage"], "conversion");
    assert_eq!(target_report["inputs"].as_array().map(Vec::len), Some(1));
    assert_eq!(target_report["inputs"][0]["alias"], "<input-1>");
    assert_eq!(target_report["discovery"].as_array().map(Vec::len), Some(1));
    assert!(
        target_report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFO1003"))
    );
    let target_json = String::from_utf8(target_failure.stdout)?;
    assert_eq!(target_json.lines().count(), 1);
    assert!(!target_json.contains("post-resolution-input-canary"));

    let human_failure = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--podman-minimum-version",
            "5.9",
        ])
        .output()?;
    assert_eq!(human_failure.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&human_failure.stdout).contains(&format!("input: {}", compose.display())));
    assert!(String::from_utf8_lossy(&human_failure.stdout).contains("stage: conversion failed"));
    assert!(String::from_utf8_lossy(&human_failure.stderr).contains("BFO1003"));

    let interpolation_failure = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-directory",
            path_text(project.path())?,
            "--interpolate",
            "--env-file",
            path_text(&environment)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(interpolation_failure.status.code(), Some(1));
    assert!(interpolation_failure.stderr.is_empty());
    let interpolation_json = String::from_utf8(interpolation_failure.stdout)?;
    let interpolation_report: serde_json::Value = serde_json::from_str(&interpolation_json)?;
    assert_eq!(interpolation_report["failed_stage"], "interpolation");
    assert_eq!(interpolation_report["inputs"].as_array().map(Vec::len), Some(1));
    assert_eq!(interpolation_report["discovery"].as_array().map(Vec::len), Some(1));
    assert!(
        interpolation_report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFO1004"))
    );
    assert_eq!(interpolation_json.lines().count(), 1);
    assert!(!interpolation_json.contains("post-resolution-input-canary"));
    assert!(!interpolation_json.contains("post-resolution-environment-canary.env"));
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
        "primary_diagnostic_code",
        "failure_summary",
        "fix_first",
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
        "fix_first",
        "span",
        "field",
        "diagnostic",
        "native_label",
        "native_finding",
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
    let diagnostic_required = schema["$defs"]["diagnostic"]["required"]
        .as_array()
        .ok_or("diagnostic required fields")?;
    for field in [
        "code",
        "name",
        "source_code",
        "severity",
        "summary",
        "help",
        "fields",
        "spans",
        "native_finding",
    ] {
        assert!(
            diagnostic_required.iter().any(|value| value == field),
            "missing diagnostic {field}"
        );
    }
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
            "--error-report-directory",
            path_text(bundle_directory.path())?,
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
    let bundle = generated_error_report(bundle_directory.path())?;
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
            "--error-report-directory",
            path_text(directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    assert!(fs::read_dir(target.path())?.next().is_none());
    fs::remove_file(directory.path())?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_tzif_environment_fails_closed_without_leaking_its_path() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-invalid-tzif");
    fs::create_dir_all(directory.path())?;
    let canary = "timezone-path-canary";
    let tzif = directory.path().join(format!("{canary}.tzif"));
    fs::write(&tzif, "not a TZif file")?;
    let report_file = directory.path().join("report.json");
    let result = boxferry_command()
        .env("TZ", path_text(&tzif)?)
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&fixture)?,
            "--report-file",
            path_text(&report_file)?,
            "--generate-error-report",
            "--error-report-directory",
            path_text(directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let console = String::from_utf8(result.stdout)?;
    let persisted = fs::read_to_string(&report_file)?;
    let stderr = String::from_utf8(result.stderr)?;
    for rendered in [&console, &persisted, &stderr] {
        assert!(!rendered.contains(canary), "timezone path leaked into {rendered}");
    }
    let console_report: serde_json::Value = serde_json::from_str(&console)?;
    assert_eq!(console_report["failed_stage"], "report-write");
    assert!(
        console_report["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFO3002"))
    );
    assert!(
        fs::read_dir(directory.path())?.all(|entry| {
            entry.is_ok_and(|entry| entry.path().extension().is_none_or(|extension| extension != "zip"))
        })
    );
    Ok(())
}

#[test]
fn report_file_is_attempted_before_the_support_bundle() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-ordering");
    fs::create_dir_all(directory.path())?;
    let report = directory.path().join("report.json");
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
            "--error-report-directory",
            path_text(directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let mut archive = ZipArchive::new(Cursor::new(fs::read(generated_error_report(directory.path())?)?))?;
    let mut entry = archive.by_name("report.json")?;
    let mut bundled = String::new();
    std::io::Read::read_to_string(&mut entry, &mut bundled)?;
    assert!(
        serde_json::from_str::<serde_json::Value>(&bundled)?["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFO3001"))
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
    let native = value["diagnostics"]
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item["native_finding"].as_object()))
        .ok_or("missing native Compose failure details")?;
    assert_eq!(native["source_format"], "compose");
    assert_eq!(native["producer"], "compose-lens");
    assert_eq!(native["stage"], "load");
    assert_eq!(native["labels"][0]["kind"], "primary");
    assert_ne!(value["exit_category"], "report-write");
    assert!(
        value["diagnostics"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["code"] == "BFO3001"))
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
                "--error-report-directory",
                path_text(directory.path())?,
            ])
            .args(presentation)
            .output()?;
        assert!(
            result.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let archive = generated_error_report(directory.path())?;
        let stdout = String::from_utf8(result.stdout)?;
        let absolute_archive = archive.canonicalize()?;
        match label {
            "quiet" => assert_eq!(stdout, format!("{}\n", absolute_archive.display())),
            "json" => {
                let console: serde_json::Value = serde_json::from_str(&stdout)?;
                assert_eq!(console["error_report_path"], absolute_archive.display().to_string());
            }
            _ => assert!(stdout.contains(&format!("error report: {}", absolute_archive.display()))),
        }
        let mut zip = ZipArchive::new(Cursor::new(fs::read(archive)?))?;
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
                assert!(report.get("error_report_path").is_none());
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
                "--error-report-directory",
                path_text(directory.path())?,
                "--console-format",
                "json",
            ])
            .output()?;
        assert_eq!(result.status.code(), Some(expected_exit));
        let mut zip = ZipArchive::new(Cursor::new(fs::read(generated_error_report(directory.path())?)?))?;
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
fn support_bundle_retries_existing_names_without_leaving_a_temporary_file() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let directory = TemporaryOutput::new("support-bundle-existing");
    fs::create_dir_all(directory.path())?;
    let archive = directory.path().join("unrelated-existing.zip");
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
            "--error-report-directory",
            path_text(directory.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success());
    assert_eq!(fs::read_to_string(&archive)?, "existing archive");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert!(report.get("error_report_path").is_some());
    assert!(
        fs::read_dir(directory.path())?
            .all(|entry| entry.is_ok_and(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp")))
    );
    Ok(())
}

#[test]
fn quadlet_to_compose_generic_route_writes_canonical_document_and_complete_report() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-to-compose-project");
    let output = TemporaryOutput::new("quadlet-to-compose-output");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(output.path())?;
    let input = project.path().join("web.container");
    fs::write(
        &input,
        "[Service]\nRestart=no\n[Container]\nImage=example.invalid/web:1\n",
    )?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["source_type"], "quadlet");
    assert_eq!(report["target_type"], "compose");
    assert_eq!(report["requested_versions"]["minimum"], "rolling");
    assert_eq!(report["requested_versions"]["maximum"], "rolling");
    assert_eq!(report["resolved_versions"]["minimum"], "rolling");
    assert_eq!(report["resolved_versions"]["maximum"], "rolling");
    assert!(report["choices"].as_array().is_some_and(|choices| {
        choices.iter().all(|choice| {
            !matches!(
                choice["name"].as_str(),
                Some("compose_provider" | "compose_provider_version" | "compose_runtime" | "compose_runtime_version")
            )
        })
    }));
    assert_eq!(report["output_artifacts"][0]["name"], "compose.yaml");
    assert!(
        report["output_artifacts"][0]["size"]
            .as_u64()
            .is_some_and(|size| size > 0)
    );
    let document = fs::read_to_string(output.path().join("compose.yaml"))?;
    assert!(document.contains("name: \"example\""));
    assert!(document.contains("\"web\""));
    let verbose = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--verbose",
        ])
        .output()?;
    assert!(verbose.status.success());
    assert!(String::from_utf8(verbose.stdout)?.contains("Compose Specification: rolling"));
    let collision = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--output-directory",
            path_text(output.path())?,
        ])
        .output()?;
    assert_eq!(collision.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&collision.stderr).contains("output directory is not empty"));
    Ok(())
}

#[test]
fn quadlet_route_validates_without_writing_and_rejects_inapplicable_or_removed_options() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-route-validation");
    fs::create_dir_all(project.path())?;
    let input = project.path().join("web.container");
    fs::write(&input, "[Container]\nImage=example.invalid/web:1\n")?;
    let base = [
        "validate",
        "--input-type",
        "quadlet",
        "--output-type",
        "compose",
        "--input-file",
        path_text(&input)?,
        "--project-name",
        "example",
        "--console-format",
        "json",
    ];
    let validated = boxferry_command().args(base).output()?;
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    assert!(!project.path().join("compose.yaml").exists());
    let missing_project = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
        ])
        .output()?;
    assert_eq!(missing_project.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_project.stderr).contains("--project-name is required"));
    let irrelevant = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--podman-minimum-version",
            "5.4",
        ])
        .output()?;
    assert_eq!(irrelevant.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&irrelevant.stderr).contains("not applicable"));
    let removed_provider = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--compose-provider",
            "docker-compose",
        ])
        .output()?;
    assert_eq!(removed_provider.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed_provider.stderr).contains("unexpected argument '--compose-provider'"));
    let removed_runtime = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--compose-runtime",
            "docker-engine",
        ])
        .output()?;
    assert_eq!(removed_runtime.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed_runtime.stderr).contains("unexpected argument '--compose-runtime'"));
    Ok(())
}

#[test]
fn quadlet_directory_discovery_is_lowercase_lexical_and_refuses_duplicate_unit_names() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-directory-discovery");
    let second = TemporaryOutput::new("quadlet-directory-second");
    let middle = TemporaryOutput::new("quadlet-directory-middle");
    let output = TemporaryOutput::new("quadlet-directory-output");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(second.path())?;
    fs::create_dir_all(middle.path())?;
    fs::write(
        project.path().join("z.container"),
        "[Container]\nImage=example.invalid/z:1\n",
    )?;
    fs::write(
        project.path().join("a.container"),
        "[Container]\nImage=example.invalid/a:1\n",
    )?;
    fs::write(project.path().join("ignored.CONTAINER"), "not a unit\n")?;
    let middle_input = middle.path().join("middle.container");
    fs::write(&middle_input, "[Container]\nImage=example.invalid/middle:1\n")?;
    fs::write(
        second.path().join("a.container"),
        "[Container]\nImage=example.invalid/other:1\n",
    )?;
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&middle_input)?,
            "--input-directory",
            path_text(project.path())?,
            "--project-name",
            "example",
            "--output-directory",
            path_text(output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["inputs"].as_array().map(Vec::len), Some(3));
    assert_eq!(report["discovery"][0]["ignored"].as_array().map(Vec::len), Some(1));
    let document = fs::read_to_string(output.path().join("compose.yaml"))?;
    let middle = document.find("\"middle\"").ok_or("missing middle")?;
    let first = document.find("\"a\"").ok_or("missing a")?;
    let last = document.find("\"z\"").ok_or("missing z")?;
    assert!(middle < first && first < last, "unexpected service order: {document}");
    let duplicate = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-directory",
            path_text(project.path())?,
            "--input-directory",
            path_text(second.path())?,
            "--project-name",
            "example",
        ])
        .output()?;
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate Quadlet unit basename"));
    Ok(())
}

#[test]
fn quadlet_parse_failure_uses_the_route_specific_report_stage() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-parse-report");
    fs::create_dir_all(project.path())?;
    let input = project.path().join("broken-source-canary.container");
    fs::write(&input, "[Container]\nImage=\n")?;
    fs::write(project.path().join("ignored.txt"), "not a unit\n")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-directory",
            path_text(project.path())?,
            "--project-name",
            "example",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["failed_stage"], "quadlet-parse");
    assert!(!report["diagnostics"].as_array().unwrap_or(&Vec::new()).is_empty());
    assert_eq!(report["inputs"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["discovery"].as_array().map(Vec::len), Some(1));
    assert!(
        !result
            .stdout
            .windows(b"broken-source-canary".len())
            .any(|window| window == b"broken-source-canary")
    );
    Ok(())
}

fn assert_detailed_quadlet_console_and_persisted_reports(
    result: Output,
    report_file: &Path,
    reports_directory: &Path,
    source_canary: &str,
    value_canary: &str,
) -> Result<serde_json::Value, Box<dyn Error>> {
    assert_eq!(result.status.code(), Some(1));
    let console = String::from_utf8(result.stdout)?;
    let bundle = generated_error_report(reports_directory)?;
    let console_report: serde_json::Value = serde_json::from_str(&console)?;
    let mut persisted_console = console_report.clone();
    let error_report_path = persisted_console
        .as_object_mut()
        .and_then(|object| object.remove("error_report_path"))
        .ok_or("missing console error report path")?;
    assert!(
        error_report_path
            .as_str()
            .is_some_and(|path| Path::new(path).is_absolute())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(report_file)?)?,
        persisted_console
    );
    let mut archive = ZipArchive::new(fs::File::open(bundle)?)?;
    let mut archived = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("report.json")?, &mut archived)?;
    assert_eq!(serde_json::from_str::<serde_json::Value>(&archived)?, persisted_console);
    for rendered in [&console, &fs::read_to_string(report_file)?, &archived] {
        assert!(!rendered.contains(source_canary));
        assert!(!rendered.contains(value_canary));
        assert!(!rendered.contains("missing.network"));
    }
    Ok(console_report)
}

fn assert_quadlet_document_set_failure(input: &Path) -> Result<(), Box<dyn Error>> {
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(input)?,
            "--project-name",
            "example",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["failed_stage"], "conversion");
    assert!(report["diagnostics"].as_array().is_some_and(|diagnostics| {
        diagnostics.iter().any(|diagnostic| {
            diagnostic["source_code"]
                .as_str()
                .is_some_and(|code| code.starts_with("QL"))
        })
    }));
    Ok(())
}

#[test]
fn quadlet_detailed_native_diagnostics_are_ordered_aliased_and_redacted_everywhere() -> Result<(), Box<dyn Error>> {
    let inputs = TemporaryOutput::new("quadlet-detailed-canary-inputs");
    let reports = TemporaryOutput::new("quadlet-detailed-canary-reports");
    fs::create_dir_all(inputs.path())?;
    fs::create_dir_all(reports.path())?;
    let source_canary = "raw-source-canary";
    let value_canary = "secret-value-canary";
    let first = inputs.path().join(format!("{source_canary}-first.container"));
    let second = inputs.path().join(format!("{source_canary}-second.container"));
    fs::write(&first, "[Container]\nImage\n")?;
    fs::write(
        &second,
        format!("[Container]\nImage={value_canary}.image\nNetwork=missing.network\n"),
    )?;
    let report_file = reports.path().join("report.json");
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&first)?,
            "--input-file",
            path_text(&second)?,
            "--project-name",
            "example",
            "--report-file",
            path_text(&report_file)?,
            "--generate-error-report",
            "--error-report-directory",
            path_text(reports.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    let console_report = assert_detailed_quadlet_console_and_persisted_reports(
        result,
        &report_file,
        reports.path(),
        source_canary,
        value_canary,
    )?;
    assert_eq!(console_report["failed_stage"], "quadlet-parse");
    let diagnostics = console_report["diagnostics"].as_array().ok_or("missing diagnostics")?;
    assert_eq!(diagnostics[0]["code"], "BFQ1101");
    assert_eq!(diagnostics[0]["source_code"], "QLS0001");
    assert_eq!(diagnostics[0]["native_finding"]["source_format"], "quadlet");
    assert_eq!(diagnostics[0]["native_finding"]["producer"], "quadlet-lens");
    assert_eq!(diagnostics[0]["native_finding"]["stage"], "syntax");
    assert_eq!(diagnostics[0]["native_finding"]["labels"][0]["kind"], "primary");
    assert_eq!(diagnostics[1]["code"], "BFQ1102");
    assert_eq!(diagnostics[1]["source_code"], "QLM0002");
    assert!(diagnostics.len() >= 3);
    assert_eq!(diagnostics[0]["spans"][0]["source"], "<input-1>");
    assert_eq!(diagnostics[1]["spans"][0]["source"], "<input-1>");
    assert!(
        diagnostics[2]["spans"]
            .as_array()
            .is_some_and(|spans| { spans.iter().any(|span| span["source"] == "<input-2>") })
    );
    assert!(diagnostics[..3].iter().all(|diagnostic| {
        diagnostic["fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field["name"] == "label_message"))
    }));
    let human = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&first)?,
            "--input-file",
            path_text(&second)?,
            "--project-name",
            "example",
        ])
        .output()?;
    assert_eq!(human.status.code(), Some(1));
    let stderr = String::from_utf8(human.stderr)?;
    assert!(stderr.contains("BFQ1101 quadlet-native-syntax [error]"));
    assert!(!stderr.contains("source rule:"));
    assert!(stderr.contains(diagnostics[0]["summary"].as_str().ok_or("missing summary")?));
    assert!(
        stderr.contains(
            diagnostics[0]["fields"][0]["value"]
                .as_str()
                .ok_or("missing label message")?
        )
    );

    assert_quadlet_document_set_failure(&second)?;
    Ok(())
}

#[test]
fn quadlet_recoverable_native_diagnostics_are_reported_on_success() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-native-warning");
    fs::create_dir_all(project.path())?;
    let input = project.path().join("web.container");
    fs::write(&input, "[Container]\nImage=example.invalid/web:1\n[Network]\n")?;
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "success");
    let warning = report["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic["severity"] == "warning")
        })
        .ok_or("missing recoverable native warning")?;
    assert_eq!(warning["code"], "BFQ1102");
    assert_eq!(warning["native_finding"]["source_format"], "quadlet");
    assert_eq!(warning["native_finding"]["stage"], "model");
    assert!(
        warning["source_code"]
            .as_str()
            .is_some_and(|code| code.starts_with("QLM"))
    );
    assert_eq!(warning["spans"][0]["source"], "<input-1>");
    assert!(
        warning["fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field["name"] == "label_message"))
    );

    let human = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
        ])
        .output()?;
    assert!(human.status.success(), "{}", String::from_utf8_lossy(&human.stderr));
    let stderr = String::from_utf8(human.stderr)?;
    assert!(stderr.contains(warning["code"].as_str().ok_or("missing warning code")?));
    assert!(stderr.contains(warning["summary"].as_str().ok_or("missing warning summary")?));
    Ok(())
}

#[test]
fn quadlet_environment_file_requires_approximate_authorization() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-environment-file-policy");
    let exact_output = TemporaryOutput::new("quadlet-environment-file-exact");
    let approximate_output = TemporaryOutput::new("quadlet-environment-file-approximate");
    fs::create_dir_all(project.path())?;
    let input = project.path().join("web.container");
    fs::write(
        &input,
        "[Container]\nImage=example.invalid/web:1\nEnvironmentFile=/etc/example/default.env\n",
    )?;
    let exact = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--output-directory",
            path_text(exact_output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(exact.status.code(), Some(2));
    assert!(!exact_output.path().exists());
    let exact_report: serde_json::Value = serde_json::from_slice(&exact.stdout)?;
    assert!(
        exact_report["fidelity"]["approximate"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    let approximate = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--loss-policy",
            "approximate",
            "--output-directory",
            path_text(approximate_output.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(
        approximate.status.success(),
        "{}",
        String::from_utf8_lossy(&approximate.stderr)
    );
    assert!(approximate_output.path().join("compose.yaml").exists());
    Ok(())
}

#[test]
fn unavailable_routes_and_cross_route_options_fail_without_misreporting_the_route() -> Result<(), Box<dyn Error>> {
    let unavailable = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "compose",
            "--input-file",
            "-",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(unavailable.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&unavailable.stdout)?;
    assert_eq!(report["source_type"], "compose");
    assert_eq!(report["target_type"], "compose");
    assert!(report["choices"].as_array().is_some_and(Vec::is_empty));
    let compose = fixture_directory("compose-to-quadlet-dependencies").join("compose.yaml");
    let removed_provider = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "compose",
            "--output-type",
            "quadlet",
            "--input-file",
            path_text(&compose)?,
            "--compose-provider",
            "docker-compose",
            "--compose-provider-version",
            "2.24.4",
        ])
        .output()?;
    assert_eq!(removed_provider.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed_provider.stderr).contains("unexpected argument '--compose-provider'"));
    let project = TemporaryOutput::new("quadlet-cross-options");
    fs::create_dir_all(project.path())?;
    let quadlet = project.path().join("web.container");
    fs::write(&quadlet, "[Container]\nImage=example.invalid/web:1\n")?;
    let quadlet_flag = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&quadlet)?,
            "--project-name",
            "example",
            "--interpolate",
        ])
        .output()?;
    assert_eq!(quadlet_flag.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&quadlet_flag.stderr).contains("not applicable"));
    Ok(())
}

#[test]
fn quadlet_input_rejects_stdin_empty_discovery_duplicate_paths_and_unsupported_extensions() -> Result<(), Box<dyn Error>>
{
    let project = TemporaryOutput::new("quadlet-input-rejections");
    let empty = TemporaryOutput::new("quadlet-empty-discovery");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(empty.path())?;
    let input = project.path().join("web.container");
    let unsupported = project.path().join("web.txt");
    fs::write(&input, "[Container]\nImage=example.invalid/web:1\n")?;
    fs::write(&unsupported, "ignored\n")?;
    let common = [
        "--input-type",
        "quadlet",
        "--output-type",
        "compose",
        "--project-name",
        "example",
    ];
    let stdin = boxferry_command()
        .args([
            "validate",
            common[0],
            common[1],
            common[2],
            common[3],
            "--input-file",
            "-",
            common[4],
            common[5],
        ])
        .output()?;
    assert_eq!(stdin.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&stdin.stderr).contains("stdin is not supported"));
    let empty = boxferry_command()
        .args([
            "validate",
            common[0],
            common[1],
            common[2],
            common[3],
            "--input-directory",
            path_text(empty.path())?,
            common[4],
            common[5],
        ])
        .output()?;
    assert_eq!(empty.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no supported Quadlet unit files"));
    let duplicate = boxferry_command()
        .args([
            "validate",
            common[0],
            common[1],
            common[2],
            common[3],
            "--input-file",
            path_text(&input)?,
            "--input-file",
            path_text(&input)?,
            common[4],
            common[5],
        ])
        .output()?;
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate resolved Quadlet input"));
    let extension = boxferry_command()
        .args([
            "validate",
            common[0],
            common[1],
            common[2],
            common[3],
            "--input-file",
            path_text(&unsupported)?,
            common[4],
            common[5],
        ])
        .output()?;
    assert_eq!(extension.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&extension.stderr).contains("supported lower-case unit extension"));
    Ok(())
}

#[test]
fn duplicate_quadlet_basenames_are_redacted_in_json_report_file_and_bundle() -> Result<(), Box<dyn Error>> {
    let first = TemporaryOutput::new("quadlet-duplicate-canary-first");
    let second = TemporaryOutput::new("quadlet-duplicate-canary-second");
    let reports = TemporaryOutput::new("quadlet-duplicate-canary-reports");
    fs::create_dir_all(first.path())?;
    fs::create_dir_all(second.path())?;
    fs::create_dir_all(reports.path())?;
    let canary = "raw-duplicate-canary";
    for directory in [first.path(), second.path()] {
        fs::write(
            directory.join(format!("{canary}.container")),
            "[Container]\nImage=example.invalid/web:1\n",
        )?;
    }
    let report_file = reports.path().join("report.json");
    let result = boxferry_command()
        .args([
            "validate",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-directory",
            path_text(first.path())?,
            "--input-directory",
            path_text(second.path())?,
            "--project-name",
            "example",
            "--report-file",
            path_text(&report_file)?,
            "--generate-error-report",
            "--error-report-directory",
            path_text(reports.path())?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let console = String::from_utf8(result.stdout)?;
    let file = fs::read_to_string(report_file)?;
    let bundle = generated_error_report(reports.path())?;
    assert!(!console.contains(canary));
    assert!(!file.contains(canary));
    let mut archive = ZipArchive::new(fs::File::open(bundle)?)?;
    assert_eq!(archive.len(), 2);
    let mut bundled = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("report.json")?, &mut bundled)?;
    assert!(!bundled.contains(canary));
    Ok(())
}

#[test]
fn quadlet_to_compose_support_bundle_excludes_source_canaries() -> Result<(), Box<dyn Error>> {
    let project = TemporaryOutput::new("quadlet-compose-bundle-project");
    let output = TemporaryOutput::new("quadlet-compose-bundle-output");
    let reports = TemporaryOutput::new("quadlet-compose-bundle-reports");
    fs::create_dir_all(project.path())?;
    fs::create_dir_all(reports.path())?;
    let input = project.path().join("raw-path-canary.container");
    let canary = "quadlet-source-canary-never-report";
    fs::write(
        &input,
        format!("[Container]\nImage=example.invalid/web:1\nEnvironment=SECRET={canary}\n"),
    )?;
    let report_file = reports.path().join("report.json");
    let result = boxferry_command()
        .args([
            "convert",
            "--input-type",
            "quadlet",
            "--output-type",
            "compose",
            "--input-file",
            path_text(&input)?,
            "--project-name",
            "example",
            "--loss-policy",
            "partial",
            "--output-directory",
            path_text(output.path())?,
            "--generate-error-report",
            "--error-report-directory",
            path_text(reports.path())?,
            "--report-file",
            path_text(&report_file)?,
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let console = String::from_utf8(result.stdout)?;
    let file_report = fs::read_to_string(report_file)?;
    let bundle = generated_error_report(reports.path())?;
    assert!(!console.contains(canary));
    assert!(!console.contains("raw-path-canary"));
    assert!(!file_report.contains(canary));
    assert!(!file_report.contains("raw-path-canary"));
    let mut archive = ZipArchive::new(fs::File::open(bundle)?)?;
    let mut report = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("report.json")?, &mut report)?;
    assert!(!report.contains(canary));
    assert!(!report.contains("raw-path-canary"));
    assert_eq!(archive.len(), 2);
    assert_eq!(archive.by_index(0)?.name(), "README.md");
    assert_eq!(archive.by_index(1)?.name(), "report.json");
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

fn generated_error_report(directory: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let mut reports = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "zip"));
    let report = reports.next().ok_or("missing generated error report")?;
    if reports.next().is_some() {
        return Err("more than one generated error report".into());
    }
    Ok(report)
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
