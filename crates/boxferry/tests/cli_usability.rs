//! Fast, offline user-journey contracts for the public CLI.
//!
//! These expectations are authored here, independently of the CLI implementation.
//! A Podman cassette serves read-only API responses; no runtime is started.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[path = "support/podman_cassette.rs"]
#[allow(dead_code)] // Shared cassette helper has mutation methods used by other integration tests.
mod podman_cassette;

use std::{
    collections::BTreeSet,
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
fn all_eighteen_cli_route_forms_preserve_a_reviewed_user_intent() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("route-forms")?;
    let mut forms = BTreeSet::new();

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
                assert!(report["failed_stage"].is_null(), "{label}");
                assert_structured_diagnostics(&report, &label)?;
                assert_expected_diagnostics(&report, input, output, &label)?;
                if input == "podman" {
                    assert_no_private_canaries(&result.stdout, &label);
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
    } else if output == "podman" {
        assert!(codes.contains("BFP0007"), "{label}: missing target-context warning");
    } else {
        assert!(codes.is_empty(), "{label}: clean document route raised {codes:?}");
    }
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
        }
        "podman" => {
            let plan: serde_json::Value = serde_json::from_slice(&fs::read(directory.join("podman.json"))?)?;
            assert_eq!(plan["schema_version"], 1);
            assert!(plan["connection"].is_null());
            assert!(
                plan["operations"]
                    .as_array()
                    .is_some_and(|operations| !operations.is_empty())
            );
            let review = fs::read_to_string(directory.join("podman-commands.sh"))?;
            assert!(review.contains(image), "{input} -> podman lost reviewed image");
        }
        _ => return Err(format!("unsupported output format: {output}").into()),
    }
    Ok(())
}

fn expected_artifacts(input: &str, output: &str) -> BTreeSet<String> {
    let names = match output {
        "compose" => BTreeSet::from(["compose.yaml"]),
        "quadlet" if input == "podman" => BTreeSet::from(["portable-app.container"]),
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
