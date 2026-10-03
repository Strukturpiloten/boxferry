//! Independent CLI contracts for environment-value authorization and report isolation.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[path = "support/podman_cassette.rs"]
#[allow(dead_code)]
mod podman_cassette;

use std::{
    error::Error,
    fs,
    io::Read,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use podman_cassette::{PodmanCassette, PodmanCassetteServer};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DOCUMENT_SECRET: &str = "ordinary-looking-private-canary-334";
const PODMAN_SECRET: &str = "podman-portable-zeta-canary";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "boxferry-privacy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_report_without_value(output: &[u8], value: &str, name: &str) -> Result<(), Box<dyn Error>> {
    let text = String::from_utf8_lossy(output);
    assert!(!text.contains(value), "private value escaped in console report");
    let report: serde_json::Value = serde_json::from_slice(output)?;
    assert!(report["choices"].is_array());
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic["fields"].as_array().is_some_and(|fields| {
                    fields.iter().any(|field| {
                        field["name"] == "subject"
                            && field["value"]
                                .as_str()
                                .is_some_and(|subject| subject.ends_with(&format!(".environment.{name}")))
                    })
                })
            })),
        "missing named prerequisite {name}: {text}"
    );
    Ok(())
}

fn portable_privacy_cassette() -> Result<PodmanCassette, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/scenarios/podman-portable-intent/input-podman.cassette.json");
    let mut cassette: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let inspect = "/v6.1.0/libpod/containers/c-portable/json";
    let interactions = cassette["interactions"]
        .as_array_mut()
        .ok_or("cassette has no interactions")?;
    let response = interactions
        .iter_mut()
        .find(|interaction| interaction["request"]["path"] == inspect)
        .ok_or("portable container inspect is missing")?;
    let host = response
        .pointer_mut("/response/body/HostConfig")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("portable container has no HostConfig")?;
    host.insert(
        "PortBindings".to_owned(),
        serde_json::json!({ "8080/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "18080" }] }),
    );
    host.insert(
        "RestartPolicy".to_owned(),
        serde_json::json!({ "Name": "always", "MaximumRetryCount": 0 }),
    );
    let config = response
        .pointer_mut("/response/body/Config")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("portable container has no Config")?;
    config.insert(
        "Healthcheck".to_owned(),
        serde_json::json!({ "Test": ["CMD", "/bin/portable-health"], "Interval": 30_000_000_000_u64, "Timeout": 5_000_000_000_u64, "Retries": 3 }),
    );
    Ok(serde_json::from_value(cassette)?)
}

fn assert_portable_settings_survive_without_environment_value(artifact: &str) {
    assert!(
        artifact.contains("    restart: always\n"),
        "restart policy missing: {artifact}"
    );
    assert!(
        artifact.contains(
            "    ports:\n      - target: 8080\n        published: \"18080\"\n        host_ip: 127.0.0.1\n        protocol: tcp\n"
        ),
        "published TCP mapping changed or missing: {artifact}"
    );
}

fn write_supabase_privacy_sources(scratch: &Scratch) -> Result<(), Box<dyn Error>> {
    let image = format!("example.invalid/privacy-fixture@sha256:{}", "f".repeat(64));
    // A one-service authored policy fixture, not a synthesized live application.
    for mode in ["cli", "compose"] {
        for selection in ["exact", "storage", "label", "all"] {
            for input in ["compose", "quadlet"] {
                let directory = scratch.0.join(format!("outputs/{mode}-{selection}-{input}"));
                fs::create_dir_all(&directory)?;
                let (filename, text) = if input == "compose" {
                    (
                        "compose.yaml",
                        format!(
                            "---\nname: policy-supabase\nservices:\n  policy-supabase-db:\n    image: {image}\n    environment:\n      - POSTGRES_PASSWORD={DOCUMENT_SECRET}\n"
                        ),
                    )
                } else {
                    (
                        "policy-supabase-db.container",
                        format!("[Container]\nImage={image}\nEnvironment=POSTGRES_PASSWORD={DOCUMENT_SECRET}\n"),
                    )
                };
                fs::write(directory.join(filename), text)?;
            }
        }
    }
    Ok(())
}

#[test]
fn supabase_chained_helper_authorizes_document_canaries_without_relaxing_privacy() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    write_supabase_privacy_sources(&scratch)?;
    let output = Command::new("bash")
        .args([
            "-c",
            r#"
        set -Eeuo pipefail
        repository_root=$1
        source "$2"
        boxferry_bin=$3
        current_case=$4
        SUPABASE_DB_PASSWORD=$BOXFERRY_TEST_CANARY
        timed_operation() { shift 2; "$@"; }
        # The minimal fixture checks artifact policy, not the full application graph.
        assert_successful_conversion() { :; }
        supabase_assert_success_contract() { :; }
        supabase_assert_output_membership() { :; }
        supabase_assert_output_semantics() { :; }
        supabase_run_reimports cli policy
        supabase_run_reimports compose policy
        supabase_assert_report_privacy "$current_case"
    "#,
            "supabase-chained-privacy",
        ])
        .arg(&repository)
        .arg(repository.join("scripts/lib/supabase-application.sh"))
        .arg(env!("CARGO_BIN_EXE_boxferry"))
        .arg(&scratch.0)
        .env("BOXFERRY_TEST_CANARY", DOCUMENT_SECRET)
        .output()?;
    assert!(!String::from_utf8_lossy(&output.stdout).contains(DOCUMENT_SECRET));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(DOCUMENT_SECRET));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut reports = 0;
    for entry in fs::read_dir(scratch.0.join("reimports"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "json") {
            reports += 1;
            let bytes = fs::read(&path)?;
            assert!(!String::from_utf8_lossy(&bytes).contains(DOCUMENT_SECRET));
            let report: serde_json::Value = serde_json::from_slice(&bytes)?;
            let withheld = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("-to-podman.report.json"));
            assert_eq!(report["status"], "success");
            assert_eq!(
                report["fidelity"]["unsupported"],
                usize::from(withheld),
                "{}",
                path.display()
            );
            assert_eq!(
                report["diagnostics"].as_array().ok_or("diagnostics")?.len(),
                usize::from(withheld)
            );
        }
    }
    assert_eq!(
        reports, 48,
        "all provisioner/selector/document-input/output combinations"
    );
    // A source artifact carrying values still grants no implicit authorization.
    let default = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args([
            "convert",
            "compose",
            "compose",
            "--loss-policy",
            "partial",
            "--input-file",
        ])
        .arg(scratch.0.join("outputs/cli-exact-compose/compose.yaml"))
        .arg("--output-directory")
        .arg(scratch.0.join("default-withheld"))
        .args(["--console-format", "json"])
        .output()?;
    assert!(default.status.success());
    assert_report_without_value(&default.stdout, DOCUMENT_SECRET, "POSTGRES_PASSWORD")?;
    assert!(!fs::read_to_string(scratch.0.join("default-withheld/compose.yaml"))?.contains(DOCUMENT_SECRET));
    Ok(())
}

#[test]
fn paperless_withheld_broker_assertion_matches_real_compose_omission_and_quadlet_health() -> Result<(), Box<dyn Error>>
{
    const BROKER_CANARY: &str = "boxferry-public-broker-canary";
    let scratch = Scratch::new()?;
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = scratch.0.join("broker.yaml");
    // Independently authored three-token command and two-token shell healthcheck,
    // matching the live broker's relevant shape without replaying a runtime.
    fs::write(
        &source,
        format!(
            "---\nname: fixture-paperless\nservices:\n  fixture-paper-broker:\n    image: example.invalid/valkey@sha256:{}\n    command: [valkey-server, --requirepass, {BROKER_CANARY}]\n    healthcheck:\n      test: [CMD-SHELL, \"valkey-cli -a {BROKER_CANARY} ping\"]\n    environment:\n      MODE: {DOCUMENT_SECRET}\n",
            "f".repeat(64),
        ),
    )?;
    for target in ["compose", "quadlet"] {
        let destination = scratch.0.join(target);
        let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "compose", target, "--input-file"])
            .arg(&source)
            .args(["--loss-policy", "partial", "--output-directory"])
            .arg(&destination)
            .args(["--console-format", "json"])
            .output()?;
        assert!(output.status.success());
        assert_report_without_value(&output.stdout, DOCUMENT_SECRET, "MODE")?;
        for bytes in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(bytes).contains(BROKER_CANARY));
        }
        let report = scratch.0.join(format!("{target}.report.json"));
        fs::write(&report, &output.stdout)?;
        let result = Command::new("bash")
            .args([
                "-c",
                "source \"$1\"; paperless_assert_withheld_broker_command \"$2\" \"$3\" \"$4\" fixture",
                "paperless-broker-privacy",
            ])
            .arg(repository.join("scripts/lib/paperless-application.sh"))
            .arg(&destination)
            .arg(&report)
            .arg(target)
            .output()?;
        assert!(!String::from_utf8_lossy(&result.stdout).contains(BROKER_CANARY));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(BROKER_CANARY));
        assert!(
            result.status.success(),
            "{target}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let artifact = if target == "compose" {
            "compose.yaml"
        } else {
            "fixture-paper-broker.container"
        };
        let artifact = fs::read_to_string(destination.join(artifact))?;
        assert!(!artifact.contains(DOCUMENT_SECRET));
        assert_eq!(
            artifact.matches(BROKER_CANARY).count(),
            if target == "compose" { 1 } else { 2 }
        );
    }
    Ok(())
}

fn paperless_broker_privacy_cassette(
    captured: &serde_json::Value,
    health_kind: &str,
    canary: &str,
) -> Result<PodmanCassette, Box<dyn Error>> {
    // Derive authored privacy cases from the admitted redacted transport
    // fixture; do not alter its provenance or claim new native evidence.
    let mut fixture = captured.clone();
    let interactions = fixture["interactions"].as_array_mut().ok_or("interactions")?;
    assert_eq!(interactions.len(), 27);
    let broker = interactions
        .iter_mut()
        .find(|interaction| interaction["response"]["body"]["Name"] == "paperless-captured-paper-broker")
        .ok_or("captured broker inspect")?;
    assert_eq!(
        broker["response"]["body"]["Config"]["Cmd"],
        serde_json::json!(["valkey-server", "--requirepass", "<redacted:PAPERLESS_REDIS_PASSWORD>"])
    );
    assert_eq!(
        broker["response"]["body"]["Config"]["Healthcheck"]["Test"],
        serde_json::json!(["CMD-SHELL", "valkey-cli -a <redacted:PAPERLESS_REDIS_PASSWORD> ping"])
    );
    broker["response"]["body"]["Config"]["Cmd"] = serde_json::json!(["valkey-server", "--requirepass", canary]);
    broker["response"]["body"]["Config"]["Healthcheck"]["Test"] = if health_kind == "CMD-SHELL" {
        serde_json::json!(["CMD-SHELL", format!("valkey-cli -a {canary} ping")])
    } else {
        serde_json::json!(["CMD", "valkey-cli", "-a", canary, "ping"])
    };
    Ok(serde_json::from_value(fixture)?)
}

#[test]
fn paperless_podman_health_findings_coexist_with_export_omission_evidence() -> Result<(), Box<dyn Error>> {
    const BROKER_CANARY: &str = "boxferry-public-broker-canary";
    const HEALTH_SUBJECT: &str = "services.paperless-captured-paper-broker.healthcheck";
    let scratch = Scratch::new()?;
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let captured: serde_json::Value = serde_json::from_slice(&fs::read(
        repository.join("fixtures/conformance/paperless-ngx-application/paperless-ngx-6.1.0-rootless.cassette.json"),
    )?)?;
    for (health_kind, source_code) in [("CMD-SHELL", "BFP0003"), ("CMD", "BFP0009")] {
        for target in ["compose", "quadlet"] {
            let cassette = paperless_broker_privacy_cassette(&captured, health_kind, BROKER_CANARY)?;
            let server = PodmanCassetteServer::start_unordered(cassette)?;
            let destination = scratch.0.join(format!("{health_kind}-{target}"));
            let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
                .args(["convert", "podman", target, "--podman-socket"])
                .arg(server.socket())
                .args([
                    "--application-name",
                    "paperless-captured-paperless",
                    "--podman-label",
                    "io.boxferry.application=paperless-captured-paperless",
                    "--promote-podman-effective-named-volumes",
                    "--promote-podman-effective-named-networks",
                    "--promote-podman-portable-effective-settings",
                    "--loss-policy",
                    "partial",
                    "--console-format",
                    "json",
                ])
                .arg("--output-directory")
                .arg(&destination)
                .output()?;
            // Complete consumption proves all 27 production acquisition requests.
            server.finish()?;
            assert!(output.status.success());
            assert_report_without_value(&output.stdout, BROKER_CANARY, "PAPERLESS_DBPASS")?;
            assert!(!String::from_utf8_lossy(&output.stderr).contains(BROKER_CANARY));
            let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let findings = report["diagnostics"]
                .as_array()
                .ok_or("diagnostics")?
                .iter()
                .filter(|finding| {
                    finding["fields"].as_array().is_some_and(|fields| {
                        fields
                            .iter()
                            .any(|field| field["name"] == "subject" && field["value"] == HEALTH_SUBJECT)
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(
                findings.iter().filter(|finding| finding["code"] == source_code).count(),
                1,
                "{health_kind} source finding must remain visible for {target}"
            );
            let omissions = findings
                .iter()
                .filter(|finding| finding["code"] == "BFC0007")
                .collect::<Vec<_>>();
            assert_eq!(omissions.len(), usize::from(target == "compose"));
            for omission in omissions {
                assert_eq!(omission["severity"], "warning");
                assert!(
                    omission["fields"]
                        .as_array()
                        .ok_or("omission fields")?
                        .iter()
                        .any(|field| {
                            field["name"] == "reason"
                                && field["value"]
                                    == "the current Compose generation boundary does not yet expose health-check fields"
                        })
                );
            }
            let report_path = scratch.0.join(format!("{health_kind}-{target}.report.json"));
            fs::write(&report_path, &output.stdout)?;
            let result = Command::new("bash")
                .args([
                    "-c",
                    "source \"$1\"; paperless_assert_withheld_broker_command \"$2\" \"$3\" \"$4\" paperless-captured",
                    "paperless-podman-broker-privacy",
                ])
                .arg(repository.join("scripts/lib/paperless-application.sh"))
                .arg(&destination)
                .arg(&report_path)
                .arg(target)
                .output()?;
            assert!(!String::from_utf8_lossy(&result.stdout).contains(BROKER_CANARY));
            assert!(!String::from_utf8_lossy(&result.stderr).contains(BROKER_CANARY));
            assert!(
                result.status.success(),
                "{health_kind} to {target}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                fs::read(&report_path)?,
                output.stdout,
                "source findings must remain untouched"
            );
        }
    }
    Ok(())
}

#[test]
fn authored_values_are_withheld_across_exporters_and_validate_the_same_way() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let compose = scratch.0.join("compose.yaml");
    let quadlet = scratch.0.join("web.container");
    fs::write(
        &compose,
        format!(
            "services:\n  web:\n    image: example.invalid/web:1\n    environment:\n      MODE: {DOCUMENT_SECRET}\n"
        ),
    )?;
    fs::write(
        &quadlet,
        format!("[Container]\nImage=example.invalid/web:1\nEnvironment=MODE={DOCUMENT_SECRET}\n"),
    )?;

    for (input, source) in [("compose", &compose), ("quadlet", &quadlet)] {
        for target in ["compose", "quadlet", "podman"] {
            for verb in ["validate", "convert"] {
                let destination = scratch.0.join(format!("{verb}-{input}-{target}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
                command.args([verb, input, target, "--input-file"]).arg(source).args([
                    "--loss-policy",
                    "partial",
                    "--console-format",
                    "json",
                ]);
                if input == "quadlet" {
                    command.args(["--application-name", "privacy"]);
                }
                if target == "podman" {
                    command.args(["--podman-target-context", "unknown"]);
                }
                if verb == "convert" {
                    command.arg("--output-directory").arg(&destination);
                }
                let output = command.output()?;
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{verb} {input}->{target}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                assert!(!String::from_utf8_lossy(&output.stderr).contains(DOCUMENT_SECRET));
                assert_report_without_value(&output.stdout, DOCUMENT_SECRET, "MODE")?;
                if verb == "validate" {
                    assert!(!destination.exists());
                } else {
                    for entry in fs::read_dir(&destination)? {
                        let artifact = fs::read_to_string(entry?.path())?;
                        assert!(!artifact.contains(DOCUMENT_SECRET));
                        assert!(
                            !artifact.contains("MODE"),
                            "withheld assignment must not become a host reference: {artifact}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn exact_policy_blocks_withheld_value_before_writing() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.0.join("compose.yaml");
    let destination = scratch.0.join("blocked");
    fs::write(
        &source,
        format!(
            "services:\n  web:\n    image: example.invalid/web:1\n    environment:\n      MODE: {DOCUMENT_SECRET}\n"
        ),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "compose", "compose", "--input-file"])
        .arg(&source)
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert_report_without_value(&output.stdout, DOCUMENT_SECRET, "MODE")?;
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["status"], "blocked");
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn explicit_inclusion_is_artifact_only_and_podman_output_fails_closed() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let compose = scratch.0.join("compose.yaml");
    fs::write(
        &compose,
        format!(
            "services:\n  web:\n    image: example.invalid/web:1\n    environment:\n      MODE: {DOCUMENT_SECRET}\n"
        ),
    )?;
    for target in ["compose", "quadlet", "podman"] {
        let destination = scratch.0.join(format!("include-{target}"));
        let report_file = scratch.0.join(format!("report-{target}.json"));
        let archive_dir = scratch.0.join(format!("archive-{target}"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "compose", target, "--input-file"])
            .arg(&compose)
            .args([
                "--environment-values",
                "include",
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ])
            .arg("--output-directory")
            .arg(&destination)
            .arg("--report-file")
            .arg(&report_file)
            .arg("--generate-error-report")
            .arg("--error-report-directory")
            .arg(&archive_dir);
        if target == "podman" {
            command.args(["--podman-target-context", "unknown"]);
        }
        let output = command.output()?;
        let expected_code = i32::from(target == "podman");
        assert_eq!(
            output.status.code(),
            Some(expected_code),
            "{target}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let report_bytes = fs::read(&report_file)?;
        for bytes in [&output.stdout[..], &output.stderr[..], &report_bytes[..]] {
            assert!(
                !bytes
                    .windows(DOCUMENT_SECRET.len())
                    .any(|window| window == DOCUMENT_SECRET.as_bytes())
            );
        }
        if target == "podman" {
            assert!(!destination.exists());
        } else {
            let texts = fs::read_dir(&destination)?
                .map(|entry| fs::read_to_string(entry?.path()))
                .collect::<Result<Vec<_>, _>>()?;
            assert!(texts.iter().any(|text| text.contains(DOCUMENT_SECRET)));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(&destination)?.permissions().mode() & 0o077, 0);
                for entry in fs::read_dir(&destination)? {
                    assert_eq!(entry?.metadata()?.permissions().mode() & 0o077, 0);
                }
            }
        }
        let entries = fs::read_dir(&archive_dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(entries.len(), 1);
        let mut zip = zip::ZipArchive::new(fs::File::open(&entries[0])?)?;
        for index in 0..zip.len() {
            let mut text = String::new();
            zip.by_index(index)?.read_to_string(&mut text)?;
            assert!(!text.contains(DOCUMENT_SECRET));
        }
    }
    Ok(())
}

#[test]
fn human_console_stays_value_free_when_inclusion_is_authorized() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.0.join("compose.yaml");
    fs::write(
        &source,
        format!(
            "services:\n  web:\n    image: example.invalid/web:1\n    environment:\n      MODE: {DOCUMENT_SECRET}\n"
        ),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "quadlet", "--input-file"])
        .arg(&source)
        .args(["--environment-values", "include", "--loss-policy", "partial"])
        .output()?;
    assert_eq!(output.status.code(), Some(0));
    assert!(
        !output
            .stdout
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    assert!(
        !output
            .stderr
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    Ok(())
}

#[test]
fn authored_quadlet_value_requires_separate_artifact_inclusion() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.0.join("web.container");
    let destination = scratch.0.join("quadlet-included");
    fs::write(
        &source,
        format!("[Container]\nImage=example.invalid/web:1\nEnvironment=MODE={DOCUMENT_SECRET}\n"),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "quadlet", "compose", "--input-file"])
        .arg(&source)
        .args([
            "--application-name",
            "privacy",
            "--environment-values",
            "include",
            "--loss-policy",
            "partial",
            "--console-format",
            "json",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !output
            .stdout
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    assert!(fs::read_to_string(destination.join("compose.yaml"))?.contains(DOCUMENT_SECRET));
    Ok(())
}

#[test]
fn podman_portability_does_not_authorize_environment_acquisition() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let cassette = portable_privacy_cassette()?;
    let server = PodmanCassetteServer::start(cassette)?;
    let destination = scratch.0.join("podman-default");
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "podman", "compose", "--podman-socket"])
        .arg(server.socket())
        .args([
            "--podman-resource",
            "container=portable-app",
            "--promote-podman-portable-effective-settings",
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
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_report_without_value(&output.stdout, PODMAN_SECRET, "ZETA")?;
    let artifact = fs::read_to_string(destination.join("compose.yaml"))?;
    assert!(!artifact.contains(PODMAN_SECRET));
    assert!(
        !artifact.contains("ZETA"),
        "withheld Podman assignment became a host reference"
    );
    assert_portable_settings_survive_without_environment_value(&artifact);
    Ok(())
}

#[test]
fn podman_health_portability_does_not_authorize_environment_acquisition() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let server = PodmanCassetteServer::start(portable_privacy_cassette()?)?;
    let destination = scratch.0.join("podman-quadlet-default");
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "podman", "quadlet", "--podman-socket"])
        .arg(server.socket())
        .args([
            "--podman-resource",
            "container=portable-app",
            "--promote-podman-portable-effective-settings",
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
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_report_without_value(&output.stdout, PODMAN_SECRET, "ZETA")?;
    let artifacts = fs::read_dir(&destination)?
        .map(|entry| fs::read_to_string(entry?.path()))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        artifacts
            .iter()
            .any(|artifact| artifact.contains("/bin/portable-health"))
    );
    assert!(artifacts.iter().all(|artifact| !artifact.contains("ZETA")));
    Ok(())
}

#[test]
fn authorized_podman_values_reach_only_compose_or_quadlet_artifacts() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    for target in ["compose", "quadlet"] {
        let cassette = portable_privacy_cassette()?;
        let server = PodmanCassetteServer::start(cassette)?;
        let destination = scratch.0.join(format!("podman-{target}"));
        let report_file = scratch.0.join(format!("podman-{target}-report.json"));
        let archive_dir = scratch.0.join(format!("podman-{target}-archive"));
        let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "podman", target, "--podman-socket"])
            .arg(server.socket())
            .args([
                "--podman-resource",
                "container=portable-app",
                "--promote-podman-portable-effective-settings",
                "--environment-values",
                "include",
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ])
            .arg("--output-directory")
            .arg(&destination)
            .arg("--report-file")
            .arg(&report_file)
            .arg("--generate-error-report")
            .arg("--include-podman-snapshot")
            .arg("--error-report-directory")
            .arg(&archive_dir)
            .output()?;
        server.finish()?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{target}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !output
                .stdout
                .windows(PODMAN_SECRET.len())
                .any(|window| window == PODMAN_SECRET.as_bytes())
        );
        assert!(
            !fs::read(&report_file)?
                .windows(PODMAN_SECRET.len())
                .any(|window| window == PODMAN_SECRET.as_bytes())
        );
        let artifacts = fs::read_dir(&destination)?
            .map(|entry| fs::read_to_string(entry?.path()))
            .collect::<Result<Vec<_>, _>>()?;
        assert!(artifacts.iter().any(|artifact| artifact.contains(PODMAN_SECRET)));
        if target == "quadlet" {
            assert!(
                artifacts
                    .iter()
                    .any(|artifact| artifact.contains("/bin/portable-health"))
            );
        }
        let archives = fs::read_dir(&archive_dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(archives.len(), 1);
        let mut zip = zip::ZipArchive::new(fs::File::open(&archives[0])?)?;
        for index in 0..zip.len() {
            let mut text = String::new();
            zip.by_index(index)?.read_to_string(&mut text)?;
            assert!(
                !text.contains(PODMAN_SECRET),
                "Podman support snapshot leaked environment value"
            );
        }
    }
    Ok(())
}

#[test]
fn selected_interpolation_sources_do_not_authorize_output_values() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let compose = scratch.0.join("compose.yaml");
    let env_file = scratch.0.join("selected.env");
    fs::write(
        &compose,
        "services:\n  web:\n    image: example.invalid/web:1\n    environment:\n      MODE: ${MODE}\n",
    )?;
    fs::write(&env_file, format!("MODE={DOCUMENT_SECRET}\n"))?;

    for source in ["literal", "process", "file"] {
        let destination = scratch.0.join(format!("interpolated-{source}"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&compose)
            .args(["--interpolate", "--loss-policy", "partial", "--console-format", "json"])
            .arg("--output-directory")
            .arg(&destination)
            .env("UNSELECTED_CANARY", "ambient-secret-must-not-be-read");
        match source {
            "literal" => {
                command.arg("--env").arg(format!("MODE={DOCUMENT_SECRET}"));
            }
            "process" => {
                command.arg("--env").arg("MODE").env("MODE", DOCUMENT_SECRET);
            }
            "file" => {
                command.arg("--env-file").arg(&env_file);
            }
            _ => unreachable!(),
        }
        let output = command.output()?;
        assert_eq!(
            output.status.code(),
            Some(0),
            "{source}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_report_without_value(&output.stdout, DOCUMENT_SECRET, "MODE")?;
        let artifact = fs::read_to_string(destination.join("compose.yaml"))?;
        assert!(!artifact.contains(DOCUMENT_SECRET));
        assert!(!artifact.contains("ambient-secret-must-not-be-read"));
    }
    let ambient_only = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "compose", "--input-file"])
        .arg(&compose)
        .args(["--interpolate", "--loss-policy", "partial", "--console-format", "json"])
        .env("MODE", DOCUMENT_SECRET)
        .output()?;
    assert!(
        !ambient_only
            .stdout
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    assert!(
        !ambient_only
            .stderr
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    Ok(())
}

#[test]
fn invalid_environment_value_policy_is_rejected_before_output() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let destination = scratch.0.join("invalid-policy");
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args([
            "convert",
            "compose",
            "quadlet",
            "--environment-values",
            "everything",
            "--output-directory",
        ])
        .arg(&destination)
        .output()?;
    assert!(!output.status.success());
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn invalid_document_cannot_echo_a_neighboring_environment_value() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let source = scratch.0.join("invalid.yaml");
    let destination = scratch.0.join("invalid-source-output");
    fs::write(
        &source,
        format!("services:\n  web:\n    image: [invalid-shape]\n    environment:\n      MODE: {DOCUMENT_SECRET}\n"),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(&source)
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert!(!output.status.success());
    assert!(
        !output
            .stdout
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    assert!(
        !output
            .stderr
            .windows(DOCUMENT_SECRET.len())
            .any(|window| window == DOCUMENT_SECRET.as_bytes())
    );
    assert!(!destination.exists());
    Ok(())
}
