//! Deterministic public contracts for every supported Compose/Quadlet document route.

#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet"))]

use std::{
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

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const COMPOSE_SOURCE_ID: u32 = 201;
const QUADLET_SOURCE_ID: u32 = 202;

struct Route {
    input: &'static str,
    output: &'static str,
    source: &'static str,
    expected: &'static str,
    artifact: &'static str,
    application_name: Option<&'static str>,
}

const ROUTES: [Route; 4] = [
    Route {
        input: "compose",
        output: "compose",
        source: "compose.yaml",
        expected: "expected-compose-neutral.yaml",
        artifact: "compose.yaml",
        application_name: None,
    },
    Route {
        input: "compose",
        output: "quadlet",
        source: "compose.yaml",
        expected: "expected-web.container",
        artifact: "web.container",
        application_name: None,
    },
    Route {
        input: "quadlet",
        output: "compose",
        source: "web.container",
        expected: "expected-compose-neutral.yaml",
        artifact: "compose.yaml",
        application_name: Some("route-matrix"),
    },
    Route {
        input: "quadlet",
        output: "quadlet",
        source: "web.container",
        expected: "expected-web.container",
        artifact: "web.container",
        application_name: Some("route-matrix"),
    },
];

#[test]
fn every_document_route_writes_reviewed_deterministic_bytes_and_stable_success_reports() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let root = TemporaryDirectory::new("route-matrix-convert")?;

    for route in &ROUTES {
        let source = fixture.join(route.source);
        let expected = fs::read(fixture.join(route.expected))?;
        let mut first_output = None;

        for run in 0..2 {
            let output = root.path().join(format!("{}-{}-{run}", route.input, route.output));
            let result = convert_route(route, &source, &output)?;
            assert!(
                result.status.success(),
                "{} -> {} run {run} failed: {}",
                route.input,
                route.output,
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(result.stderr.is_empty());
            assert_success_report(&result.stdout, route, "convert")?;
            assert_eq!(artifact_names(&output)?, [route.artifact]);
            let output_bytes = fs::read(output.join(route.artifact))?;
            assert_eq!(
                output_bytes, expected,
                "{} -> {} output diverged from its reviewed fixture",
                route.input, route.output
            );
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
fn every_document_route_validates_planned_artifacts_without_writing() -> Result<(), Box<dyn Error>> {
    let fixture = fixture_directory();
    let root = TemporaryDirectory::new("route-matrix-validate")?;

    for route in &ROUTES {
        let source = fixture.join(route.source);
        let mut command = boxferry_command();
        command
            .current_dir(root.path())
            .args(["validate", route.input, route.output, "--input-file"])
            .arg(&source);
        if let Some(application_name) = route.application_name {
            command.args(["--application-name", application_name]);
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
        assert_success_report(&result.stdout, route, "validate")?;
        assert_eq!(artifact_names(root.path())?, Vec::<String>::new());
    }
    Ok(())
}

#[test]
fn malformed_native_input_fails_before_writing_for_every_document_route() -> Result<(), Box<dyn Error>> {
    let root = TemporaryDirectory::new("route-matrix-invalid")?;

    for (input, output, extension, contents, failed_stage, diagnostic_prefix, application_name) in [
        (
            "compose",
            "compose",
            "yaml",
            "services: [not-valid\n",
            "compose-merge",
            "BFC",
            None,
        ),
        (
            "compose",
            "quadlet",
            "yaml",
            "services: [not-valid\n",
            "compose-merge",
            "BFC",
            None,
        ),
        (
            "quadlet",
            "compose",
            "container",
            "[Container]\nImage=\n",
            "quadlet-parse",
            "BFQ",
            Some("route-matrix"),
        ),
        (
            "quadlet",
            "quadlet",
            "container",
            "[Container]\nImage=\n",
            "quadlet-parse",
            "BFQ",
            Some("route-matrix"),
        ),
    ] {
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

fn convert_route(route: &Route, source: &Path, output: &Path) -> Result<std::process::Output, Box<dyn Error>> {
    let mut command = boxferry_command();
    command
        .args(["convert", route.input, route.output, "--input-file"])
        .arg(source);
    if let Some(application_name) = route.application_name {
        command.args(["--application-name", application_name]);
    }
    Ok(command
        .arg("--output-directory")
        .arg(output)
        .args(["--console-format", "json"])
        .output()?)
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
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["events"], serde_json::json!([]));
    assert_eq!(
        report["output_artifacts"]
            .as_array()
            .ok_or("output artifacts")?
            .iter()
            .map(|artifact| artifact["name"].as_str().ok_or("artifact name"))
            .collect::<Result<Vec<_>, _>>()?,
        [route.artifact]
    );
    assert_eq!(report["invocation"]["command_kind"], command_kind);
    Ok(())
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
