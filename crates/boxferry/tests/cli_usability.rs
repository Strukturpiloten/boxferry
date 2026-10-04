//! Fast, offline user-journey contracts for the public CLI.
//!
//! These expectations are authored here, independently of the CLI implementation.
//! A Podman cassette serves read-only API responses; no runtime is started.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[path = "support/podman_cassette.rs"]
#[allow(dead_code)] // Shared cassette helper has mutation methods used by other integration tests.
mod podman_cassette;

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use podman_cassette::{PodmanCassette, PodmanCassetteServer};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
const FORMATS: [&str; 3] = ["compose", "podman", "quadlet"];

#[test]
fn single_quadlet_unit_infers_application_for_validation_and_every_exporter() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("single-quadlet-identity")?;
    let input = scratch.path().join("web.container");
    fs::write(&input, "[Container]\nImage=example.invalid/web:1\n")?;

    for output in FORMATS {
        for verb in ["validate", "convert"] {
            let destination = scratch.path().join(format!("{verb}-{output}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
            command.args([verb, "quadlet", output, "--input-file"]).arg(&input);
            if output == "podman" {
                command.args(["--podman-target-context", "unknown"]);
            }
            if verb == "convert" {
                command.arg("--output-directory").arg(&destination);
            }
            let result = command.args(["--console-format", "json"]).output()?;
            assert_eq!(
                result.status.code(),
                Some(0),
                "{verb} quadlet {output}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert!(result.stderr.is_empty());
            let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
            assert_eq!(report["application"], "web", "{verb} quadlet {output}");
            assert_eq!(report["status"], "success");
            assert_eq!(report["source_type"], "quadlet");
            assert_eq!(report["target_type"], output);
            if verb == "validate" {
                assert!(!destination.exists());
            } else {
                assert_eq!(artifact_names(&destination)?, expected_artifacts("quadlet", output));
            }
        }
    }

    let existing = scratch.path().join("convert-compose/compose.yaml");
    let original = fs::read(&existing)?;
    let no_clobber = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "quadlet", "compose", "--input-file"])
        .arg(&input)
        .arg("--output-directory")
        .arg(scratch.path().join("convert-compose"))
        .output()?;
    assert!(!no_clobber.status.success());
    assert_eq!(fs::read(&existing)?, original);

    let from_directory = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "quadlet", "compose", "--input-directory"])
        .arg(scratch.path())
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(from_directory.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&from_directory.stdout)?;
    assert_eq!(report["application"], "web");
    assert_eq!(report["inputs"][0]["kind"], "discovered");

    let override_name = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "quadlet", "compose", "--input-file"])
        .arg(&input)
        .args(["--application-name", "chosen-app", "--console-format", "json"])
        .output()?;
    assert_eq!(override_name.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&override_name.stdout)?;
    assert_eq!(report["application"], "chosen-app");
    Ok(())
}

#[test]
fn multiple_quadlet_units_require_explicit_neutral_application_identity() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("quadlet-identity-ambiguity")?;
    fs::write(
        scratch.path().join("first.container"),
        "[Container]\nImage=example.invalid/first:1\n",
    )?;
    fs::write(
        scratch.path().join("second.container"),
        "[Container]\nImage=example.invalid/second:1\n",
    )?;
    let destination = scratch.path().join("blocked");
    let missing_name = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "quadlet", "compose", "--input-directory"])
        .arg(scratch.path())
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(missing_name.status.code(), Some(1));
    assert!(!destination.exists());
    let report: serde_json::Value = serde_json::from_slice(&missing_name.stdout)?;
    assert_eq!(report["status"], "failure");
    assert_eq!(report["failed_stage"], "input-discovery");
    assert_eq!(report["primary_diagnostic_code"], "BFO1000");
    assert!(report["diagnostics"][0]["fields"].as_array().is_some_and(|fields| {
        fields.iter().any(|field| {
            field["name"] == "reason"
                && field["value"]
                    .as_str()
                    .is_some_and(|value| value.contains("multiple Quadlet unit files need --application-name"))
        })
    }));
    assert_eq!(report["output_artifacts"], serde_json::json!([]));

    let explicit = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "quadlet", "compose", "--input-directory"])
        .arg(scratch.path())
        .args(["--application-name", "chosen-app", "--console-format", "json"])
        .output()?;
    assert_eq!(
        explicit.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&explicit.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&explicit.stdout)?;
    assert_eq!(report["application"], "chosen-app");
    Ok(())
}

#[test]
fn minimal_compose_discovery_preserves_one_authored_service_across_all_outputs() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("implicit-compose")?;
    fs::write(
        scratch.path().join("compose.yaml"),
        "name: discovered\nservices:\n  web:\n    image: example.invalid/web:1\n",
    )?;
    fs::write(scratch.path().join(".env"), "PRIVATE_CANARY=not-an-input\n")?;

    for output in FORMATS {
        let mut validation = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        validation
            .current_dir(scratch.path())
            .args(["validate", "compose", output]);
        if output == "podman" {
            validation.args(["--podman-target-context", "unknown"]);
        }
        let validation = validation.args(["--console-format", "json"]).output()?;
        assert_eq!(
            validation.status.code(),
            Some(0),
            "{output}: {}",
            String::from_utf8_lossy(&validation.stdout)
        );
        assert!(validation.stderr.is_empty(), "{output}");
        let planned: serde_json::Value = serde_json::from_slice(&validation.stdout)?;
        assert_implicit_compose_report(&planned, scratch.path(), output)?;

        let destination = scratch.path().join(format!("generated-{output}"));
        let mut conversion = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        conversion
            .current_dir(scratch.path())
            .args(["convert", "compose", output]);
        if output == "podman" {
            conversion.args(["--podman-target-context", "unknown"]);
        }
        let conversion = conversion
            .arg("--output-directory")
            .arg(&destination)
            .args(["--console-format", "json"])
            .output()?;
        assert_eq!(
            conversion.status.code(),
            Some(0),
            "{output}: {}",
            String::from_utf8_lossy(&conversion.stdout)
        );
        assert!(conversion.stderr.is_empty(), "{output}");
        let converted: serde_json::Value = serde_json::from_slice(&conversion.stdout)?;
        assert_implicit_compose_report(&converted, scratch.path(), output)?;
        assert_eq!(
            planned["fidelity"], converted["fidelity"],
            "{output}: validation changed fidelity"
        );
        assert_eq!(
            planned["diagnostics"], converted["diagnostics"],
            "{output}: validation changed findings"
        );
        let artifacts = artifact_names(&destination)?;
        assert_eq!(artifacts, expected_artifacts("compose", output), "{output}");
        let written = match output {
            "compose" => fs::read_to_string(destination.join("compose.yaml"))?,
            "quadlet" => fs::read_to_string(destination.join("web.container"))?,
            "podman" => fs::read_to_string(destination.join("podman.json"))?,
            _ => unreachable!("finite output formats"),
        };
        assert!(
            written.contains("example.invalid/web:1"),
            "{output}: authored image disappeared"
        );
        assert!(
            !written.contains("not-an-input"),
            "{output}: implicit .env value leaked"
        );
    }
    Ok(())
}

fn assert_implicit_compose_report(report: &serde_json::Value, root: &Path, output: &str) -> Result<(), Box<dyn Error>> {
    assert_eq!(report["status"], "success", "{output}");
    assert_eq!(report["source_type"], "compose", "{output}");
    assert_eq!(report["target_type"], output, "{output}");
    assert_eq!(report["application"], "discovered", "{output}");
    assert_eq!(
        report["inputs"],
        serde_json::json!([{ "alias": "<input-1>", "kind": "discovered" }])
    );
    assert_eq!(report["discovery"][0]["selected"], "<input-1>");
    assert_eq!(report["discovery"].as_array().map(Vec::len), Some(1));
    let encoded = serde_json::to_string(report)?;
    assert!(
        !encoded.contains(&root.display().to_string()),
        "{output}: source path leaked"
    );
    assert!(
        !encoded.contains("not-an-input"),
        "{output}: implicit .env value leaked"
    );
    Ok(())
}

#[test]
fn minimal_compose_discovery_fails_closed_on_no_match_and_ambiguity() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("implicit-compose-failure")?;
    fs::create_dir(scratch.path().join("child"))?;
    fs::write(
        scratch.path().join("child/compose.yaml"),
        "services:\n  hidden:\n    image: example.invalid/hidden:1\n",
    )?;
    fs::write(scratch.path().join(".env"), "PRIVATE_CANARY=not-an-input\n")?;
    let destination = scratch.path().join("generated");
    let no_match = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .current_dir(scratch.path())
        .args(["convert", "compose", "compose", "--output-directory"])
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(no_match.status.code(), Some(1));
    assert!(no_match.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&no_match.stdout)?;
    assert_eq!(report["failed_stage"], "input-discovery");
    assert_eq!(report["primary_diagnostic_code"], "BFO1000");
    assert!(
        report["diagnostics"][0]["fields"]
            .to_string()
            .contains("no conventional Compose file")
    );
    assert!(!destination.exists());

    fs::write(
        scratch.path().join("compose.yaml"),
        "name: chosen\nservices:\n  web:\n    image: example.invalid/web:1\n",
    )?;
    fs::write(
        scratch.path().join("docker-compose.yml"),
        "name: other\nservices:\n  other:\n    image: example.invalid/other:1\n",
    )?;
    let ambiguous = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .current_dir(scratch.path())
        .args(["validate", "compose", "quadlet", "--console-format", "json"])
        .output()?;
    assert_eq!(ambiguous.status.code(), Some(1));
    assert!(ambiguous.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&ambiguous.stdout)?;
    assert_eq!(report["failed_stage"], "input-discovery");
    assert_eq!(report["primary_diagnostic_code"], "BFO1000");
    let reason = report["diagnostics"][0]["fields"].to_string();
    assert!(reason.contains("multiple conventional Compose files"));
    assert!(reason.contains("compose.yaml") && reason.contains("docker-compose.yml"));
    assert!(!reason.contains(&scratch.path().display().to_string()));
    assert!(!destination.exists());

    let explicit = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .current_dir(scratch.path())
        .args([
            "validate",
            "compose",
            "compose",
            "--input-file",
            "compose.yaml",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(
        explicit.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&explicit.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&explicit.stdout)?;
    assert_eq!(report["inputs"][0]["kind"], "file");
    assert_eq!(report["application"], "chosen");
    Ok(())
}

#[test]
fn all_eighteen_cli_route_forms_preserve_a_reviewed_user_intent() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("route-forms")?;
    let mut forms = BTreeSet::new();
    let mut validation_evidence = BTreeMap::new();

    for input in FORMATS {
        for output in FORMATS {
            for verb in ["validate", "convert"] {
                let label = format!("{verb}-{input}-{output}");
                let destination = scratch.path().join(&label);
                let result = run_route(verb, input, output, &destination)?;
                assert_eq!(
                    result.status.code(),
                    Some(0),
                    "{label}: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
                assert!(
                    result.stderr.is_empty(),
                    "{label}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );

                let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
                assert_eq!(report["schema_version"], 1, "{label}");
                assert_eq!(report["status"], "success", "{label}");
                assert_eq!(report["exit_category"], "success", "{label}");
                assert_eq!(report["source_type"], input, "{label}");
                assert_eq!(report["target_type"], output, "{label}");
                assert_eq!(report["application"], application(input), "{label}");
                assert_eq!(report["invocation"]["command_kind"], verb, "{label}");
                if output == "quadlet" {
                    assert_eq!(report["requested_versions"]["minimum"], "5.4", "{label}");
                    assert_eq!(report["requested_versions"]["maximum"], "6.1", "{label}");
                    assert_eq!(report["resolved_versions"]["minimum"], "5.4.0", "{label}");
                    assert_eq!(report["resolved_versions"]["maximum"], "6.1.2", "{label}");
                } else if output == "podman" {
                    assert_eq!(report["requested_versions"]["maximum"], "6.1", "{label}");
                    assert_eq!(report["resolved_versions"]["maximum"], "6.1.0", "{label}");
                }
                assert!(report["failed_stage"].is_null(), "{label}");
                assert_structured_diagnostics(&report, &label)?;
                assert_expected_diagnostics(&report, input, output, &label)?;
                let route = format!("{input}-{output}");
                let semantic_evidence = (
                    report["fidelity"].clone(),
                    diagnostic_codes_and_subjects(&report)?,
                    report["choices"].clone(),
                );
                if verb == "validate" {
                    assert!(validation_evidence.insert(route, semantic_evidence).is_none());
                } else {
                    assert_eq!(
                        validation_evidence.get(&route),
                        Some(&semantic_evidence),
                        "{label}: conversion changed the validation decision"
                    );
                }
                if input == "podman" {
                    assert_portable_podman_choices(&report, &result, &label);
                }

                let artifacts = report["output_artifacts"]
                    .as_array()
                    .ok_or_else(|| format!("{label}: missing artifact array"))?;
                let expected = expected_artifacts(input, output);
                let reported = artifacts
                    .iter()
                    .map(|artifact| {
                        artifact["name"]
                            .as_str()
                            .map(str::to_owned)
                            .ok_or("artifact without name")
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                assert_eq!(reported, expected, "{label}: planned artifact set");
                if verb == "validate" {
                    assert!(!destination.exists(), "{label}: validate wrote output");
                } else {
                    let actual = artifact_names(&destination)?;
                    assert_eq!(actual, expected, "{label}: artifact set");
                    assert_artifact_intent(&destination, input, output)?;
                    if input == "podman" {
                        for name in &actual {
                            assert_no_private_canaries(&fs::read(destination.join(name))?, &label);
                        }
                    }
                }
                assert!(forms.insert(label));
            }
        }
    }
    assert_eq!(forms.len(), 18);
    assert_eq!(validation_evidence.len(), 9);
    Ok(())
}

fn assert_portable_podman_choices(report: &serde_json::Value, result: &Output, label: &str) {
    assert!(report["choices"].as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|choice| choice["name"] == "podman_import_policy" && choice["value"] == "portable")
    }));
    for name in [
        "promote_portable_effective_settings",
        "promote_effective_named_volumes",
        "promote_effective_named_networks",
    ] {
        assert!(
            report["choices"].as_array().is_some_and(|choices| {
                choices
                    .iter()
                    .any(|choice| choice["name"] == name && choice["value"] == "true")
            }),
            "{label}: {name} not enabled by portable preset"
        );
    }
    assert_eq!(
        report["choices"][0]["value"], "partial",
        "{label}: unrelated fixture omissions need partial"
    );
    let options = report["invocation"]["provided_option_names"].as_array();
    assert!(
        options.is_some_and(|options| options.iter().all(|option| {
            !option
                .as_str()
                .is_some_and(|name| name.starts_with("--promote-podman-"))
        })),
        "{label}: portable import required a promotion flag"
    );
    assert_no_private_canaries(&result.stdout, label);
}

#[test]
fn portable_podman_import_does_not_silently_relax_exact_loss_policy() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("podman-exact-loss")?;
    let destination = scratch.path().join("blocked");
    let server = PodmanCassetteServer::start(PodmanCassette::load(&podman_fixture())?)?;
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "podman", "compose", "--podman-socket"])
        .arg(server.socket())
        .args([
            "--podman-resource",
            "container=portable-app",
            "--console-format",
            "json",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .output()?;
    server.finish()?;
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stderr.is_empty());
    assert_no_private_canaries(&result.stdout, "exact Podman policy");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["output_artifacts"], serde_json::json!([]));
    assert!(
        report["fidelity"]["unsupported"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert_expected_diagnostics(&report, "podman", "compose", "exact Podman policy")?;
    assert!(!destination.exists(), "an exact-policy refusal wrote output");
    Ok(())
}

#[test]
fn explicit_same_host_bind_promotion_retains_both_authored_paths_in_document_outputs() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("reviewed-podman-binds")?;
    // The cassette records two host-local binds. The route matrix deliberately
    // keeps the ordinary CLI default, which requires a separate same-host choice.
    for output in ["compose", "quadlet"] {
        let destination = scratch.path().join(output);
        let server = PodmanCassetteServer::start(PodmanCassette::load(&podman_fixture())?)?;
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "podman", output, "--podman-socket"])
            .arg(server.socket())
            .args([
                "--podman-resource",
                "container=portable-app",
                "--promote-podman-effective-bind-mounts",
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ])
            .arg("--output-directory")
            .arg(&destination)
            .output()?;
        server.finish()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "{output}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_no_private_canaries(&result.stdout, output);
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert!(report["choices"].as_array().is_some_and(|choices| {
            choices
                .iter()
                .any(|choice| choice["name"] == "promote_effective_bind_mounts" && choice["value"] == "true")
        }));
        let artifact = fs::read_to_string(destination.join(if output == "compose" {
            "compose.yaml"
        } else {
            "portable-app.container"
        }))?;
        for (source, target, mode) in [
            ("/srv/boxferry-scenario/private", "/etc/boxferry-scenario", "ro"),
            ("/srv/boxferry-scenario/shared", "/srv/boxferry-scenario", "rw"),
        ] {
            if output == "compose" {
                let binding = format!("      - {source}:{target}:");
                let line = artifact
                    .lines()
                    .find(|line| line.starts_with(&binding))
                    .ok_or_else(|| format!("{output}: missing associated bind {binding}"))?;
                let options = line.strip_prefix(&binding).ok_or("Compose bind options")?;
                assert_eq!(
                    options.split(',').any(|option| option == "ro"),
                    mode == "ro",
                    "{output}: wrong bind mode for {source}: {line}"
                );
            } else {
                let binding = format!("Volume={source}:{target}:");
                let line = artifact
                    .lines()
                    .find(|line| line.starts_with(&binding))
                    .ok_or_else(|| format!("{output}: missing associated bind {binding}"))?;
                let options = line.strip_prefix(&binding).ok_or("Quadlet bind options")?;
                assert_eq!(
                    options.split(',').any(|option| option == "ro"),
                    mode == "ro",
                    "{output}: wrong bind mode for {source}: {line}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn compose_values_imply_interpolation_and_obey_explicit_precedence() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("compose-interpolation")?;
    let input = scratch.path().join("compose.yaml");
    let first = scratch.path().join("first.env");
    let second = scratch.path().join("second.env");
    fs::write(
        &input,
        "name: interpolation\nservices:\n  web:\n    image: example.invalid/web:${TAG}\n",
    )?;
    fs::write(&first, "TAG=first\n")?;
    fs::write(&second, "TAG=second\n")?;

    for (label, explicit, expected) in [
        ("later-file", None, "second"),
        ("explicit-env", Some("TAG=chosen"), "chosen"),
    ] {
        let destination = scratch.path().join(label);
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&input)
            .arg("--env-file")
            .arg(&first)
            .arg("--env-file")
            .arg(&second);
        if let Some(value) = explicit {
            command.args(["--env", value]);
        }
        let result = command
            .arg("--output-directory")
            .arg(&destination)
            .args(["--console-format", "json"])
            .env("TAG", "ambient")
            .output()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "{label}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "success");
        let artifact = fs::read_to_string(destination.join("compose.yaml"))?;
        assert!(
            artifact.contains(&format!("image: example.invalid/web:{expected}\n")),
            "{label}: {artifact}"
        );
        assert!(
            !artifact.contains("ambient"),
            "{label}: process fallback overrode supplied values"
        );
    }
    Ok(())
}

#[test]
fn focused_help_and_error_paths_keep_canonical_defaults_and_fail_closed() -> Result<(), Box<dyn Error>> {
    let help = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["help", "validate", "podman", "compose"])
        .output()?;
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout)?;
    for (option, next_option, expected) in [
        (
            "--podman-import-policy <IMPORT_POLICY>",
            "--promote-podman-effective-bind-mounts",
            "[default: portable]",
        ),
        (
            "--loss-policy <LOSS_POLICY>",
            "--environment-values",
            "[default: exact]",
        ),
        (
            "--environment-values <ENVIRONMENT_VALUES>",
            "Diagnostics and reports:",
            "[default: withhold]",
        ),
    ] {
        let section = text
            .split_once(option)
            .and_then(|(_, after)| after.split_once(next_option))
            .map(|(section, _)| section)
            .ok_or_else(|| format!("missing help option {option}"))?;
        assert!(section.contains(expected), "{option}: missing {expected}");
        if option == "--loss-policy <LOSS_POLICY>" {
            assert!(section.contains("[possible values: exact, approximate, partial]"));
        }
    }
    assert!(text.contains("--podman-resource"));

    for input in FORMATS {
        for verb in ["convert", "validate"] {
            let help = Command::new(env!("CARGO_BIN_EXE_boxferry"))
                .args(["help", verb, input, "quadlet"])
                .output()?;
            assert!(help.status.success(), "{verb} {input} quadlet help");
            let text = String::from_utf8(help.stdout)?;
            let section = text
                .split_once("--podman-maximum-version")
                .and_then(|(_, after)| after.split_once("--quadlet-grouping"))
                .map(|(section, _)| section)
                .ok_or("missing Quadlet maximum-version help section")?;
            assert!(section.contains("[default: 6.1]"), "{verb} {input} quadlet: {section}");
        }
    }

    let scratch = Scratch::new("error-paths")?;
    let input = scratch.path().join("compose.yaml");
    fs::write(
        &input,
        "name: limits\nservices:\n  web:\n    image: example.invalid/web:1\n    restart: unless-stopped\n",
    )?;
    let exact = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "quadlet", "--input-file"])
        .arg(&input)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(exact.status.code(), Some(2));
    let exact_report: serde_json::Value = serde_json::from_slice(&exact.stdout)?;
    assert_eq!(exact_report["status"], "blocked");
    assert_diagnostic_subject(&exact_report, "BFQ0009", "services.web.restart_policy")?;
    let approximate = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "quadlet", "--input-file"])
        .arg(&input)
        .args(["--loss-policy", "approximate", "--console-format", "json"])
        .output()?;
    assert_eq!(approximate.status.code(), Some(0));
    let approximate_report: serde_json::Value = serde_json::from_slice(&approximate.stdout)?;
    assert_eq!(approximate_report["status"], "success");
    assert_diagnostic_subject(&approximate_report, "BFQ0009", "services.web.restart_policy")?;

    assert_unsupported_podman_target(&input)?;
    assert_unsupported_quadlet_ranges(&input)?;
    assert_ambiguous_quadlet_inputs(scratch.path())?;
    Ok(())
}

fn assert_unsupported_quadlet_ranges(input: &Path) -> Result<(), Box<dyn Error>> {
    for (minimum, maximum) in [("5.4", "6.1.3"), ("5.4", "6.2"), ("6.1.2", "6.1.0")] {
        let unsupported = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["validate", "compose", "quadlet", "--input-file"])
            .arg(input)
            .args([
                "--podman-minimum-version",
                minimum,
                "--podman-maximum-version",
                maximum,
                "--console-format",
                "json",
            ])
            .output()?;
        assert_eq!(unsupported.status.code(), Some(1), "{minimum} through {maximum}");
        assert!(unsupported.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&unsupported.stdout)?;
        assert_eq!(report["status"], "failure");
        assert_eq!(report["failed_stage"], "conversion");
        assert_eq!(report["primary_diagnostic_code"], "BFO1003");
        assert_eq!(report["output_artifacts"], serde_json::json!([]));
    }
    Ok(())
}

fn assert_unsupported_podman_target(input: &Path) -> Result<(), Box<dyn Error>> {
    let unsupported = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "podman", "--input-file"])
        .arg(input)
        .args([
            "--podman-target-context",
            "rootful",
            "--podman-max-version",
            "5.0",
            "--console-format",
            "json",
        ])
        .output()?;
    assert_eq!(unsupported.status.code(), Some(1));
    let unsupported_report: serde_json::Value = serde_json::from_slice(&unsupported.stdout)?;
    assert_eq!(unsupported_report["status"], "failure");
    assert_eq!(unsupported_report["failed_stage"], "conversion");
    assert_eq!(unsupported_report["primary_diagnostic_code"], "BFP0006");
    assert_eq!(unsupported_report["output_artifacts"], serde_json::json!([]));
    assert!(
        unsupported_report["diagnostics"][0]["fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| {
                field["name"] == "reason"
                    && field["value"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("below the oldest reviewed Podman target"))
            }))
    );
    Ok(())
}

fn assert_ambiguous_quadlet_inputs(scratch: &Path) -> Result<(), Box<dyn Error>> {
    let first_directory = scratch.join("first");
    let second_directory = scratch.join("second");
    fs::create_dir(&first_directory)?;
    fs::create_dir(&second_directory)?;
    for directory in [&first_directory, &second_directory] {
        fs::write(
            directory.join("web.container"),
            "[Container]\nImage=example.invalid/web:1\n",
        )?;
    }
    let ambiguous = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "quadlet", "compose", "--input-directory"])
        .arg(&first_directory)
        .arg("--input-directory")
        .arg(&second_directory)
        .args(["--application-name", "ambiguous", "--console-format", "json"])
        .output()?;
    assert_eq!(ambiguous.status.code(), Some(1));
    let ambiguous_report: serde_json::Value = serde_json::from_slice(&ambiguous.stdout)?;
    assert_eq!(ambiguous_report["status"], "failure");
    assert_eq!(ambiguous_report["failed_stage"], "input-discovery");
    assert_eq!(ambiguous_report["primary_diagnostic_code"], "BFO1000");
    assert_eq!(
        ambiguous_report["diagnostics"][0]["summary"],
        "duplicate Quadlet unit basename"
    );
    assert_eq!(ambiguous_report["output_artifacts"], serde_json::json!([]));
    Ok(())
}

#[test]
fn ordinary_podman_application_needs_no_selector_or_promotion_switch_for_each_exporter() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("podman-minimum-options")?;
    for output in FORMATS {
        for verb in ["validate", "convert"] {
            for format in ["json", "human"] {
                let server = PodmanCassetteServer::start(PodmanCassette::load(&podman_fixture())?)?;
                let destination = scratch.path().join(format!("{verb}-{output}-{format}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
                command.args([verb, "podman", output, "--podman-socket"]);
                command.arg(server.socket()).args(["--loss-policy", "partial"]);
                if output == "podman" {
                    command.args(["--podman-target-context", "rootful"]);
                }
                if verb == "convert" {
                    command.arg("--output-directory").arg(&destination);
                }
                if format == "json" {
                    command.args(["--console-format", "json"]);
                }
                let result = command.output()?;
                server.finish()?;
                assert_eq!(
                    result.status.code(),
                    Some(0),
                    "{verb} -> {output} {format}: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
                assert_no_private_canaries(&result.stdout, format);
                assert_no_private_canaries(&result.stderr, format);
                if format == "json" {
                    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
                    assert_eq!(report["status"], "success");
                    assert_eq!(report["application"], "portable-app");
                    assert!(report["choices"].as_array().is_some_and(|choices| {
                        choices
                            .iter()
                            .any(|choice| choice["name"] == "podman_import_policy" && choice["value"] == "portable")
                    }));
                    assert!(report["diagnostics"].as_array().is_some_and(|diagnostics| {
                        diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFP0009")
                    }));
                } else {
                    let stdout = String::from_utf8_lossy(&result.stdout);
                    assert!(stdout.contains("Podman input:"), "{verb} -> {output}: {stdout}");
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    assert!(stderr.contains("BFP0009"), "{verb} -> {output}: {stderr}");
                }
                if verb == "validate" {
                    assert!(!destination.exists());
                } else {
                    assert!(!artifact_names(&destination)?.is_empty());
                }
            }
        }
    }
    Ok(())
}

#[test]
fn conservative_podman_import_retains_effective_state_as_evidence() -> Result<(), Box<dyn Error>> {
    let server = PodmanCassetteServer::start(PodmanCassette::load(&podman_fixture())?)?;
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "podman", "compose", "--podman-socket"])
        .arg(server.socket())
        .args([
            "--podman-import-policy",
            "conservative",
            "--loss-policy",
            "partial",
            "--console-format",
            "json",
        ])
        .output()?;
    server.finish()?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert!(report["choices"].as_array().is_some_and(|choices| {
        choices
            .iter()
            .any(|choice| choice["name"] == "podman_import_policy" && choice["value"] == "conservative")
    }));
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| { diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFP0003") })
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| { diagnostics.iter().all(|diagnostic| diagnostic["code"] != "BFP0009") })
    );
    assert_no_private_canaries(&result.stdout, "conservative import");
    Ok(())
}

#[test]
fn invalid_input_and_existing_output_never_overwrite_user_files() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("no-clobber")?;
    let invalid = scratch.path().join("invalid.yaml");
    fs::write(&invalid, "services: [not-an-object]\n")?;
    let destination = scratch.path().join("destination");
    let invalid_result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(&invalid)
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(invalid_result.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&invalid_result.stdout)?;
    assert_eq!(report["status"], "failure");
    assert_eq!(report["output_artifacts"], serde_json::json!([]));
    assert_structured_diagnostics(&report, "invalid input")?;
    assert!(!destination.exists());

    fs::create_dir(&destination)?;
    let marker = destination.join("user-data.txt");
    fs::write(&marker, "keep this\n")?;
    let blocked = run_route("convert", "compose", "quadlet", &destination)?;
    assert_eq!(blocked.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&blocked.stdout)?;
    assert_eq!(report["status"], "failure");
    assert_eq!(report["primary_diagnostic_code"], "BFO2001");
    assert_eq!(
        artifact_names(&destination)?,
        BTreeSet::from(["user-data.txt".to_owned()])
    );
    assert_eq!(fs::read_to_string(marker)?, "keep this\n");
    Ok(())
}

#[test]
fn reviewed_core_fixture_checks_representative_structure_and_selected_losses() -> Result<(), Box<dyn Error>> {
    // Independent truth: fixtures/conversion/compose-to-quadlet-core/scenario.toml.
    // The authored scenario requires web's runtime name, PORT=9090, two published
    // ports, and a read-only named-volume mount. Its Compose source sets restart=no.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-core");
    let scratch = Scratch::new("core-semantics")?;

    for output in FORMATS {
        let destination = scratch.path().join(output);
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "compose", output, "--input-file"])
            .arg(fixture.join("compose.yaml"))
            .arg("--input-file")
            .arg(fixture.join("compose.override.yaml"))
            .args(["--loss-policy", "partial", "--output-directory"])
            .arg(&destination)
            .args(["--console-format", "json"]);
        if output != "podman" {
            command.args(["--environment-values", "include"]);
        }
        if output == "podman" {
            command.args(["--podman-target-context", "unknown"]);
        }
        let result = command.output()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "compose -> {output}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(result.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "success");
        assert_structured_diagnostics(&report, output)?;

        match output {
            "compose" => assert_core_compose(&destination, &report)?,
            "quadlet" => assert_core_quadlet(&destination, &report)?,
            "podman" => assert_core_podman(&destination, &report)?,
            _ => unreachable!("the route matrix contains only supported formats"),
        }
    }
    Ok(())
}

fn assert_core_compose(destination: &Path, report: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        artifact_names(destination)?,
        BTreeSet::from(["compose.yaml".to_owned()])
    );
    let yaml = fs::read_to_string(destination.join("compose.yaml"))?;
    assert!(yaml.contains("services:\n  web:\n    container_name: ferry-web\n"));
    assert!(!yaml.contains("  worker:\n"), "inactive profile unexpectedly appeared");
    assert!(yaml.contains("    environment:\n      - EMPTY=\n"));
    assert!(yaml.contains("      - PORT=9090\n"));
    assert!(yaml.contains("    restart: \"no\"\n"));
    assert!(yaml.contains(
        "    ports:\n      - target: 80\n        published: \"8080\"\n        host_ip: 127.0.0.1\n        protocol: tcp\n"
    ));
    assert!(yaml.contains(
        "    volumes:\n      - type: volume\n        source: data\n        target: /var/lib/data\n        read_only: true\n"
    ));
    assert_diagnostic_subject(report, "BFC0007", "services.web.healthcheck")
}

fn assert_core_quadlet(destination: &Path, report: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        artifact_names(destination)?,
        BTreeSet::from([
            "data.volume".to_owned(),
            "frontend.network".to_owned(),
            "web.container".to_owned(),
        ])
    );
    let unit = fs::read_to_string(destination.join("web.container"))?;
    assert!(unit.starts_with("[Container]\n"));
    for line in [
        "ContainerName=ferry-web",
        "Environment=EMPTY=",
        "Environment=PORT=9090",
        "PublishPort=127.0.0.1:8080:80/tcp",
        "PublishPort=8443:8443/tcp",
        "Volume=data.volume:/var/lib/data:ro",
    ] {
        assert!(unit.lines().any(|actual| actual == line), "missing {line}");
    }
    assert!(unit.contains("\n[Service]\nRestart=no\n"));
    assert_diagnostic_subject(report, "BFQ0003", "services.web.environment.FROM_HOST")
}

fn assert_core_podman(destination: &Path, report: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        artifact_names(destination)?,
        BTreeSet::from(["podman-commands.sh".to_owned(), "podman.json".to_owned()])
    );
    let plan: serde_json::Value = serde_json::from_slice(&fs::read(destination.join("podman.json"))?)?;
    let operations = plan["operations"].as_array().ok_or("Podman plan operations")?;
    let create = operations
        .iter()
        .find(|operation| operation["action"] == "create" && operation["resource"]["kind"] == "container")
        .ok_or("missing container create operation")?;
    assert_eq!(create["resource"]["name"], "web");
    let body = &create["libpod"]["body"]["json"];
    assert_eq!(body["command"], serde_json::json!(["php", "-v"]));
    assert_eq!(body["restart_policy"], "no");
    assert_eq!(body["volumes"][0]["Name"], "data");
    assert_eq!(body["volumes"][0]["Dest"], "/var/lib/data");
    assert_eq!(body["volumes"][0]["Options"], serde_json::json!(["ro", "copy"]));
    assert!(body.get("environment").is_none());
    assert!(body.get("ports").is_none());
    for subject in [
        "services.web.environment.PORT",
        "services.web.ports",
        "services.web.runtime_name",
    ] {
        assert_diagnostic_subject(report, "BFP0007", subject)?;
    }
    Ok(())
}

fn assert_diagnostic_subject(report: &serde_json::Value, code: &str, subject: &str) -> Result<(), Box<dyn Error>> {
    let diagnostics = report["diagnostics"].as_array().ok_or("diagnostics array")?;
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == code
                && diagnostic["fields"].as_array().is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| field["name"] == "subject" && field["value"] == subject)
                })
        }),
        "missing {code} diagnostic for {subject}"
    );
    Ok(())
}

fn diagnostic_codes_and_subjects(report: &serde_json::Value) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let diagnostics = report["diagnostics"].as_array().ok_or("missing diagnostics")?;
    diagnostics
        .iter()
        .map(|diagnostic| {
            let code = diagnostic["code"].as_str().ok_or("diagnostic has no code")?;
            let subject = diagnostic["fields"]
                .as_array()
                .and_then(|fields| fields.iter().find(|field| field["name"] == "subject"))
                .and_then(|field| field["value"].as_str())
                .unwrap_or("");
            Ok((code.to_owned(), subject.to_owned()))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()
}

fn run_route(verb: &str, input: &str, output: &str, destination: &Path) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command.args([verb, input, output]);

    let server = if input == "podman" {
        let cassette = PodmanCassette::load(&podman_fixture())?;
        let server = PodmanCassetteServer::start(cassette)?;
        command.arg("--podman-socket").arg(server.socket()).args([
            "--podman-resource",
            "container=portable-app",
            "--application-name",
            "podman-portable-intent",
        ]);
        Some(server)
    } else {
        let source = document_fixture().join(if input == "compose" {
            "compose.yaml"
        } else {
            "web.container"
        });
        command.arg("--input-file").arg(source);
        if input == "quadlet" {
            command.args(["--application-name", "route-matrix"]);
        }
        None
    };

    if input == "podman" {
        command.args(["--loss-policy", "partial"]);
    } else if output != "podman" {
        command.args(["--environment-values", "include"]);
    }
    if output == "podman" {
        command.args(["--podman-target-context", "unknown"]);
        if input != "podman" {
            command.args(["--loss-policy", "partial"]);
        }
    }
    if verb == "convert" {
        command.arg("--output-directory").arg(destination);
    }
    let result = command.args(["--console-format", "json"]).output()?;
    if let Some(server) = server {
        server.finish().map_err(|error| {
            format!(
                "{verb} {input} {output} cassette replay: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            )
        })?;
    }
    Ok(result)
}

fn assert_structured_diagnostics(report: &serde_json::Value, label: &str) -> Result<(), Box<dyn Error>> {
    let diagnostics = report["diagnostics"]
        .as_array()
        .ok_or_else(|| format!("{label}: missing diagnostics array"))?;
    for diagnostic in diagnostics {
        assert!(
            diagnostic["code"].as_str().is_some_and(|code| !code.is_empty()),
            "{label}"
        );
        assert!(diagnostic["severity"].as_str().is_some(), "{label}");
    }
    Ok(())
}

fn assert_expected_diagnostics(
    report: &serde_json::Value,
    input: &str,
    output: &str,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let codes = report["diagnostics"]
        .as_array()
        .ok_or("missing diagnostics")?
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .collect::<BTreeSet<_>>();
    if input == "podman" {
        assert!(
            codes.contains("BFP0002"),
            "{label}: missing known native-evidence warning"
        );
        for subject in ["services.portable-app.mounts[0]", "services.portable-app.mounts[1]"] {
            assert_diagnostic_subject(report, "BFP0003", subject)?;
            assert_diagnostic_field(
                report,
                "BFP0003",
                subject,
                "available_promotion",
                "--promote-podman-effective-bind-mounts",
            )?;
        }
        assert_diagnostic_subject(report, "BFP0009", "services.portable-app.networks")?;
        if output == "podman" {
            for subject in ["networks.scenario-net.internal", "networks.scenario-net.ipam_configs"] {
                assert_diagnostic_subject(report, "BFP0007", subject)?;
            }
        }
    } else if output == "podman" {
        assert!(codes.contains("BFP0007"), "{label}: missing target-context warning");
        assert_diagnostic_subject(report, "BFP0007", "services.web.runtime_name")?;
    } else {
        assert!(codes.is_empty(), "{label}: clean document route raised {codes:?}");
    }
    Ok(())
}

fn assert_diagnostic_field(
    report: &serde_json::Value,
    code: &str,
    subject: &str,
    name: &str,
    value: &str,
) -> Result<(), Box<dyn Error>> {
    let diagnostics = report["diagnostics"].as_array().ok_or("diagnostics array")?;
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == code
                && diagnostic["fields"].as_array().is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| field["name"] == "subject" && field["value"] == subject)
                        && fields
                            .iter()
                            .any(|field| field["name"] == name && field["value"] == value)
                })
        }),
        "missing {code} {subject} field {name}={value}"
    );
    Ok(())
}

fn assert_no_private_canaries(bytes: &[u8], label: &str) {
    let text = String::from_utf8_lossy(bytes);
    for canary in ["podman-portable-zeta-canary", "podman-portable-alpha-canary"] {
        assert!(!text.contains(canary), "{label}: private Podman value escaped");
    }
}

fn assert_artifact_intent(directory: &Path, input: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let image = if input == "podman" {
        "example.invalid/portable/app:1.0.0"
    } else {
        "example.invalid/web:1"
    };
    match output {
        "compose" => {
            let yaml = fs::read_to_string(directory.join("compose.yaml"))?;
            assert!(yaml.contains(image), "{input} -> compose lost reviewed image");
            assert!(yaml.contains(if input == "podman" { "portable-app:" } else { "web:" }));
            if input == "podman" {
                let (service, networks) = yaml
                    .split_once("\nnetworks:\n")
                    .ok_or("Podman Compose network section")?;
                assert!(
                    service.contains("scenario-net"),
                    "Podman -> Compose lost service network attachment"
                );
                assert!(
                    networks.contains("  scenario-net:"),
                    "Podman -> Compose lost named network"
                );
                assert_no_unpromoted_bind_sources(yaml.as_bytes(), "Podman -> Compose");
            } else {
                assert!(
                    yaml.contains("    container_name: web-runtime\n"),
                    "{input} -> Compose lost authored runtime name"
                );
                assert!(
                    yaml.contains("    restart: \"no\"\n"),
                    "{input} -> Compose lost authored restart policy"
                );
            }
        }
        "quadlet" => {
            let unit = if input == "podman" {
                "portable-app.container"
            } else {
                "web.container"
            };
            let text = fs::read_to_string(directory.join(unit))?;
            assert!(
                text.contains(&format!("Image={image}")),
                "{input} -> quadlet lost reviewed image"
            );
            if input == "podman" {
                assert!(text.lines().any(|line| line == "Network=scenario-net.network"));
                assert_no_unpromoted_bind_sources(text.as_bytes(), "Podman -> Quadlet");
                let network = fs::read_to_string(directory.join("scenario-net.network"))?;
                assert!(network.starts_with("[Network]\n"));
                assert!(network.lines().any(|line| line == "Internal=true"));
            } else {
                assert!(
                    text.lines().any(|line| line == "ContainerName=web-runtime"),
                    "{input} -> Quadlet lost authored runtime name"
                );
                assert!(
                    text.contains("\n[Service]\nRestart=no\n"),
                    "{input} -> Quadlet lost authored restart policy"
                );
            }
        }
        "podman" => {
            assert_podman_artifact(directory, input, image)?;
        }
        _ => return Err(format!("unsupported output format: {output}").into()),
    }
    Ok(())
}

fn assert_podman_artifact(directory: &Path, input: &str, image: &str) -> Result<(), Box<dyn Error>> {
    let plan: serde_json::Value = serde_json::from_slice(&fs::read(directory.join("podman.json"))?)?;
    assert_eq!(plan["schema_version"], 1);
    assert!(plan["connection"].is_null());
    assert!(
        plan["operations"]
            .as_array()
            .is_some_and(|operations| !operations.is_empty())
    );
    let operations = plan["operations"].as_array().ok_or("Podman operations")?;
    let create = operations
        .iter()
        .find(|operation| operation["action"] == "create" && operation["resource"]["kind"] == "container")
        .ok_or("Podman container create operation")?;
    assert_eq!(create["libpod"]["body"]["json"]["image"], image);
    if input == "podman" {
        assert_eq!(create["resource"]["name"], "portable-app");
        // A separately planned network does not prove that the
        // container joins it. The reviewed Libpod create body does.
        assert!(
            create["libpod"]["body"]["json"]["Networks"]
                .get("scenario-net")
                .is_some(),
            "Podman -> Podman lost the container's scenario-net attachment"
        );
        assert!(
            operations
                .iter()
                .any(|operation| operation["resource"]["kind"] == "network"
                    && operation["resource"]["name"] == "scenario-net")
        );
        assert_no_unpromoted_bind_sources(&fs::read(directory.join("podman.json"))?, "Podman -> Podman plan");
    } else {
        assert_eq!(
            create["resource"]["name"], "web",
            "authored runtime-name loss must be explicit"
        );
        assert_eq!(create["libpod"]["body"]["json"]["restart_policy"], "no");
    }
    let review = fs::read_to_string(directory.join("podman-commands.sh"))?;
    assert!(review.contains(image), "{input} -> podman lost reviewed image");
    if input == "podman" {
        assert_no_unpromoted_bind_sources(review.as_bytes(), "Podman -> Podman script");
    }
    Ok(())
}

fn assert_no_unpromoted_bind_sources(bytes: &[u8], label: &str) {
    let text = String::from_utf8_lossy(bytes);
    for source in ["/srv/boxferry-scenario/private", "/srv/boxferry-scenario/shared"] {
        assert!(
            !text.contains(source),
            "{label}: host-local source {source} crossed the default same-host boundary"
        );
    }
}

fn expected_artifacts(input: &str, output: &str) -> BTreeSet<String> {
    let names = match output {
        "compose" => BTreeSet::from(["compose.yaml"]),
        "quadlet" if input == "podman" => BTreeSet::from(["portable-app.container", "scenario-net.network"]),
        "quadlet" => BTreeSet::from(["web.container"]),
        "podman" => BTreeSet::from(["podman-commands.sh", "podman.json"]),
        _ => unreachable!("the route matrix contains only supported formats"),
    };
    names.into_iter().map(str::to_owned).collect()
}

fn artifact_names(directory: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let names = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    let names = names
        .iter()
        .map(|name| name.to_str().map(str::to_owned).ok_or("non-UTF-8 artifact name"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(names)
}

fn application(input: &str) -> &'static str {
    if input == "podman" {
        "podman-portable-intent"
    } else {
        "route-matrix"
    }
}

fn document_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/document-route-matrix")
}

fn podman_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/scenarios/podman-portable-intent/input-podman.cassette.json")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-cli-usability-{}-{label}-{id}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
