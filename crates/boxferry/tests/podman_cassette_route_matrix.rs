//! Production-CLI coverage for every reviewed complex Podman cassette and exporter.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[path = "support/podman_cassette.rs"]
mod podman_cassette;

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use podman_cassette::{PodmanCassette, PodmanCassetteServer};
use serde::Deserialize;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const VERSIONS: [&str; 7] = ["5.4.0", "5.5.0", "5.6.0", "5.7.0", "5.8.6", "6.0.0", "6.1.0"];
const CONTEXTS: [&str; 2] = ["rootful", "rootless"];
const OUTPUTS: [&str; 3] = ["compose", "quadlet", "podman"];

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    files: BTreeSet<String>,
    extensions: CorpusExtensions,
}

#[derive(Debug, Deserialize)]
struct CorpusExtensions {
    #[serde(rename = "podman-corpus")]
    podman_corpus: PodmanCorpus,
}

#[derive(Debug, Deserialize)]
struct PodmanCorpus {
    artifacts: Vec<HashedArtifact>,
}

#[derive(Debug, Deserialize)]
struct HashedArtifact {
    file: String,
    sha256: String,
}

#[test]
fn cassette_manifest_hashes_pin_every_reviewed_artifact() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let manifest: CorpusManifest = toml::from_str(&fs::read_to_string(fixture.join("fixture.toml"))?)?;
    let artifacts = manifest
        .extensions
        .podman_corpus
        .artifacts
        .into_iter()
        .map(|artifact| (artifact.file, artifact.sha256))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(artifacts.len(), VERSIONS.len() * CONTEXTS.len());
    assert_eq!(artifacts.keys().cloned().collect::<BTreeSet<_>>(), manifest.files);
    for (file, expected) in artifacts {
        assert_eq!(
            sha256(&fixture.join(&file))?,
            expected,
            "cassette hash drifted for {file}"
        );
    }
    Ok(())
}

#[test]
fn every_complex_cassette_replays_through_every_exporter() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let root = TemporaryDirectory::new("podman-cassette-routes")?;
    let mut scenarios = BTreeSet::new();

    for version in VERSIONS {
        for context in CONTEXTS {
            let name = format!("complex-{version}-{context}");
            let cassette_path = fixture.join(format!("{name}.cassette.json"));
            let cassette = PodmanCassette::load(&cassette_path)?;
            assert_eq!(cassette.scenario_id(), name);
            assert_eq!(cassette.engine_version(), version);
            assert_eq!(cassette.execution_context(), context);
            assert_eq!(cassette.interaction_count(), 30);
            assert!(scenarios.insert(name.clone()));

            for output in OUTPUTS {
                let directory = root.path().join(format!("{name}-{output}"));
                let result = run_route(&cassette_path, output, context, &directory)?;
                assert_route_succeeded(&result, &name, output)?;
                assert_report(&result.stdout, output, &directory)?;
                match output {
                    "compose" => assert_compose_output(&directory)?,
                    "quadlet" => assert_quadlet_output(&directory)?,
                    "podman" => assert_podman_output(&directory)?,
                    _ => return Err(format!("unhandled output {output}").into()),
                }
            }

            assert_document_reimports(&name, context, root.path())?;

            let first = root.path().join(format!("{name}-podman"));
            let repeated = root.path().join(format!("{name}-podman-repeat"));
            let result = run_route(&cassette_path, "podman", context, &repeated)?;
            assert_route_succeeded(&result, &name, "podman repeat")?;
            assert_eq!(
                fs::read(first.join("podman.json"))?,
                fs::read(repeated.join("podman.json"))?
            );
            assert_eq!(
                fs::read(first.join("podman-commands.sh"))?,
                fs::read(repeated.join("podman-commands.sh"))?
            );
        }
    }

    assert_eq!(scenarios.len(), VERSIONS.len() * CONTEXTS.len());
    Ok(())
}

#[test]
fn opted_in_support_bundle_contains_only_redacted_podman_snapshots() -> Result<(), Box<dyn Error>> {
    let cassette_path = fixture_directory().join("complex-6.1.0-rootless.cassette.json");
    let cassette = PodmanCassette::load(&cassette_path)?;
    let server = PodmanCassetteServer::start(cassette)?;
    let socket = server.socket().to_owned();
    let root = TemporaryDirectory::new("podman-support-bundle")?;
    let reports = root.path().join("reports");
    fs::create_dir(&reports)?;

    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "podman", "compose", "--podman-socket"])
        .arg(&socket)
        .args([
            "--podman-all",
            "--application-name",
            "complex",
            "--promote-podman-effective-named-volumes",
            "--promote-podman-effective-named-networks",
            "--loss-policy",
            "partial",
            "--console-format",
            "json",
            "--generate-error-report",
            "--include-podman-snapshot",
            "--error-report-directory",
        ])
        .arg(&reports)
        .output()?;
    let replay = server.finish();
    if let Err(error) = replay {
        return Err(format!(
            "support-bundle cassette replay failed: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert!(report["redaction"]["count"].as_u64().is_some_and(|count| count > 0));
    assert!(
        report["redaction"]["classes"]
            .as_array()
            .is_some_and(|classes| classes.iter().any(|class| class == "podman-support-snapshot"))
    );

    let archives = fs::read_dir(&reports)?.collect::<Result<Vec<_>, _>>()?;
    let [archive] = archives.as_slice() else {
        return Err(format!("expected one support archive, found {}", archives.len()).into());
    };
    let mut zip = zip::ZipArchive::new(fs::File::open(archive.path())?)?;
    let mut entries = BTreeMap::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        entries.insert(entry.name().to_owned(), text);
    }
    assert_eq!(
        entries.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "README.md",
            "podman-acquisition-findings-v1.json",
            "podman-discovery-graph-v1.json",
            "podman-inventory-v1.json",
            "report.json",
        ])
    );
    for (name, contents) in &entries {
        for protected in [
            "COMPLEX_PROTECTED_VALUE_NEVER_PRINT",
            "COMPLEX_DB_PROTECTED_VALUE",
            "COMPLEX_NORMAL_HEALTH_SENTINEL",
            "COMPLEX_STARTUP_HEALTH_SENTINEL",
        ] {
            assert!(!contents.contains(protected), "{name} retained protected Podman value");
        }
        assert!(!contents.contains(".sock"), "{name} retained the connection endpoint");
    }
    assert!(entries["README.md"].contains("operationally sensitive"));
    let inventory: serde_json::Value = serde_json::from_str(&entries["podman-inventory-v1.json"])?;
    assert_eq!(inventory["schema_version"], 1);
    let graph: serde_json::Value = serde_json::from_str(&entries["podman-discovery-graph-v1.json"])?;
    assert_eq!(graph["schema_version"], 1);
    let findings: serde_json::Value = serde_json::from_str(&entries["podman-acquisition-findings-v1.json"])?;
    assert_eq!(findings["schema_version"], 1);
    Ok(())
}

#[test]
fn exact_prefix_and_label_selectors_infer_names_through_every_exporter() -> Result<(), Box<dyn Error>> {
    let source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    let root = TemporaryDirectory::new("podman-cassette-selector-inference")?;

    for (case, arguments, expected_application) in [
        ("exact", ["--podman-resource", "container=observer"], "observer"),
        ("prefix", ["--podman-resource-prefix", "container=obs"], "obs"),
        (
            "label",
            ["--podman-label", "org.example.complex.service=observer"],
            "observer",
        ),
    ] {
        for output in OUTPUTS {
            let directory = root.path().join(format!("{case}-{output}"));
            let result = run_cassette_with_application(
                source.clone(),
                output,
                "rootless",
                &directory,
                &arguments,
                "partial",
                None,
            )?;
            assert_route_succeeded(&result, case, output)?;
            let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
            assert_eq!(report["application"], expected_application);
            assert_only_observer(output, &directory)?;
        }
    }
    Ok(())
}

fn assert_only_observer(output: &str, directory: &Path) -> Result<(), Box<dyn Error>> {
    match output {
        "compose" => {
            let document = fs::read_to_string(directory.join("compose.yaml"))?;
            assert!(document.contains("  observer:"));
            assert!(!document.contains("  api:"));
        }
        "quadlet" => {
            let names = artifact_names(directory)?;
            assert!(names.iter().any(|name| name == "observer.container"));
            assert!(!names.iter().any(|name| name == "api.container"));
        }
        "podman" => {
            let plan = fs::read_to_string(directory.join("podman.json"))?;
            assert!(plan.contains("\"name\": \"observer\""));
            assert!(!plan.contains("\"name\": \"api\""));
        }
        _ => return Err(format!("unhandled output {output}").into()),
    }
    Ok(())
}

fn assert_document_reimports(name: &str, context: &str, root: &Path) -> Result<(), Box<dyn Error>> {
    for input in ["compose", "quadlet"] {
        let source = root.join(format!("{name}-{input}"));
        for output in OUTPUTS {
            let route = format!("{input}-to-{output}");
            let first = root.join(format!("{name}-{route}-reimport"));
            let result = run_document_conversion(input, output, &source, &first, context)?;
            let first_report = assert_document_route(&result, name, input, output, &first)?;

            let repeated = root.join(format!("{name}-{route}-repeat"));
            let result = run_document_conversion(input, output, &source, &repeated, context)?;
            let repeated_report = assert_document_route(&result, name, input, output, &repeated)?;
            assert_eq!(
                repeated_report["fidelity"], first_report["fidelity"],
                "{name}: {input} -> {output} fidelity counts are not repeatable"
            );
            assert_eq!(
                repeated_report["diagnostics"], first_report["diagnostics"],
                "{name}: {input} -> {output} diagnostic sequence is not repeatable"
            );
            assert_eq!(
                artifact_snapshot(&repeated)?,
                artifact_snapshot(&first)?,
                "{name}: {input} -> {output} output depends on destination path or repetition"
            );

            if input == output {
                assert_eq!(
                    artifact_snapshot(&first)?,
                    artifact_snapshot(&source)?,
                    "{name}: cassette-produced {input} projection changed after neutral-model re-import"
                );
            }

            if output != "podman" {
                let canonical = root.join(format!("{name}-{route}-canonical"));
                let result = run_document_conversion(output, output, &first, &canonical, context)?;
                let canonical_report = assert_document_route(&result, name, output, output, &canonical)?;
                assert_eq!(
                    artifact_snapshot(&canonical)?,
                    artifact_snapshot(&first)?,
                    "{name}: generated {input} -> {output} document is not a semantic fixed point"
                );

                let fixed = root.join(format!("{name}-{route}-fixed"));
                let result = run_document_conversion(output, output, &canonical, &fixed, context)?;
                let fixed_report = assert_document_route(&result, name, output, output, &fixed)?;
                assert_eq!(
                    fixed_report["fidelity"], canonical_report["fidelity"],
                    "{name}: chained {output} fidelity counts are not stable"
                );
                assert_eq!(
                    fixed_report["diagnostics"], canonical_report["diagnostics"],
                    "{name}: chained {output} diagnostic sequence is not stable"
                );
                assert_eq!(
                    artifact_snapshot(&fixed)?,
                    artifact_snapshot(&canonical)?,
                    "{name}: chained {input} -> {output} document output has no deterministic fixed point"
                );
            }
        }
    }
    Ok(())
}

fn assert_document_route(
    result: &Output,
    scenario: &str,
    input: &str,
    output: &str,
    directory: &Path,
) -> Result<serde_json::Value, Box<dyn Error>> {
    assert_route_succeeded(result, scenario, &format!("{input} -> {output} re-import"))?;
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["status"], "success");
    assert_eq!(report["exit_category"], "success");
    assert_eq!(report["source_type"], input);
    assert_eq!(report["target_type"], output);
    assert_eq!(report["application"], "complex");
    assert!(report["failed_stage"].is_null());
    let fidelity = report["fidelity"].as_object().ok_or("missing fidelity counts")?;
    for class in ["exact", "approximate", "unsupported"] {
        assert!(fidelity.get(class).is_some_and(serde_json::Value::is_u64));
    }
    assert!(report["diagnostics"].is_array(), "missing diagnostic sequence");
    assert!(
        report["output_artifacts"]
            .as_array()
            .is_some_and(|artifacts| !artifacts.is_empty())
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains(".sock"));
    assert_document_output(output, directory)?;
    for artifact in artifact_paths(directory)? {
        assert!(
            !fs::read_to_string(&artifact)?.contains(".sock"),
            "{scenario}: {input} -> {output} disclosed a socket path in {}",
            artifact.display()
        );
    }
    Ok(report)
}

fn assert_document_output(output: &str, directory: &Path) -> Result<(), Box<dyn Error>> {
    match output {
        "compose" => {
            assert_eq!(artifact_names(directory)?, ["compose.yaml"]);
            assert!(fs::read_to_string(directory.join("compose.yaml"))?.contains("services:"));
        }
        "quadlet" => {
            let names = artifact_names(directory)?;
            assert!(!names.is_empty(), "document conversion produced no Quadlet artifacts");
            for name in names {
                assert!(
                    fs::read_to_string(directory.join(name))?.contains('['),
                    "document conversion produced an empty Quadlet artifact"
                );
            }
        }
        "podman" => {
            assert_eq!(artifact_names(directory)?, ["podman-commands.sh", "podman.json"]);
            let deployment: serde_json::Value = serde_json::from_slice(&fs::read(directory.join("podman.json"))?)?;
            assert_eq!(deployment["schema_version"], 1);
            assert!(matches!(
                deployment["status"].as_str(),
                Some("exact" | "manual" | "approximate")
            ));
            assert!(deployment["connection"].is_null());
            assert!(
                deployment["operations"]
                    .as_array()
                    .is_some_and(|operations| !operations.is_empty())
            );
            assert!(fs::read_to_string(directory.join("podman-commands.sh"))?.contains("podman"));
        }
        _ => return Err(format!("unhandled output {output}").into()),
    }
    Ok(())
}

// Public PodmanLens cassette responses expose only complete, unavailable, or malformed resource
// observations. The 404 and malformed-body overlays below therefore exhaust the representable
// incomplete selected-resource states without constructing opaque internal observations.
#[test]
fn unavailable_and_malformed_inspects_block_every_exporter() -> Result<(), Box<dyn Error>> {
    let source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    let root = TemporaryDirectory::new("podman-cassette-invalid-overlays")?;
    let mut unavailable = source.clone();
    unavailable.set_status("/v6.1.0/libpod/containers/c-api/json", 404)?;
    let mut malformed = source;
    malformed.set_body("/v6.1.0/libpod/containers/c-api/json", serde_json::json!({}))?;

    for (case, cassette, native_code) in [
        ("unavailable", unavailable, "PLN0016"),
        ("malformed", malformed, "PLN0015"),
    ] {
        for output in OUTPUTS {
            let directory = root.path().join(format!("{case}-{output}"));
            let result = run_cassette(
                cassette.clone(),
                output,
                "rootless",
                &directory,
                &["--podman-all"],
                "partial",
            )?;
            assert_failed_without_output(&result, &directory, Some(native_code))?;
        }
    }
    Ok(())
}

#[test]
fn ambiguous_references_and_duplicate_identities_block_all_policies_and_exporters() -> Result<(), Box<dyn Error>> {
    let source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    let container_path = "/v6.1.0/libpod/containers/c-worker/json";
    let mut ambiguous = source.clone();
    ambiguous.insert_body_field(container_path, "Name", serde_json::json!("c-api"))?;
    let mut duplicate = source;
    duplicate.insert_body_field(container_path, "Name", serde_json::json!("api"))?;
    let root = TemporaryDirectory::new("podman-cassette-identity-conflicts")?;

    for (case, cassette) in [("ambiguous-reference", ambiguous), ("duplicate-identity", duplicate)] {
        for policy in ["exact", "approximate", "partial"] {
            for output in OUTPUTS {
                let directory = root.path().join(format!("{case}-{policy}-{output}"));
                let result = run_cassette(
                    cassette.clone(),
                    output,
                    "rootless",
                    &directory,
                    &["--podman-all"],
                    policy,
                )?;
                assert_failed_without_output(&result, &directory, None)?;
                assert_report_has_diagnostic(&result, "BFP0004")?;
            }
        }
    }
    Ok(())
}

#[test]
fn strict_policy_blocks_every_exporter_without_writes() -> Result<(), Box<dyn Error>> {
    let source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    let root = TemporaryDirectory::new("podman-cassette-strict-policy")?;

    for output in OUTPUTS {
        let directory = root.path().join(output);
        let result = run_cassette(
            source.clone(),
            output,
            "rootless",
            &directory,
            &["--podman-all"],
            "exact",
        )?;
        assert_failed_without_output(&result, &directory, None)?;
    }
    Ok(())
}

#[test]
fn explicit_network_boundary_controls_selector_crossing() -> Result<(), Box<dyn Error>> {
    let source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    let root = TemporaryDirectory::new("podman-cassette-boundary")?;
    let default_directory = root.path().join("default");
    let default = run_cassette(
        source.clone(),
        "compose",
        "rootless",
        &default_directory,
        &["--podman-resource", "container=c-observer"],
        "partial",
    )?;
    assert_route_succeeded(&default, "selector-default", "compose")?;
    let default_compose = fs::read_to_string(default_directory.join("compose.yaml"))?;
    assert!(default_compose.contains("  observer:"));
    assert!(!default_compose.contains("  api:"));
    assert!(!default_compose.contains("  isolated-control:"));

    let crossed_directory = root.path().join("crossed");
    let crossed = run_cassette(
        source,
        "compose",
        "rootless",
        &crossed_directory,
        &[
            "--podman-resource",
            "container=c-observer",
            "--podman-network-boundary",
            "app-net",
        ],
        "partial",
    )?;
    assert_route_succeeded(&crossed, "selector-crossed", "compose")?;
    let crossed_compose = fs::read_to_string(crossed_directory.join("compose.yaml"))?;
    for service in ["observer", "api", "proxy"] {
        assert!(
            crossed_compose.contains(&format!("  {service}:")),
            "authorized network crossing omitted {service}"
        );
    }
    assert!(!crossed_compose.contains("  isolated-control:"));
    Ok(())
}

#[test]
fn unexpected_secret_payload_never_reaches_reports_or_artifacts() -> Result<(), Box<dyn Error>> {
    const CANARY: &str = "BOXFERRY-SECRET-PAYLOAD-CANARY";
    let mut source = PodmanCassette::load(&fixture_directory().join("complex-6.1.0-rootless.cassette.json"))?;
    source.insert_body_field(
        "/v6.1.0/libpod/secrets/s-db/json",
        "SecretData",
        serde_json::json!(CANARY),
    )?;
    let root = TemporaryDirectory::new("podman-cassette-secret-redaction")?;

    for output in OUTPUTS {
        let directory = root.path().join(output);
        let result = run_cassette(
            source.clone(),
            output,
            "rootless",
            &directory,
            &["--podman-all"],
            "partial",
        )?;
        assert_route_succeeded(&result, "secret-redaction", output)?;
        let report = String::from_utf8(result.stdout)?;
        assert!(report.contains("PLN0018"));
        assert!(!report.contains(CANARY));
        for artifact in artifact_names(&directory)? {
            let bytes = fs::read(directory.join(artifact))?;
            assert!(!String::from_utf8_lossy(&bytes).contains(CANARY));
        }
    }
    Ok(())
}

fn assert_failed_without_output(
    result: &Output,
    directory: &Path,
    native_code: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    assert!(!result.status.success());
    assert!(result.stderr.is_empty());
    assert!(!directory.exists(), "blocked conversion wrote output");
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["schema_version"], 1);
    assert!(matches!(report["status"].as_str(), Some("failure" | "blocked")));
    assert!(report["primary_diagnostic_code"].is_string());
    assert_eq!(report["output_artifacts"].as_array().map(Vec::len), Some(0));
    if let Some(native_code) = native_code {
        assert!(
            report["diagnostics"].as_array().is_some_and(|diagnostics| diagnostics
                .iter()
                .any(|diagnostic| diagnostic["source_code"] == native_code)),
            "report omitted native code {native_code}: {report}"
        );
    }
    Ok(())
}

fn assert_report_has_diagnostic(result: &Output, expected: &str) -> Result<(), Box<dyn Error>> {
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| diagnostic["code"] == expected)),
        "report omitted {expected}: {report}"
    );
    Ok(())
}

fn run_route(cassette_path: &Path, output: &str, context: &str, directory: &Path) -> Result<Output, Box<dyn Error>> {
    run_cassette(
        PodmanCassette::load(cassette_path)?,
        output,
        context,
        directory,
        &["--podman-all"],
        "partial",
    )
}

fn run_document_conversion(
    input: &str,
    output: &str,
    source_directory: &Path,
    output_directory: &Path,
    context: &str,
) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command.args(["convert", input, output]);
    for source in artifact_paths(source_directory)? {
        command.arg("--input-file").arg(source);
    }
    if input == "quadlet" {
        command.args(["--application-name", "complex"]);
    }
    if output == "podman" {
        command.args(["--podman-target-context", context]);
    }
    command
        .args(["--loss-policy", "partial", "--output-directory"])
        .arg(output_directory)
        .args(["--console-format", "json"]);
    Ok(command.output()?)
}

fn run_cassette(
    cassette: PodmanCassette,
    output: &str,
    context: &str,
    directory: &Path,
    selection_arguments: &[&str],
    loss_policy: &str,
) -> Result<Output, Box<dyn Error>> {
    run_cassette_with_application(
        cassette,
        output,
        context,
        directory,
        selection_arguments,
        loss_policy,
        Some("complex"),
    )
}

fn run_cassette_with_application(
    cassette: PodmanCassette,
    output: &str,
    context: &str,
    directory: &Path,
    selection_arguments: &[&str],
    loss_policy: &str,
    application_name: Option<&str>,
) -> Result<Output, Box<dyn Error>> {
    let scenario = cassette.scenario_id().to_owned();
    let server = PodmanCassetteServer::start(cassette)?;
    let socket = server.socket().to_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command
        .args(["convert", "podman", output])
        .arg("--podman-socket")
        .arg(&socket);
    if let Some(application_name) = application_name {
        command.args(["--application-name", application_name]);
    }
    command
        .args(selection_arguments)
        .args([
            "--promote-podman-effective-named-volumes",
            "--promote-podman-effective-named-networks",
            "--loss-policy",
            loss_policy,
            "--output-directory",
        ])
        .arg(directory)
        .args(["--console-format", "json"]);
    if output == "podman" {
        command.args(["--podman-target-context", context]);
    }
    let result = command.output()?;
    let replay = server.finish();
    if let Err(error) = replay {
        return Err(format!(
            "cassette replay failed for {scenario} -> {output}: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(result)
}

fn assert_route_succeeded(result: &Output, scenario: &str, output: &str) -> Result<(), Box<dyn Error>> {
    if !result.status.success() {
        return Err(format!(
            "{scenario} -> {output} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    assert!(result.stderr.is_empty(), "{scenario} -> {output} wrote stderr");
    Ok(())
}

fn assert_report(bytes: &[u8], output: &str, directory: &Path) -> Result<(), Box<dyn Error>> {
    let report: serde_json::Value = serde_json::from_slice(bytes)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["status"], "success");
    assert_eq!(report["exit_category"], "success");
    assert_eq!(report["source_type"], "podman");
    assert_eq!(report["target_type"], output);
    assert_eq!(report["application"], "complex");
    assert!(report["failed_stage"].is_null());
    let text = String::from_utf8_lossy(bytes);
    assert!(!text.contains(".sock"), "report disclosed its test socket path");
    let artifacts = report["output_artifacts"]
        .as_array()
        .ok_or("report output_artifacts missing")?;
    assert!(!artifacts.is_empty());
    assert!(directory.is_dir());
    Ok(())
}

fn assert_compose_output(directory: &Path) -> Result<(), Box<dyn Error>> {
    assert_eq!(artifact_names(directory)?, ["compose.yaml"]);
    let compose = fs::read_to_string(directory.join("compose.yaml"))?;
    assert!(compose.contains("services:"));
    for service in [
        "api",
        "backup",
        "database",
        "isolated-control",
        "metrics",
        "observer",
        "proxy",
        "worker",
    ] {
        assert!(
            compose.contains(&format!("  {service}:")),
            "Compose output omitted {service}"
        );
    }
    assert!(compose.contains("networks:"));
    assert!(compose.contains("volumes:"));
    Ok(())
}

fn assert_quadlet_output(directory: &Path) -> Result<(), Box<dyn Error>> {
    let names = artifact_names(directory)?;
    assert!(
        names.len() >= 8,
        "complex Quadlet output was unexpectedly small: {names:?}"
    );
    assert!(
        names.iter().filter(|name| name.ends_with(".container")).count() >= 8,
        "Quadlet output omitted complex services: {names:?}"
    );
    for name in names {
        let text = fs::read_to_string(directory.join(name))?;
        assert!(text.contains('['), "Quadlet artifact was empty");
    }
    Ok(())
}

fn assert_podman_output(directory: &Path) -> Result<(), Box<dyn Error>> {
    assert_eq!(artifact_names(directory)?, ["podman-commands.sh", "podman.json"]);
    let bytes = fs::read(directory.join("podman.json"))?;
    let deployment: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(deployment["schema_version"], 1);
    assert!(matches!(
        deployment["status"].as_str(),
        Some("exact" | "manual" | "approximate")
    ));
    assert!(deployment["connection"].is_null());
    let external = deployment["external_preconditions"]
        .as_array()
        .ok_or("Podman external_preconditions missing")?;
    assert!(!external.is_empty());
    let operations = deployment["operations"].as_array().ok_or("Podman operations missing")?;
    assert!(!operations.is_empty());
    for operation in operations {
        assert_eq!(operation["cli"]["program"], "podman");
        assert_eq!(operation["libpod"]["method"], "POST");
        assert!(
            operation["libpod"]["path_and_query"]
                .as_str()
                .is_some_and(|path| path.starts_with("/v6.1.0/libpod/"))
        );
        assert!(operation["resource"]["kind"].is_string());
        assert!(operation["resource"]["name"].is_string());
    }
    let deployment_text = String::from_utf8(bytes)?;
    assert!(!deployment_text.contains("DATABASE_PASSWORD="));
    assert!(!deployment_text.contains(".sock"));

    let shell = fs::read_to_string(directory.join("podman-commands.sh"))?;
    assert!(shell.starts_with("#!/"));
    assert!(shell.contains("podman"));
    assert!(!shell.contains("DATABASE_PASSWORD="));
    assert!(!shell.contains(".sock"));
    Ok(())
}

fn artifact_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|name| format!("non-UTF-8 artifact name: {}", PathBuf::from(name).display()).into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(names)
}

fn artifact_paths(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    Ok(artifact_names(directory)?
        .into_iter()
        .map(|name| directory.join(name))
        .collect())
}

fn artifact_snapshot(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn Error>> {
    artifact_names(directory)?
        .into_iter()
        .map(|name| Ok((name.clone(), fs::read(directory.join(name))?)))
        .collect()
}

fn sha256(path: &Path) -> Result<String, Box<dyn Error>> {
    #[cfg(target_os = "macos")]
    let output = Command::new("shasum").args(["-a", "256"]).arg(path).output()?;
    #[cfg(not(target_os = "macos"))]
    let output = Command::new("sha256sum").arg(path).output()?;

    if !output.status.success() {
        return Err(format!("SHA-256 command failed for {}", path.display()).into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    let digest = stdout
        .split_whitespace()
        .next()
        .ok_or("SHA-256 command returned no digest")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid SHA-256 digest for {}: {digest}", path.display()).into());
    }
    Ok(digest.to_owned())
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/differential/podman-lens-complex-corpus")
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
