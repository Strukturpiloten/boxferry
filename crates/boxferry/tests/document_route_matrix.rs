//! Deterministic public contracts for every supported Compose/Quadlet document route.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    render::GeneratedComposeDocument,
    source::SourceId as ComposeSourceId,
};
use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeImporter,
    ComposeSource, Identifier, LossPolicy, PlatformVersion, QuadletDocumentInput, QuadletExporter, QuadletImporter,
    QuadletSource, SourceId, TargetProfile, convert,
};
use serde::Deserialize;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const COMPOSE_SOURCE_ID: u32 = 201;
const QUADLET_SOURCE_ID: u32 = 202;

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    extensions: MatrixExtensions,
}

#[derive(Debug, Deserialize)]
struct MatrixExtensions {
    matrix: Matrix,
}

#[derive(Debug, Deserialize)]
struct Matrix {
    inputs: BTreeMap<String, MatrixInput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct MatrixInput {
    sources: Vec<String>,
    application_name: Option<String>,
    exports: BTreeMap<String, Vec<ArtifactExpectation>>,
}

#[derive(Clone, Debug, Deserialize)]
struct ArtifactExpectation {
    artifact: String,
    expected: String,
}

#[derive(Clone, Debug)]
struct Route {
    input: String,
    output: String,
    sources: Vec<String>,
    artifacts: Vec<ArtifactExpectation>,
    application_name: Option<String>,
}

impl Matrix {
    fn routes(&self) -> Vec<Route> {
        self.inputs
            .iter()
            .flat_map(|(input, scenario)| {
                scenario.exports.iter().map(move |(output, artifacts)| Route {
                    input: input.clone(),
                    output: output.clone(),
                    sources: scenario.sources.clone(),
                    artifacts: artifacts.clone(),
                    application_name: scenario.application_name.clone(),
                })
            })
            .collect()
    }
}

#[test]
fn every_document_route_writes_reviewed_deterministic_bytes_and_stable_success_reports() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let root = TemporaryDirectory::new("route-matrix-convert")?;
    let matrix = load_matrix(&fixture)?;
    assert_matrix_matches_capabilities(&matrix)?;

    for route in matrix.routes() {
        let mut first_output = None;

        for run in 0..2 {
            let output = root.path().join(format!("{}-{}-{run}", route.input, route.output));
            let result = convert_route(&route, &fixture, &output)?;
            assert!(
                result.status.success(),
                "{} -> {} run {run} failed: {}",
                route.input,
                route.output,
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(result.stderr.is_empty());
            assert_success_report(&result.stdout, &route, "convert")?;
            let expected_names = route
                .artifacts
                .iter()
                .map(|artifact| artifact.artifact.clone())
                .collect::<Vec<_>>();
            assert_eq!(artifact_names(&output)?, expected_names);
            let output_bytes = artifact_bytes(&output)?;
            for artifact in &route.artifacts {
                assert_eq!(
                    output_bytes.get(&artifact.artifact),
                    Some(&fs::read(fixture.join(&artifact.expected))?),
                    "{} -> {} artifact {} diverged from its reviewed fixture",
                    route.input,
                    route.output,
                    artifact.artifact
                );
            }
            if route.output == "podman" {
                assert_podman_artifacts(&output, &format!("{} -> {}", route.input, route.output))?;
            }
            if let Some(first) = &first_output {
                assert_eq!(
                    output_bytes, *first,
                    "{} -> {} is not deterministic",
                    route.input, route.output
                );
            } else {
                first_output = Some(output_bytes);
            }
        }
    }
    Ok(())
}

#[test]
fn exact_generated_artifacts_reimport_to_fixed_points_and_are_path_independent() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let matrix = load_matrix(&fixture)?;
    assert_matrix_matches_capabilities(&matrix)?;
    let root = TemporaryDirectory::new("route-matrix-chain")?;
    let routes = matrix.routes();
    let mut direct_directories = BTreeMap::new();
    let mut direct_artifacts = BTreeMap::new();

    for route in &routes {
        let output = root.path().join(format!("direct-{}-{}", route.input, route.output));
        let result = convert_route(route, &fixture, &output)?;
        assert!(
            result.status.success(),
            "{} -> {} failed: {}",
            route.input,
            route.output,
            String::from_utf8_lossy(&result.stderr)
        );
        direct_artifacts.insert((route.input.clone(), route.output.clone()), artifact_bytes(&output)?);
        direct_directories.insert((route.input.clone(), route.output.clone()), output);
    }

    let outputs = routes.iter().map(|route| route.output.clone()).collect::<BTreeSet<_>>();
    let document_outputs = outputs
        .iter()
        .filter(|output| output.as_str() != "podman")
        .collect::<Vec<_>>();
    for input in matrix.inputs.keys() {
        for intermediate in document_outputs.iter().copied() {
            let first_directory = direct_directories
                .get(&(input.clone(), intermediate.clone()))
                .ok_or("direct intermediate output")?;
            let first_sources = artifact_paths(first_directory)?;
            let application_name = (intermediate == "quadlet").then_some("route-matrix");

            let second_directory = root.path().join(format!("fixed-{input}-{intermediate}-2"));
            run_conversion(
                intermediate,
                intermediate,
                &first_sources,
                application_name,
                &second_directory,
                "exact",
            )?;
            assert_eq!(
                artifact_bytes(first_directory)?,
                artifact_bytes(&second_directory)?,
                "{input} -> {intermediate} did not reach its same-format fixed point"
            );

            let third_directory = root.path().join(format!("fixed-{input}-{intermediate}-3"));
            run_conversion(
                intermediate,
                intermediate,
                &artifact_paths(&second_directory)?,
                application_name,
                &third_directory,
                "exact",
            )?;
            assert_eq!(
                artifact_bytes(&second_directory)?,
                artifact_bytes(&third_directory)?,
                "{input} -> {intermediate} fixed point is not stable"
            );

            for terminal in &outputs {
                let chained_directory = root.path().join(format!("chain-{input}-{intermediate}-{terminal}"));
                let loss_policy = if terminal == "podman" { "partial" } else { "exact" };
                run_conversion(
                    intermediate,
                    terminal,
                    &first_sources,
                    application_name,
                    &chained_directory,
                    loss_policy,
                )?;
                assert_eq!(
                    artifact_bytes(&chained_directory)?,
                    *direct_artifacts
                        .get(&(input.clone(), terminal.clone()))
                        .ok_or("direct terminal output")?,
                    "{input} -> {intermediate} -> {terminal} differs from direct {input} -> {terminal}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn realistic_lossy_chain_is_policy_controlled_stable_and_redacted() -> Result<(), Box<dyn Error>> {
    const CANARIES: [&str; 3] = ["LABEL-CANARY", "ANNOTATION-CANARY", "LOGOPT-CANARY"];

    let fixture = fixture_directory();
    let source = [fixture.join("lossy-compose.yaml")];
    let root = TemporaryDirectory::new("route-matrix-lossy-chain")?;
    let mut reports = Vec::new();

    for policy in ["exact", "approximate"] {
        let output = root.path().join(format!("blocked-{policy}"));
        let result = execute_conversion("compose", "quadlet", &source, None, &output, policy)?;
        assert!(
            !result.status.success(),
            "{policy} unexpectedly authorized a partial conversion"
        );
        assert!(result.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "blocked");
        assert_eq!(report["exit_category"], "policy-blocked");
        assert_eq!(report["fidelity"]["approximate"], 2);
        assert_eq!(report["fidelity"]["unsupported"], 1);
        assert_eq!(report["output_artifacts"], serde_json::json!([]));
        assert!(!output.exists(), "{policy} wrote blocked output");
        reports.push(report);
    }

    let first_quadlet = root.path().join("compose-quadlet");
    let first_report = run_conversion("compose", "quadlet", &source, None, &first_quadlet, "partial")?;
    assert_eq!(first_report["fidelity"]["approximate"], 2);
    assert_eq!(first_report["fidelity"]["unsupported"], 1);
    assert_eq!(
        report_diagnostic_codes(&first_report)?,
        ["BFQ0003", "BFQ0007", "BFQ0007"]
    );
    assert_eq!(
        fs::read(first_quadlet.join("web.container"))?,
        fs::read(fixture.join("expected-lossy-web.container"))?
    );
    reports.push(first_report);

    let second_quadlet = root.path().join("quadlet-fixed-point");
    let second_report = run_conversion(
        "quadlet",
        "quadlet",
        &artifact_paths(&first_quadlet)?,
        Some("lossy-chain"),
        &second_quadlet,
        "partial",
    )?;
    assert_eq!(
        report_diagnostic_codes(&second_report)?,
        ["BFQ0007", "BFQ0007", "BFQ1003", "BFQ1003"]
    );
    assert_eq!(
        artifact_bytes(&first_quadlet)?,
        artifact_bytes(&second_quadlet)?,
        "generated quoted Label, Annotation, and LogOpt assignments did not re-import to a fixed point"
    );
    reports.push(second_report);

    let chained_compose = root.path().join("quadlet-compose");
    let chained_report = run_conversion(
        "quadlet",
        "compose",
        &artifact_paths(&first_quadlet)?,
        Some("lossy-chain"),
        &chained_compose,
        "partial",
    )?;
    assert_eq!(
        report_diagnostic_codes(&chained_report)?,
        ["BFC0007", "BFQ1003", "BFQ1003"]
    );
    assert_eq!(
        fs::read(chained_compose.join("compose.yaml"))?,
        fs::read(fixture.join("expected-lossy-compose-after-quadlet.yaml"))?
    );
    reports.push(chained_report);

    let direct_compose = root.path().join("compose-compose");
    let direct_report = run_conversion("compose", "compose", &source, None, &direct_compose, "partial")?;
    assert_eq!(report_diagnostic_codes(&direct_report)?, ["BFC0007"]);
    assert_eq!(
        artifact_bytes(&direct_compose)?,
        artifact_bytes(&chained_compose)?,
        "direct Compose projection differs from the Compose -> Quadlet -> Compose path"
    );
    reports.push(direct_report);

    let compose_fixed_point = root.path().join("compose-fixed-point");
    let fixed_report = run_conversion(
        "compose",
        "compose",
        &artifact_paths(&chained_compose)?,
        None,
        &compose_fixed_point,
        "exact",
    )?;
    assert!(report_diagnostic_codes(&fixed_report)?.is_empty());
    assert_eq!(
        artifact_bytes(&chained_compose)?,
        artifact_bytes(&compose_fixed_point)?,
        "the lossy chain's stable Compose projection is not a same-format fixed point"
    );
    reports.push(fixed_report);

    for report in reports {
        let serialized = serde_json::to_string(&report)?;
        for canary in CANARIES {
            assert!(!serialized.contains(canary), "structured report disclosed {canary}");
        }
    }
    Ok(())
}

#[test]
fn explicit_files_and_directory_discovery_produce_identical_artifacts() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let matrix = load_matrix(&fixture)?;
    assert_matrix_matches_capabilities(&matrix)?;
    let root = TemporaryDirectory::new("route-matrix-discovery")?;

    for (input, scenario) in &matrix.inputs {
        let input_directory = root.path().join(format!("{input}-input"));
        fs::create_dir(&input_directory)?;
        for source in &scenario.sources {
            fs::copy(fixture.join(source), input_directory.join(source))?;
        }
        for output in scenario.exports.keys() {
            let discovered_output = root.path().join(format!("{input}-{output}-discovered"));
            let mut command = boxferry_command();
            command
                .args(["convert", input, output, "--input-directory"])
                .arg(&input_directory);
            if output == "podman" {
                command.args(["--podman-target-context", "unknown", "--loss-policy", "partial"]);
            }
            if let Some(application_name) = scenario.application_name.as_deref() {
                command.args(["--application-name", application_name]);
            }
            let result = command
                .arg("--output-directory")
                .arg(&discovered_output)
                .args(["--console-format", "json"])
                .output()?;
            assert!(
                result.status.success(),
                "{input} -> {output} discovery failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );

            let explicit_output = root.path().join(format!("{input}-{output}-explicit"));
            run_conversion(
                input,
                output,
                &scenario
                    .sources
                    .iter()
                    .map(|source| fixture.join(source))
                    .collect::<Vec<_>>(),
                scenario.application_name.as_deref(),
                &explicit_output,
                if output == "podman" { "partial" } else { "exact" },
            )?;
            assert_eq!(
                artifact_bytes(&discovered_output)?,
                artifact_bytes(&explicit_output)?,
                "{input} -> {output} depends on discovery path"
            );
        }
    }
    Ok(())
}

#[test]
fn every_document_route_validates_planned_artifacts_without_writing() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let root = TemporaryDirectory::new("route-matrix-validate")?;
    let matrix = load_matrix(&fixture)?;
    assert_matrix_matches_capabilities(&matrix)?;

    for route in matrix.routes() {
        let mut command = boxferry_command();
        command
            .current_dir(root.path())
            .args(["validate", route.input.as_str(), route.output.as_str()]);
        for source in &route.sources {
            command.arg("--input-file").arg(fixture.join(source));
        }
        if let Some(application_name) = route.application_name.as_deref() {
            command.args(["--application-name", application_name]);
        }
        if route.output == "podman" {
            command.args(["--podman-target-context", "unknown", "--loss-policy", "partial"]);
        }
        let result = command.args(["--console-format", "json"]).output()?;
        assert!(
            result.status.success(),
            "{} -> {} validation failed: {}",
            route.input,
            route.output,
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty());
        assert_success_report(&result.stdout, &route, "validate")?;
        assert_eq!(artifact_names(root.path())?, Vec::<String>::new());
    }
    Ok(())
}

#[test]
fn malformed_native_input_fails_before_writing_for_every_document_route() -> Result<(), Box<dyn Error>> {
    let root = TemporaryDirectory::new("route-matrix-invalid")?;
    let matrix = load_matrix(&fixture_directory())?;
    assert_matrix_matches_capabilities(&matrix)?;

    for route in matrix.routes() {
        let (extension, contents, failed_stage, diagnostic_prefix, application_name) = match route.input.as_str() {
            "compose" => ("yaml", "services: [not-valid\n", "compose-merge", "BFC", None),
            "quadlet" => (
                "container",
                "[Container]\nImage=\n",
                "quadlet-parse",
                "BFQ",
                Some("route-matrix"),
            ),
            input => return Err(format!("unsupported matrix input {input}").into()),
        };
        let input = route.input.as_str();
        let output = route.output.as_str();
        let input_path = root.path().join(format!("invalid-{input}-{output}.{extension}"));
        let output_path = root.path().join(format!("output-{input}-{output}"));
        fs::write(&input_path, contents)?;

        let mut command = boxferry_command();
        command
            .args(["convert", input, output, "--input-file"])
            .arg(&input_path);
        if let Some(application_name) = application_name {
            command.args(["--application-name", application_name]);
        }
        if output == "podman" {
            command.args(["--podman-target-context", "unknown"]);
        }
        let result = command
            .arg("--output-directory")
            .arg(&output_path)
            .args(["--console-format", "json"])
            .output()?;

        assert!(!result.status.success(), "{input} -> {output} unexpectedly succeeded");
        assert!(result.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "failure");
        assert_eq!(report["exit_category"], "input-or-execution");
        assert_eq!(report["failed_stage"], failed_stage);
        assert_eq!(report["source_type"], input);
        assert_eq!(report["target_type"], output);
        assert!(
            report["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                    diagnostic["code"]
                        .as_str()
                        .is_some_and(|code| code.starts_with(diagnostic_prefix))
                }))
        );
        assert_eq!(report["output_artifacts"], serde_json::json!([]));
        assert!(
            !output_path.exists(),
            "{input} -> {output} created output before failing"
        );
    }
    Ok(())
}

#[test]
fn public_facade_routes_every_pair_through_the_neutral_application() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let expected_compose = fs::read_to_string(fixture.join("expected-compose-neutral.yaml"))?;
    let expected_quadlet = fs::read_to_string(fixture.join("expected-web.container"))?;
    let compose_source = load_compose_source(&fixture)?;
    let quadlet_source = load_quadlet_source(&fixture)?;

    let compose_importer = ComposeImporter::new()?;
    let quadlet_importer = QuadletImporter::new()?;
    let compose_exporter = ComposeExporter::new()?;
    let quadlet_exporter = QuadletExporter::new()?;

    let compose_target = TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    let quadlet_target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let compose_to_compose = convert(
        &compose_importer,
        &compose_source,
        &compose_exporter,
        &compose_target,
        LossPolicy::ExactOnly,
    )?;
    assert!(
        !compose_to_compose.is_blocked(),
        "{:#?}",
        compose_to_compose.diagnostics()
    );
    assert_eq!(
        compose_to_compose.output().map(GeneratedComposeDocument::text),
        Some(expected_compose.as_str())
    );

    let compose_to_quadlet = convert(
        &compose_importer,
        &compose_source,
        &quadlet_exporter,
        &quadlet_target,
        LossPolicy::ExactOnly,
    )?;
    assert!(
        !compose_to_quadlet.is_blocked(),
        "{:#?}",
        compose_to_quadlet.diagnostics()
    );
    assert_eq!(
        compose_to_quadlet
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry::QuadletFile::text),
        Some(expected_quadlet.as_str())
    );

    let quadlet_to_compose = convert(
        &quadlet_importer,
        &quadlet_source,
        &compose_exporter,
        &compose_target,
        LossPolicy::ExactOnly,
    )?;
    assert!(
        !quadlet_to_compose.is_blocked(),
        "{:#?}",
        quadlet_to_compose.diagnostics()
    );
    assert_eq!(
        quadlet_to_compose.output().map(GeneratedComposeDocument::text),
        Some(expected_compose.as_str())
    );

    let quadlet_to_quadlet = convert(
        &quadlet_importer,
        &quadlet_source,
        &quadlet_exporter,
        &quadlet_target,
        LossPolicy::ExactOnly,
    )?;
    assert!(
        !quadlet_to_quadlet.is_blocked(),
        "{:#?}",
        quadlet_to_quadlet.diagnostics()
    );
    assert_eq!(
        quadlet_to_quadlet
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry::QuadletFile::text),
        Some(expected_quadlet.as_str())
    );

    Ok(())
}

fn convert_route(route: &Route, fixture: &Path, output: &Path) -> Result<std::process::Output, Box<dyn Error>> {
    let mut command = boxferry_command();
    command.args(["convert", route.input.as_str(), route.output.as_str()]);
    for source in &route.sources {
        command.arg("--input-file").arg(fixture.join(source));
    }
    if let Some(application_name) = route.application_name.as_deref() {
        command.args(["--application-name", application_name]);
    }
    if route.output == "podman" {
        command.args(["--podman-target-context", "unknown", "--loss-policy", "partial"]);
    }
    Ok(command
        .arg("--output-directory")
        .arg(output)
        .args(["--console-format", "json"])
        .output()?)
}

fn run_conversion(
    input: &str,
    output: &str,
    sources: &[PathBuf],
    application_name: Option<&str>,
    output_directory: &Path,
    loss_policy: &str,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let result = execute_conversion(input, output, sources, application_name, output_directory, loss_policy)?;
    if !result.status.success() {
        return Err(format!(
            "{input} -> {output} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&result.stdout)?)
}

fn execute_conversion(
    input: &str,
    output: &str,
    sources: &[PathBuf],
    application_name: Option<&str>,
    output_directory: &Path,
    loss_policy: &str,
) -> Result<std::process::Output, Box<dyn Error>> {
    let mut command = boxferry_command();
    command.args(["convert", input, output]);
    for source in sources {
        command.arg("--input-file").arg(source);
    }
    if let Some(application_name) = application_name {
        command.args(["--application-name", application_name]);
    }
    if output == "podman" {
        command.args(["--podman-target-context", "unknown"]);
    }
    let result = command
        .args(["--loss-policy", loss_policy, "--output-directory"])
        .arg(output_directory)
        .args(["--console-format", "json"])
        .output()?;
    Ok(result)
}

fn assert_success_report(bytes: &[u8], route: &Route, command_kind: &str) -> Result<(), Box<dyn Error>> {
    let report: serde_json::Value = serde_json::from_slice(bytes)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["status"], "success");
    assert_eq!(report["exit_category"], "success");
    assert!(report["failed_stage"].is_null());
    assert!(report["primary_diagnostic_code"].is_null());
    assert!(report["failure_summary"].is_null());
    assert!(report["fix_first"].is_null());
    assert_eq!(report["source_type"], route.input);
    assert_eq!(report["target_type"], route.output);
    assert_eq!(report["application"], "route-matrix");
    assert_eq!(
        report_diagnostic_codes(&report)?,
        if route.output == "podman" {
            vec!["BFP0007"]
        } else {
            Vec::new()
        }
    );
    assert_eq!(report["events"], serde_json::json!([]));
    assert_eq!(
        report["output_artifacts"]
            .as_array()
            .ok_or("output artifacts")?
            .iter()
            .map(|artifact| artifact["name"].as_str().ok_or("artifact name"))
            .collect::<Result<Vec<_>, _>>()?,
        route
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(report["invocation"]["command_kind"], command_kind);
    Ok(())
}

fn assert_podman_artifacts(output_directory: &Path, route: &str) -> Result<(), Box<dyn Error>> {
    let deployment_text = fs::read_to_string(output_directory.join("podman.json"))?;
    let review = fs::read_to_string(output_directory.join("podman-commands.sh"))?;
    let deployment: serde_json::Value = serde_json::from_str(&deployment_text)?;

    assert_eq!(deployment["schema_version"], 1, "{route} Podman schema changed");
    assert!(
        matches!(
            deployment["status"].as_str(),
            Some("exact" | "deferred_sensitive_input")
        ),
        "{route} Podman deployment has an unexpected status"
    );
    assert!(
        deployment["connection"].is_null(),
        "{route} Podman output must remain connection-independent"
    );
    assert!(
        deployment["external_preconditions"].is_array(),
        "{route} Podman preconditions must be explicit"
    );

    let operations = deployment["operations"]
        .as_array()
        .ok_or_else(|| format!("{route} Podman operations array missing"))?;
    assert!(!operations.is_empty(), "{route} Podman deployment has no operations");
    let command_lines = review
        .lines()
        .filter(|line| line.starts_with("podman "))
        .collect::<Vec<_>>();
    assert_eq!(
        command_lines.len(),
        operations.len(),
        "{route} command script operation count changed"
    );

    for (operation, command_line) in operations.iter().zip(command_lines) {
        assert!(
            operation["status"].as_str().is_some(),
            "{route} Podman operation status missing"
        );
        assert!(
            operation["action"].as_str().is_some_and(|action| !action.is_empty()),
            "{route} Podman operation action missing"
        );
        assert!(
            operation["resource"]["kind"]
                .as_str()
                .is_some_and(|kind| !kind.is_empty()),
            "{route} Podman operation resource kind missing"
        );
        assert!(
            operation["resource"]["name"]
                .as_str()
                .is_some_and(|name| !name.is_empty()),
            "{route} Podman operation resource name missing"
        );
        assert_eq!(operation["cli"]["program"], "podman");
        let arguments = operation["cli"]["argv"]
            .as_array()
            .ok_or_else(|| format!("{route} Podman CLI arguments missing"))?
            .iter()
            .map(|argument| {
                argument
                    .as_str()
                    .ok_or_else(|| format!("{route} Podman CLI argument is not text"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert!(!arguments.is_empty(), "{route} Podman CLI operation is empty");
        let expected_command = format!(
            "podman {}",
            arguments
                .iter()
                .map(|argument| podman_shell_quote(argument))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(
            command_line.starts_with(&expected_command),
            "{route} review command differs from podman.json: {command_line}"
        );
        assert_eq!(operation["libpod"]["method"], "POST");
        assert!(
            operation["libpod"]["path_and_query"]
                .as_str()
                .is_some_and(|path| path.starts_with("/v6.1.0/libpod/")),
            "{route} operation does not use the default reviewed Podman 6.1 target"
        );
        assert!(
            operation["libpod"]["body"].is_object(),
            "{route} Podman operation body missing"
        );
        if operation["cli"]["external_sensitive_input_required"] == true {
            assert_eq!(operation["libpod"]["body"]["kind"], "external_sensitive_input");
            assert!(
                review.contains("PODMAN_LENS_SECRET_INPUT_"),
                "{route} sensitive operation lacks an explicit review placeholder"
            );
        }
    }

    assert!(review.starts_with("#!/bin/sh\n"));
    assert!(review.contains("# Review generated Podman commands before running this file.\n"));
    assert!(review.contains("set -eu\n"));
    Ok(())
}

fn podman_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn report_diagnostic_codes(report: &serde_json::Value) -> Result<Vec<&str>, String> {
    report["diagnostics"]
        .as_array()
        .ok_or_else(|| "report diagnostics array missing".to_owned())?
        .iter()
        .map(|diagnostic| {
            diagnostic["code"]
                .as_str()
                .ok_or_else(|| "diagnostic code missing".to_owned())
        })
        .collect()
}

fn load_matrix(fixture: &Path) -> Result<Matrix, Box<dyn Error>> {
    let manifest: FixtureManifest = toml::from_str(&fs::read_to_string(fixture.join("fixture.toml"))?)?;
    Ok(manifest.extensions.matrix)
}

fn assert_matrix_matches_capabilities(matrix: &Matrix) -> Result<(), Box<dyn Error>> {
    let capabilities = boxferry_command()
        .args(["capabilities", "--console-format", "json"])
        .output()?;
    if !capabilities.status.success() {
        return Err(format!(
            "capability discovery failed: {}",
            String::from_utf8_lossy(&capabilities.stderr)
        )
        .into());
    }
    let document: serde_json::Value = serde_json::from_slice(&capabilities.stdout)?;
    let registered = document["routes"]
        .as_array()
        .ok_or("capability routes")?
        .iter()
        .filter(|route| {
            route["input_type"]
                .as_str()
                .is_some_and(|input| matches!(input, "compose" | "quadlet"))
        })
        .map(|route| {
            Ok((
                route["input_type"].as_str().ok_or("capability input")?.to_owned(),
                route["output_type"].as_str().ok_or("capability output")?.to_owned(),
            ))
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    let declared = matrix
        .inputs
        .iter()
        .flat_map(|(input, scenario)| {
            scenario
                .exports
                .keys()
                .map(move |output| (input.clone(), output.clone()))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(declared, registered, "fixture matrix must cover every registered route");

    let registered_inputs = registered
        .iter()
        .map(|(input, _)| input.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        matrix.inputs.keys().cloned().collect::<BTreeSet<_>>(),
        registered_inputs,
        "every registered importer needs one fixture scenario"
    );
    for (input, scenario) in &matrix.inputs {
        assert!(!scenario.sources.is_empty(), "{input} scenario has no source");
        let expected_outputs = registered
            .iter()
            .filter(|(registered_input, _)| registered_input == input)
            .map(|(_, output)| output.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            scenario.exports.keys().cloned().collect::<BTreeSet<_>>(),
            expected_outputs,
            "{input} scenario must define every registered exporter"
        );
        assert!(
            scenario.exports.values().all(|artifacts| !artifacts.is_empty()),
            "{input} scenario has an exporter without reviewed artifacts"
        );
    }
    Ok(())
}

fn artifact_bytes(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn Error>> {
    artifact_names(directory)?
        .into_iter()
        .map(|name| Ok((name.clone(), fs::read(directory.join(name))?)))
        .collect()
}

fn artifact_paths(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    Ok(artifact_names(directory)?
        .into_iter()
        .map(|name| directory.join(name))
        .collect())
}

fn artifact_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            entry
                .file_name()
                .into_string()
                .map_err(|name| format!("non-UTF-8 artifact name: {}", PathBuf::from(name).display()).into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(names)
}

fn load_compose_source(fixture: &Path) -> Result<ComposeSource, Box<dyn Error>> {
    load_compose_source_from_text(&fs::read_to_string(fixture.join("compose.yaml"))?)
}

fn load_compose_source_from_text(text: &str) -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(COMPOSE_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("compose.yaml", "document-route-matrix"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged Compose project")?.clone();
    Ok(ComposeSource::new(project, Identifier::new("route-matrix")?)?
        .with_source_id(source_id, SourceId::new("compose.yaml")?))
}

fn load_quadlet_source(fixture: &Path) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(
        Identifier::new("route-matrix")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(QUADLET_SOURCE_ID),
            fs::read_to_string(fixture.join("web.container"))?,
        )],
    )?
    .into_source())
}

fn boxferry_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_boxferry"))
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/document-route-matrix")
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-{label}-{}-{id}", std::process::id()));
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
