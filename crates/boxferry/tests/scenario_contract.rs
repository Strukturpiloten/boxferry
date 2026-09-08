//! Offline acceptance: authored intent, every exporter, separate executed evidence.
#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[allow(dead_code, unused_imports)]
mod support;

use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use boxferry::compose::compose_lens::{
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::podman::podman_lens::{
    AcquisitionOptions, DiscoveryRequest, ReadOnlyUnixTransport, ReadOnlyUnixTransportTimeouts,
    ResourceKind as PodmanResourceKind, ResourceSelector, TransportLimits, UnixConnection,
};
use boxferry::{
    ComposeExporter, ComposeImporter, ComposeSource, ConversionKind, ConversionResult, Identifier, ImportAdapter,
    ImportResult, PodmanExporter, PodmanImporter, PodmanPromotionPolicy, QuadletDocumentInput, QuadletExporter,
    QuadletGroupingPolicy, QuadletImporter, QuadletSource, SourceId, acquire_podman_source, convert_imported,
};
use serde::Deserialize;
use support::{
    Dimension, DimensionState, NativeInput, Outcome, PodmanCassette, PodmanCassetteServer, RouteExpectation,
    RouteObservation, ScenarioManifest, diagnostic_facts, load_loss_expectations, load_string_expectations,
    observe_prerequisites, validate_diagnostics, validate_evidence, validate_exporter_coverage, validate_losses,
    validate_manifest, validate_neutral_application,
};

const OFFLINE: &str = "Offline authored fixture; no runtime operation is claimed.";
const NO_PODMAN_REIMPORT: &str = "Deployment plans are not observed Podman inventories.";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const SCENARIO_CATALOGUE: &str = "fixtures/scenarios/catalogue.toml";
const APPROVED_SCENARIO_ROOTS: &[&str] = &[
    "fixtures/adapter-contract",
    "fixtures/conversion",
    "fixtures/differential",
    "fixtures/real-world",
    "fixtures/scenarios",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioCatalogue {
    schema: u32,
    scenarios: Vec<ScenarioRegistration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RealWorldCorpus {
    schema: u32,
    reviewed: String,
    projects: Vec<RealWorldCorpusProject>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RealWorldCorpusProject {
    id: String,
    tier: String,
    repository: String,
    revision: String,
    path: String,
    blob_sha: String,
    license: String,
    minimum_services: usize,
    compose_fields: Vec<String>,
    goals: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioRegistration {
    id: String,
    manifest: String,
}

#[derive(Debug)]
struct RegisteredScenario {
    id: String,
    manifest: PathBuf,
}

fn validate_real_world_provenance_link(
    registration_id: &str,
    manifest: &ScenarioManifest,
    project: &RealWorldCorpusProject,
    observed_blob_sha: &str,
) -> Result<(), String> {
    if registration_id != format!("real-world-compose-{}", project.id)
        || manifest.provenance.corpus_project.as_deref() != Some(project.id.as_str())
        || manifest.provenance.source != "external"
        || manifest.application.name != project.id
    {
        return Err("real-world registration identity differs from pinned provenance".into());
    }
    if manifest.application.revision.as_deref() != Some(project.revision.as_str()) {
        return Err("real-world revision differs from pinned provenance".into());
    }
    if manifest.provenance.license != project.license {
        return Err("real-world license differs from pinned provenance".into());
    }
    if observed_blob_sha != project.blob_sha {
        return Err("real-world source blob differs from pinned provenance".into());
    }
    Ok(())
}

fn registered_scenarios(root: &Path) -> Result<Vec<RegisteredScenario>, Box<dyn Error>> {
    let catalogue: ScenarioCatalogue = toml::from_str(&fs::read_to_string(root.join(SCENARIO_CATALOGUE))?)?;
    Ok(validate_scenario_catalogue(root, &catalogue)?)
}

fn validate_scenario_catalogue(root: &Path, catalogue: &ScenarioCatalogue) -> Result<Vec<RegisteredScenario>, String> {
    if catalogue.schema != 1 {
        return Err("scenario catalogue schema must be exactly 1".into());
    }
    if catalogue.scenarios.is_empty() {
        return Err("scenario catalogue cannot be empty".into());
    }

    let mut ids = BTreeSet::new();
    let mut manifests = BTreeSet::new();
    let mut registered = Vec::with_capacity(catalogue.scenarios.len());
    for registration in &catalogue.scenarios {
        if registration.id.is_empty()
            || registration
                .id
                .bytes()
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        {
            return Err(format!("unsafe scenario catalogue id {}", registration.id));
        }
        let relative = Path::new(&registration.manifest);
        if registration.manifest.is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
            || !is_scenario_manifest_name(relative, &registration.id)
        {
            return Err(format!("unsafe scenario manifest path {}", registration.manifest));
        }
        if !APPROVED_SCENARIO_ROOTS
            .iter()
            .any(|approved| relative.starts_with(Path::new(approved)))
        {
            return Err(format!(
                "scenario manifest is outside approved fixture roots: {}",
                registration.manifest
            ));
        }
        if !manifests.insert(relative.to_path_buf()) {
            return Err(format!("duplicate scenario manifest path {}", registration.manifest));
        }
        if !ids.insert(registration.id.as_str()) {
            return Err(format!("duplicate scenario catalogue id {}", registration.id));
        }

        let manifest = root.join(relative);
        if !manifest.is_file() {
            return Err(format!("missing scenario manifest {}", registration.manifest));
        }
        let canonical_manifest = manifest.canonicalize().map_err(|error| error.to_string())?;
        let within_approved_root = APPROVED_SCENARIO_ROOTS.iter().try_fold(false, |within, approved| {
            let canonical_root = root.join(approved).canonicalize().map_err(|error| error.to_string())?;
            Ok::<_, String>(within || canonical_manifest.starts_with(canonical_root))
        })?;
        if !within_approved_root {
            return Err(format!(
                "scenario manifest resolves outside approved fixture roots: {}",
                registration.manifest
            ));
        }

        registered.push(RegisteredScenario {
            id: registration.id.clone(),
            manifest,
        });
    }
    Ok(registered)
}

fn is_scenario_manifest_name(path: &Path, id: &str) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if file_name == "scenario.toml" {
        return path.parent().and_then(Path::file_name).and_then(|name| name.to_str()) == Some(id);
    }
    file_name.strip_suffix(".scenario.toml") == Some(id)
}

#[test]
fn scenario_catalogue_is_explicit_bounded_and_sidecar_ready() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let catalogue: ScenarioCatalogue = toml::from_str(&fs::read_to_string(root.join(SCENARIO_CATALOGUE))?)?;
    let registered = validate_scenario_catalogue(&root, &catalogue)?;
    assert_eq!(registered.len(), 26);
    assert!(is_scenario_manifest_name(
        Path::new("fixtures/conversion/document-route-matrix/document-route-normal.scenario.toml"),
        "document-route-normal"
    ));

    let mut duplicate = catalogue.clone();
    duplicate.scenarios.push(duplicate.scenarios[0].clone());
    let error = validate_scenario_catalogue(&root, &duplicate)
        .err()
        .ok_or("duplicate catalogue path must fail")?;
    assert!(
        error.contains("duplicate scenario manifest path"),
        "wrong duplicate error: {error}"
    );

    let mut missing = catalogue.clone();
    missing.scenarios[0].manifest = "fixtures/scenarios/authored-core/authored-core.scenario.toml".into();
    let error = validate_scenario_catalogue(&root, &missing)
        .err()
        .ok_or("missing catalogue path must fail")?;
    assert!(
        error.contains("missing scenario manifest"),
        "wrong missing error: {error}"
    );

    let mut unsafe_path = catalogue.clone();
    unsafe_path.scenarios[0].manifest = "../authored-core/scenario.toml".into();
    let error = validate_scenario_catalogue(&root, &unsafe_path)
        .err()
        .ok_or("unsafe catalogue path must fail")?;
    assert!(
        error.contains("unsafe scenario manifest path"),
        "wrong unsafe error: {error}"
    );

    let mut unapproved = catalogue;
    unapproved.scenarios[0].manifest = "fixtures/conformance/authored-core.scenario.toml".into();
    let error = validate_scenario_catalogue(&root, &unapproved)
        .err()
        .ok_or("unapproved fixture root must fail")?;
    assert!(
        error.contains("outside approved fixture roots"),
        "wrong root error: {error}"
    );
    Ok(())
}

#[test]
fn vendored_real_world_sources_match_the_reviewed_pinned_corpus() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let corpus: RealWorldCorpus = toml::from_str(&fs::read_to_string(root.join("fixtures/real-world/corpus.toml"))?)?;
    if corpus.schema != 1 || corpus.reviewed.trim().is_empty() {
        return Err("real-world corpus schema and review date must be explicit".into());
    }
    for project in &corpus.projects {
        if [&project.tier, &project.repository, &project.path]
            .into_iter()
            .any(|value| value.trim().is_empty())
            || project.minimum_services == 0
            || project.compose_fields.is_empty()
            || project.goals.is_empty()
        {
            return Err(format!("real-world corpus project {} is incomplete", project.id).into());
        }
    }
    let expected = corpus
        .projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();

    for registration in registered_scenarios(&root)? {
        let manifest = read_manifest_file(&registration.manifest)?;
        let Some(project_id) = manifest.provenance.corpus_project.as_deref() else {
            continue;
        };
        let project = corpus
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .ok_or("scenario references an unknown real-world corpus project")?;
        assert!(
            observed.insert(project_id.to_owned()),
            "duplicate real-world corpus scenario"
        );
        assert_eq!(
            manifest.application.revision.as_deref(),
            Some(project.revision.as_str())
        );
        assert_eq!(manifest.provenance.license, project.license);
        assert_eq!(manifest.native_inputs.len(), 1);
        assert_eq!(manifest.native_inputs[0].files.as_slice(), ["input.compose.yaml"]);

        let fixture = registration.manifest.parent().ok_or("scenario fixture directory")?;
        let source = fixture.join("input.compose.yaml");
        let output = Command::new("git")
            .args(["hash-object", "--no-filters"])
            .arg(&source)
            .current_dir(&root)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "git hash-object failed for {project_id}: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let observed_blob_sha = String::from_utf8(output.stdout)?;
        validate_real_world_provenance_link(&registration.id, &manifest, project, observed_blob_sha.trim())?;
        let license = fs::read_to_string(fixture.join("UPSTREAM-LICENSE"))?;
        assert!(!license.trim().is_empty(), "{project_id} upstream license is empty");
        let notice = fixture.join("UPSTREAM-NOTICE");
        if notice.exists() {
            assert!(
                !fs::read_to_string(notice)?.trim().is_empty(),
                "{project_id} upstream notice is empty"
            );
        }
    }

    assert_eq!(observed, expected);
    Ok(())
}

#[test]
fn authored_scenarios_execute_every_exporter_and_record_independent_evidence() -> Result<(), Box<dyn Error>> {
    let registrations = registered_scenarios(&repository_root())?;
    assert!(
        !registrations.is_empty(),
        "scenario catalogue cannot silently disappear"
    );
    for registration in registrations {
        let fixture = registration.manifest.parent().ok_or("scenario fixture directory")?;
        let manifest = read_manifest_file(&registration.manifest)?;
        if manifest.id != registration.id {
            return Err("scenario catalogue id differs from manifest id".into());
        }
        validate_manifest(&manifest, &registration.manifest)?;
        validate_exporter_coverage(&manifest, &capability_routes()?)?;
        for input in &manifest.native_inputs {
            let sources = input.files.iter().map(|file| fixture.join(file)).collect::<Vec<_>>();
            if input.outcome == Outcome::ExpectedRejection {
                for route in manifest.evidence.iter().filter(|route| route.input == input.id) {
                    run_import_rejection(&manifest, route, &sources, fixture).map_err(|error| {
                        format!(
                            "scenario {} input {} exporter {}: {error}",
                            manifest.id, input.id, route.exporter
                        )
                    })?;
                }
                continue;
            }
            let imported = import_native_input(&manifest, input, fixture)?;
            validate_no_error_diagnostics(imported.diagnostics())?;
            assert_no_protected_environment_values(
                &manifest,
                &format!("{:?}", imported.diagnostics()),
                "library diagnostics",
            )?;
            let expected_import_diagnostics =
                if input.import_diagnostics.is_none() && input.import_diagnostics_file.is_none() {
                    manifest.semantics.expected_diagnostics.clone()
                } else {
                    load_string_expectations(
                        input.import_diagnostics.as_deref().unwrap_or(&[]),
                        input.import_diagnostics_file.as_deref(),
                        fixture,
                        "input diagnostics",
                    )?
                };
            validate_diagnostics(&expected_import_diagnostics, &diagnostic_facts(imported.diagnostics()))?;
            validate_neutral_application(
                &manifest,
                imported.application().ok_or("missing imported Application")?,
                &manifest.semantics.required_environment_order,
                &[],
            )?;
            for route in manifest.evidence.iter().filter(|route| route.input == input.id) {
                run_route(&manifest, route, &sources, fixture, imported.clone()).map_err(|error| {
                    format!(
                        "scenario {} input {} exporter {}: {error}",
                        manifest.id, input.id, route.exporter
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn run_import_rejection(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    sources: &[PathBuf],
    fixture: &Path,
) -> Result<(), Box<dyn Error>> {
    let output = TemporaryDirectory::new(&format!("{}-import-rejection", route.exporter))?;
    let report = convert_cli(manifest, sources, route, output.path())?;
    assert_eq!(report["status"], "failure", "importer rejection status changed");
    let expected_diagnostics = load_string_expectations(
        &route.diagnostics,
        route.diagnostics_file.as_deref(),
        fixture,
        "import rejection diagnostics",
    )?;
    validate_diagnostics(&expected_diagnostics, &report_diagnostic_facts(&report)?)?;
    assert_eq!(
        fs::read_dir(output.path())?.count(),
        0,
        "importer rejection emitted artifacts"
    );
    validate_evidence(route, &rejected_observation())?;
    Ok(())
}

fn run_route(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    sources: &[PathBuf],
    fixture: &Path,
    imported: ImportResult,
) -> Result<(), Box<dyn Error>> {
    let expected_artifacts = load_string_expectations(
        &route.artifacts,
        route.artifacts_file.as_deref(),
        fixture,
        "route artifacts",
    )?;
    let expected_diagnostics = load_string_expectations(
        &route.diagnostics,
        route.diagnostics_file.as_deref(),
        fixture,
        "route diagnostics",
    )?;
    let expected_losses = load_loss_expectations(
        &route.allowed_losses,
        route.allowed_losses_file.as_deref(),
        fixture,
        "route losses",
    )?;
    let missing = observe_prerequisites(manifest, fixture)?;
    validate_diagnostics(&route.unavailable_prerequisites, &missing)?;
    if !missing.is_empty() {
        assert!(expected_artifacts.is_empty());
        validate_diagnostics(&expected_diagnostics, &[])?;
        validate_losses(&expected_losses, &[], &route.version_scope())?;
        validate_diagnostics(&route.semantic_gaps, &[])?;
        return Ok(validate_evidence(route, &unavailable_observation())?);
    }
    let target = route.target()?;
    match route.exporter.as_str() {
        "compose" => validate_result(
            route,
            fixture,
            &convert_imported(imported, &ComposeExporter::new()?, &target, route.policy()?)?,
        )?,
        "quadlet" => validate_result(
            route,
            fixture,
            &convert_imported(imported, &quadlet_exporter(route)?, &target, route.policy()?)?,
        )?,
        "podman" => validate_result(
            route,
            fixture,
            &convert_imported(imported, &PodmanExporter::new()?, &target, route.policy()?)?,
        )?,
        _ => return Err("registered exporter has no scenario runner".into()),
    }
    let output = TemporaryDirectory::new(&route.exporter)?;
    let report = convert_cli(manifest, sources, route, output.path())?;
    let diagnostics = report_diagnostic_facts(&report)?;
    validate_diagnostics(&expected_diagnostics, &diagnostics)?;
    let mut artifacts = fs::read_dir(output.path())?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("non-UTF-8 artifact"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    artifacts.sort();
    let mut expected = expected_artifacts;
    expected.sort();
    assert_eq!(artifacts, expected, "artifact inventory changed");
    if report["status"] != "success" {
        if route.outcome != Outcome::ExpectedRejection {
            return Err("CLI failure is not an expected rejection".into());
        }
        return Ok(validate_evidence(route, &rejected_observation())?);
    }

    let (semantics_match, reimport) = observe_native_output(manifest, route, &artifacts, output.path(), fixture)?;
    let observation = RouteObservation {
        blocked: report["status"] != "success",
        semantics_match,
        native_validation: Dimension::passed(),
        runtime_probe: Dimension::not_applicable(OFFLINE),
        reimport,
    };
    validate_evidence(route, &observation)?;
    Ok(())
}

fn report_diagnostic_facts(report: &serde_json::Value) -> Result<Vec<String>, Box<dyn Error>> {
    report["diagnostics"]
        .as_array()
        .ok_or("missing CLI diagnostics")?
        .iter()
        .map(|diagnostic| {
            let subject = diagnostic["fields"]
                .as_array()
                .and_then(|fields| fields.iter().find(|field| field["name"] == "subject"))
                .and_then(|field| field["value"].as_str())
                .unwrap_or("<global>");
            Ok(format!(
                "{}|{subject}",
                diagnostic["code"].as_str().ok_or("diagnostic missing code")?
            ))
        })
        .collect()
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps every native exporter observation in one transaction"
)]
fn observe_native_output(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    artifacts: &[String],
    output: &Path,
    fixture: &Path,
) -> Result<(bool, Dimension), Box<dyn Error>> {
    let expected_reimport_diagnostics = load_string_expectations(
        &route.reimport_diagnostics,
        route.reimport_diagnostics_file.as_deref(),
        fixture,
        "route reimport diagnostics",
    )?;
    let expected_reimport_losses = load_loss_expectations(
        &route.reimport_allowed_losses,
        route.reimport_allowed_losses_file.as_deref(),
        fixture,
        "route reimport losses",
    )?;
    Ok(match route.exporter.as_str() {
        "compose" => {
            let reimport = import_compose(
                &fs::read_to_string(output.join("compose.yaml"))?,
                &manifest.application.name,
            )?;
            validate_losses(
                &expected_reimport_losses,
                reimport.outcomes(),
                route
                    .reimport_version_scope
                    .as_deref()
                    .unwrap_or(&route.version_scope()),
            )?;
            validate_diagnostics(
                &expected_reimport_diagnostics,
                &diagnostic_facts(reimport.diagnostics()),
            )?;
            validate_neutral_application(
                manifest,
                reimport.application().ok_or("Compose reimport failed")?,
                &route.environment_order,
                &route.reimport_semantic_gaps,
            )?;
            let reimport = if route.reimport_semantic_gaps.is_empty() {
                Dimension::passed()
            } else {
                if route.reimport.state != DimensionState::KnownMigrationGap {
                    return Err("Compose reimport semantic gaps require reviewed known-gap evidence".into());
                }
                let reason = route
                    .reimport
                    .reason
                    .clone()
                    .ok_or("missing reviewed Compose reimport-gap reason")?;
                Dimension {
                    state: DimensionState::KnownMigrationGap,
                    reason: Some(reason),
                }
            };
            (route.semantic_gaps.is_empty(), reimport)
        }
        "quadlet" => {
            validate_quadlet_network_artifacts(manifest, output)?;
            let units = artifacts
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    Ok(QuadletDocumentInput::new(
                        name,
                        boxferry::quadlet::quadlet_lens::source::SourceId::new(u32::try_from(index)?),
                        fs::read_to_string(output.join(name))?,
                    ))
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            let parsed = QuadletSource::parse(Identifier::new(&manifest.application.name)?, units)?;
            assert!(parsed.diagnostics().is_empty(), "unexpected native Quadlet diagnostics");
            let reimport = QuadletImporter::new()?.import(parsed.source());
            validate_losses(
                &expected_reimport_losses,
                reimport.outcomes(),
                route
                    .reimport_version_scope
                    .as_deref()
                    .unwrap_or(&route.version_scope()),
            )?;
            validate_diagnostics(
                &expected_reimport_diagnostics,
                &diagnostic_facts(reimport.diagnostics()),
            )?;
            validate_neutral_application(
                manifest,
                reimport.application().ok_or("Quadlet reimport failed")?,
                &route.environment_order,
                &route.reimport_semantic_gaps,
            )?;
            let reimport = if route.reimport_semantic_gaps.is_empty() {
                Dimension::passed()
            } else {
                if route.reimport.state != DimensionState::KnownMigrationGap {
                    return Err("Quadlet reimport semantic gaps require reviewed known-gap evidence".into());
                }
                let reason = route
                    .reimport
                    .reason
                    .clone()
                    .ok_or("missing reviewed Quadlet reimport-gap reason")?;
                Dimension {
                    state: DimensionState::KnownMigrationGap,
                    reason: Some(reason),
                }
            };
            (route.semantic_gaps.is_empty(), reimport)
        }
        "podman" => {
            let native: serde_json::Value = serde_json::from_str(&fs::read_to_string(output.join("podman.json"))?)?;
            let observed_gaps = podman_semantics(manifest, route, &native)?;
            validate_diagnostics(&route.semantic_gaps, &observed_gaps)?;
            (observed_gaps.is_empty(), Dimension::not_applicable(NO_PODMAN_REIMPORT))
        }
        _ => return Err("missing native validator".into()),
    })
}

fn quadlet_exporter(route: &RouteExpectation) -> Result<QuadletExporter, Box<dyn Error>> {
    let grouping = match route.grouping.as_deref().unwrap_or("separate") {
        "separate" => QuadletGroupingPolicy::SeparateContainers,
        "pod" => QuadletGroupingPolicy::SinglePod,
        "preserve" => QuadletGroupingPolicy::PreserveSingleGroup,
        _ => return Err("unknown Quadlet grouping".into()),
    };
    let mut exporter = QuadletExporter::new()?.with_grouping_policy(grouping);
    if let Some(pod_name) = &route.pod_name {
        exporter = exporter.with_pod_name(pod_name);
    }
    Ok(exporter)
}

fn validate_quadlet_network_artifacts(manifest: &ScenarioManifest, output: &Path) -> Result<(), Box<dyn Error>> {
    for expected in &manifest.semantics.required_networks {
        let path = output.join(format!("{}.network", expected.name));
        if expected.ownership != "application" {
            if path.exists() {
                return Err(format!("non-application network {} emitted as an owned artifact", expected.name).into());
            }
            continue;
        }
        let source = fs::read_to_string(path)?;
        let lines = source.lines().collect::<Vec<_>>();
        for setting in [
            expected.internal.map(|value| format!("Internal={value}")),
            expected.ipv6.map(|value| format!("IPv6={value}")),
        ]
        .into_iter()
        .flatten()
        {
            let count = lines.iter().filter(|line| **line == setting).count();
            if count != 1 {
                return Err(format!(
                    "Quadlet network {} must contain exactly one {setting}, observed {count}",
                    expected.name
                )
                .into());
            }
        }

        let expected_ipam = expected
            .ipam
            .iter()
            .flatten()
            .flat_map(|row| {
                std::iter::once(format!("Subnet={}", row.subnet))
                    .chain(row.gateway.iter().map(|value| format!("Gateway={value}")))
                    .chain(row.ip_range.iter().map(|value| format!("IPRange={value}")))
            })
            .collect::<Vec<_>>();
        let actual_ipam = lines
            .iter()
            .filter(|line| line.starts_with("Subnet=") || line.starts_with("Gateway=") || line.starts_with("IPRange="))
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>();
        if actual_ipam != expected_ipam {
            return Err(format!(
                "Quadlet network {} IPAM order differs: expected {expected_ipam:?}, observed {actual_ipam:?}",
                expected.name
            )
            .into());
        }
    }
    Ok(())
}

const NOT_EMITTED: &str = "Rejected before artifact emission; no native artifact was emitted.";
const MISSING_PREREQUISITE: &str = "An explicitly required fixture-relative file is unavailable.";

fn rejected_observation() -> RouteObservation {
    RouteObservation {
        blocked: true,
        semantics_match: false,
        native_validation: Dimension::not_applicable(NOT_EMITTED),
        runtime_probe: Dimension::not_applicable(OFFLINE),
        reimport: Dimension::not_applicable(NOT_EMITTED),
    }
}
fn unavailable_observation() -> RouteObservation {
    RouteObservation {
        blocked: true,
        semantics_match: false,
        native_validation: Dimension::not_applicable(MISSING_PREREQUISITE),
        runtime_probe: Dimension {
            state: DimensionState::UnsupportedEnvironment,
            reason: Some(MISSING_PREREQUISITE.into()),
        },
        reimport: Dimension::not_applicable(MISSING_PREREQUISITE),
    }
}

fn validate_result<T>(route: &RouteExpectation, fixture: &Path, result: &ConversionResult<T>) -> Result<(), String> {
    if result.is_blocked() != (route.outcome == Outcome::ExpectedRejection) {
        return Err("actual policy authorization differs from expected route outcome".into());
    }
    if route.outcome != Outcome::ExpectedRejection {
        validate_no_error_diagnostics(result.diagnostics())?;
        if result
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
        {
            return Err("successful or known-gap route contains an invalid conversion outcome".into());
        }
    }
    let expected_losses = load_loss_expectations(
        &route.allowed_losses,
        route.allowed_losses_file.as_deref(),
        fixture,
        "route losses",
    )?;
    validate_losses(&expected_losses, result.outcomes(), &route.version_scope())
}

fn validate_no_error_diagnostics(diagnostics: &[boxferry::Diagnostic]) -> Result<(), String> {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity() == boxferry::Severity::Error)
        .map(|diagnostic| diagnostic.code().as_str())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("non-rejection path contains error diagnostics: {errors:?}"))
    }
}

/// Validate the native JSON shape and independent desired create operations. A
/// deployment artifact cannot be passed to the inventory importer as a round trip.
#[allow(clippy::too_many_lines, reason = "keeps Podman CLI and API plan evidence together")]
fn podman_semantics(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    value: &serde_json::Value,
) -> Result<Vec<String>, Box<dyn Error>> {
    assert_eq!(value["schema_version"], 1);
    let operations = value["operations"].as_array().ok_or("native deployment operations")?;
    let creates = operations
        .iter()
        .filter(|operation| operation["action"] == "create")
        .collect::<Vec<_>>();
    if let Some(expected) = &route.podman_plan {
        validate_podman_plan_observation(expected, value, operations)?;
    } else {
        let actual = creates
            .iter()
            .map(|operation| {
                let kind = match operation["resource"]["kind"].as_str().ok_or("resource kind")? {
                    "container" => "service",
                    "volume" => "volume",
                    "network" => "network",
                    _ => return Err("unreviewed Podman resource kind".into()),
                };
                Ok(format!(
                    "{kind}:{}",
                    operation["resource"]["name"].as_str().ok_or("resource name")?
                ))
            })
            .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
        assert_eq!(actual, manifest.semantics.selected_resources.iter().cloned().collect());
    }
    for operation in operations {
        assert_eq!(operation["cli"]["program"], "podman");
        assert!(
            operation["cli"]["argv"]
                .as_array()
                .ok_or("native argv")?
                .iter()
                .all(serde_json::Value::is_string)
        );
        assert!(operation["libpod"]["path_and_query"].is_string());
    }
    let mut gaps = Vec::new();
    for component in &manifest.components {
        let create = creates
            .iter()
            .find(|operation| {
                operation["resource"]["kind"] == "container" && operation["resource"]["name"] == component.name
            })
            .ok_or("missing container create")?;
        let argv = create["cli"]["argv"].as_array().ok_or("create argv")?;
        let body = &create["libpod"]["body"]["json"];
        let digest_reference = component
            .image
            .as_ref()
            .zip(component.digest.as_ref())
            .map(|(image, digest)| format!("{image}\u{40}{digest}"));
        let image = component
            .configured_image
            .as_deref()
            .or(digest_reference.as_deref())
            .ok_or("component image expectation")?;
        let image_index = argv
            .iter()
            .position(|argument| argument.as_str() == Some(image))
            .ok_or("container create command lacks the reviewed image")?;
        assert_eq!(body["image"], image);
        let options = &argv[..image_index];
        let command = argv[image_index + 1..]
            .iter()
            .map(|argument| argument.as_str().ok_or("container command argument"))
            .collect::<Result<Vec<_>, _>>()?;
        let api_command = body
            .get("command")
            .map(|value| value.as_array().ok_or("native command must be an array"))
            .transpose()?
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_str().ok_or("native command argument"))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        assert_eq!(command, api_command, "CLI/API command evidence diverges");
        for expected in manifest
            .semantics
            .required_commands
            .iter()
            .filter(|expected| expected.service == component.name)
        {
            let matches = match expected.kind.as_str() {
                "exec" => command == expected.values.iter().map(String::as_str).collect::<Vec<_>>(),
                "absent" => command.is_empty(),
                _ => return Err("unvalidated Podman command expectation kind".into()),
            };
            if !matches {
                gaps.push(format!("command:{}", component.name));
            }
        }
        let expected_entrypoint = manifest
            .semantics
            .required_entrypoints
            .iter()
            .find(|expected| expected.service == component.name)
            .map(|expected| (expected.kind.as_str(), expected.values.as_slice()));
        if let Some(gap) = podman_entrypoint_gap(&component.name, expected_entrypoint, options, body)? {
            gaps.push(gap);
        }
        for requirement in &manifest.semantics.required_runtime_names {
            let (service, expected) = requirement.split_once(':').ok_or("runtime-name grammar")?;
            if service == component.name && option_values(options, &["--name"]).as_slice() != [expected] {
                gaps.push(format!("runtime-name:{service}"));
            }
        }
        gaps.extend(podman_environment_and_ports(
            manifest,
            route,
            &component.name,
            options,
            body,
        )?);
        gaps.extend(podman_structured_port_gaps(manifest, &component.name, options, body)?);
        gaps.extend(validate_podman_mounts(manifest, &component.name, options, body)?);
        gaps.extend(podman_structured_volume_mount_gaps(
            manifest,
            &component.name,
            options,
            body,
        )?);
    }
    for expected in &manifest.semantics.required_config_grants {
        gaps.push(format!("config-grant:{}:{}", expected.service, expected.source));
    }
    for expected in &manifest.semantics.required_secret_grants {
        gaps.push(format!("secret-grant:{}:{}", expected.service, expected.source));
    }
    for expected in &manifest.semantics.required_dependencies {
        gaps.push(format!("dependency:{}:{}", expected.service, expected.dependency));
    }
    for expected in &manifest.semantics.required_healthchecks {
        gaps.push(format!("healthcheck:{}", expected.service));
    }
    gaps.extend(podman_network_gaps(manifest, &creates)?);
    Ok(gaps)
}

fn validate_podman_plan_observation(
    expected: &support::PodmanPlanExpectation,
    document: &serde_json::Value,
    operations: &[serde_json::Value],
) -> Result<(), Box<dyn Error>> {
    assert!(
        document["connection"].is_null(),
        "scenario plan must not target a live connection"
    );
    let actual_creates = operations
        .iter()
        .filter(|operation| operation["action"] == "create")
        .map(|operation| podman_resource_id(&operation["resource"]))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_creates = expected.creatable_resources.iter().cloned().collect::<BTreeSet<_>>();
    if actual_creates.len() != expected_creates.len()
        || actual_creates.iter().cloned().collect::<BTreeSet<_>>() != expected_creates
    {
        return Err("Podman creatable resources differ from the reviewed plan contract".into());
    }

    let preconditions = document["external_preconditions"]
        .as_array()
        .ok_or("native external preconditions")?;
    let actual_preconditions = preconditions
        .iter()
        .map(podman_resource_id)
        .collect::<Result<Vec<_>, _>>()?;
    let expected_preconditions = expected.external_preconditions.iter().cloned().collect::<BTreeSet<_>>();
    if actual_preconditions.len() != expected_preconditions.len()
        || actual_preconditions.iter().cloned().collect::<BTreeSet<_>>() != expected_preconditions
    {
        return Err("Podman external preconditions differ from the reviewed plan contract".into());
    }

    let image_operations = operations
        .iter()
        .filter(|operation| operation["action"] == "ensure_image")
        .collect::<Vec<_>>();
    if image_operations.len() != expected.image_operations.len() {
        return Err("Podman image operation count changed".into());
    }
    for (operation, expected) in image_operations.iter().zip(&expected.image_operations) {
        assert_eq!(
            podman_resource_id(&operation["resource"])?,
            format!("image:{}", expected.resource)
        );
        assert_eq!(
            operation["cli"]["argv"],
            serde_json::json!([
                "image",
                "pull",
                format!("--policy={}", expected.policy),
                expected.reference
            ])
        );
    }

    let actual_start_order = operations
        .iter()
        .filter(|operation| operation["action"] == "start_container")
        .map(|operation| {
            let resource = podman_resource_id(&operation["resource"])?;
            let name = resource
                .strip_prefix("container:")
                .ok_or("start operation resource kind")?;
            assert_eq!(
                operation["cli"]["argv"],
                serde_json::json!(["container", "start", name])
            );
            Ok(name.to_owned())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    if actual_start_order != expected.start_order {
        return Err("Podman start order differs from reviewed plan contract".into());
    }

    if operations.len() != actual_creates.len() + image_operations.len() + actual_start_order.len()
        || operations.iter().any(|operation| {
            !matches!(
                operation["action"].as_str(),
                Some("create" | "ensure_image" | "start_container")
            )
        })
    {
        return Err("Podman plan contains an unreviewed operation".into());
    }
    Ok(())
}

fn podman_resource_id(resource: &serde_json::Value) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "{}:{}",
        resource["kind"].as_str().ok_or("resource kind")?,
        resource["name"].as_str().ok_or("resource name")?
    ))
}

fn option_values<'a>(argv: &'a [serde_json::Value], names: &[&str]) -> Vec<&'a str> {
    argv.iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let argument = argument.as_str()?;
            for name in names {
                if argument == *name {
                    return argv.get(index + 1).and_then(serde_json::Value::as_str);
                }
                if let Some(value) = argument.strip_prefix(name).and_then(|suffix| suffix.strip_prefix('=')) {
                    return Some(value);
                }
            }
            None
        })
        .collect()
}

fn podman_entrypoint_gap(
    service: &str,
    expected: Option<(&str, &[String])>,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<Option<String>, Box<dyn Error>> {
    let entrypoint_options = option_values(options, &["--entrypoint"]);
    let cli_entrypoint = match entrypoint_options.as_slice() {
        [] => None,
        [value] => Some(serde_json::from_str::<Vec<String>>(value)?),
        _ => return Err("native entrypoint option must occur at most once".into()),
    };
    let api_entrypoint = body
        .get("entrypoint")
        .map(|value| serde_json::from_value::<Vec<String>>(value.clone()))
        .transpose()?;
    if cli_entrypoint != api_entrypoint {
        return Err("CLI/API entrypoint evidence diverges".into());
    }
    let Some((kind, values)) = expected else {
        return Ok(None);
    };
    let expected_entrypoint = match kind {
        "exec" => Some(values.to_vec()),
        "shell" if values.len() == 1 => Some(vec!["/bin/sh".to_owned(), "-c".to_owned(), values[0].clone()]),
        "empty" if values.is_empty() => Some(Vec::new()),
        "absent" if values.is_empty() => None,
        _ => return Err("unvalidated Podman entrypoint expectation kind".into()),
    };
    Ok((cli_entrypoint.as_ref() != expected_entrypoint.as_ref()).then(|| format!("entrypoint:{service}")))
}

fn podman_environment_and_ports(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    name: &str,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<Vec<String>, Box<dyn Error>> {
    let environment = option_values(options, &["--env", "-e"]);
    let publications = option_values(options, &["--publish", "-p"]);
    let api_environment = body
        .get("env")
        .map(|value| value.as_object().ok_or("native env must be an object"))
        .transpose()?;
    let api_ports = body
        .get("portmappings")
        .map(|value| value.as_array().ok_or("native portmappings must be an array"))
        .transpose()?;
    let expected_order = route
        .environment_order
        .iter()
        .filter_map(|requirement| requirement.strip_prefix(&format!("{name}:")))
        .collect::<Vec<_>>();
    if environment != expected_order {
        return Err(format!("Podman CLI environment order changed for {name}").into());
    }
    if let Some(values) = api_environment {
        let expected_keys = expected_order
            .iter()
            .map(|assignment| assignment.split_once('=').map(|(key, _)| key))
            .collect::<Option<Vec<_>>>()
            .ok_or("ordered environment assignment")?;
        if values.keys().map(String::as_str).collect::<Vec<_>>() != expected_keys {
            return Err(format!("Podman API environment order changed for {name}").into());
        }
    }
    let mut gaps = Vec::new();
    for requirement in &manifest.semantics.required_environment {
        let (owner, assignment) = requirement.split_once(':').ok_or("environment grammar")?;
        if owner != name {
            continue;
        }
        let (key, value) = assignment.split_once('=').ok_or("assignment grammar")?;
        let cli_has = environment.contains(&assignment);
        let api_has = api_environment
            .and_then(|values| values.get(key))
            .is_some_and(|entry| entry == value);
        assert_eq!(cli_has, api_has, "CLI/API environment evidence diverges");
        if !cli_has {
            gaps.push(format!("environment:{name}:{key}"));
        }
    }
    for requirement in &manifest.semantics.required_ports {
        let (owner, port) = requirement.split_once(':').ok_or("port grammar")?;
        if owner != name {
            continue;
        }
        let (host, container_protocol) = port.split_once(':').ok_or("port grammar")?;
        let (container, protocol) = container_protocol.split_once('/').ok_or("port grammar")?;
        let host = host.parse::<u64>()?;
        let container = container.parse::<u64>()?;
        let cli_has = publications
            .iter()
            .any(|entry| *entry == port || *entry == port.trim_end_matches("/tcp"));
        let api_has = api_ports.is_some_and(|ports| {
            ports.iter().any(|entry| {
                entry["host_port"] == host && entry["container_port"] == container && entry["protocol"] == protocol
            })
        });
        assert_eq!(cli_has, api_has, "CLI/API publication evidence diverges");
        if !cli_has {
            gaps.push(format!("publication:{name}:{port}"));
        }
    }
    if manifest
        .semantics
        .unpublished_services
        .iter()
        .any(|service| service == name)
    {
        assert!(
            publications.is_empty() && api_ports.is_none_or(Vec::is_empty),
            "database publication is forbidden in both artifacts"
        );
    }
    Ok(gaps)
}

fn podman_structured_port_gaps(
    manifest: &ScenarioManifest,
    name: &str,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<Vec<String>, Box<dyn Error>> {
    let publications = option_values(options, &["--publish", "-p"]);
    let api_ports = body
        .get("portmappings")
        .map(|value| value.as_array().ok_or("native portmappings must be an array"))
        .transpose()?;
    let mut gaps = Vec::new();
    for expected in manifest
        .semantics
        .required_port_bindings
        .iter()
        .filter(|expected| expected.service == name)
    {
        let port = match (expected.host_address.as_deref(), expected.published) {
            (Some(host), Some(published)) if host.contains(':') => {
                format!("[{host}]:{published}:{}/{}", expected.container, expected.protocol)
            }
            (Some(host), Some(published)) => {
                format!("{host}:{published}:{}/{}", expected.container, expected.protocol)
            }
            (None, Some(published)) => {
                format!("{published}:{}/{}", expected.container, expected.protocol)
            }
            (None, None) => format!("{}/{}", expected.container, expected.protocol),
            (Some(_), None) => return Err("host address requires a published port".into()),
        };
        let cli_has = publications.contains(&port.as_str())
            || (expected.protocol == "tcp" && publications.contains(&port.trim_end_matches("/tcp")));
        let api_has = api_ports.is_some_and(|ports| {
            ports.iter().any(|entry| {
                entry["host_port"].as_u64() == expected.published.map(u64::from)
                    && entry["container_port"].as_u64() == Some(u64::from(expected.container))
                    && entry["host_ip"].as_str() == expected.host_address.as_deref()
                    && entry["protocol"].as_str() == Some(expected.protocol.as_str())
            })
        });
        assert_eq!(cli_has, api_has, "CLI/API structured publication evidence diverges");
        if !cli_has {
            gaps.push(format!(
                "port:{}:{}:{:?}",
                expected.service, expected.container, expected.published
            ));
        }
    }
    Ok(gaps)
}

fn podman_structured_volume_mount_gaps(
    manifest: &ScenarioManifest,
    name: &str,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<Vec<String>, Box<dyn Error>> {
    let volumes = option_values(options, &["--volume", "-v"]);
    let native = body
        .get("volumes")
        .map(|value| value.as_array().ok_or("native named volumes must be an array"))
        .transpose()?;
    let mut gaps = Vec::new();
    for expected in manifest
        .semantics
        .required_volume_mounts
        .iter()
        .filter(|expected| expected.service == name)
    {
        let access = if expected.read_only { "ro" } else { "rw" };
        let mut mount_options = vec![access.to_owned(), "copy".to_owned()];
        match expected.selinux_relabel.as_deref() {
            None => {}
            Some("shared") => mount_options.push("z".into()),
            Some("private") => mount_options.push("Z".into()),
            Some(_) => return Err("unvalidated named-volume SELinux relabel".into()),
        }
        let spelling = format!("{}:{}:{}", expected.source, expected.target, mount_options.join(","));
        let cli_has = volumes.contains(&spelling.as_str());
        let api_has = native.is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["Name"] == expected.source
                    && entry["Dest"] == expected.target
                    && entry["Options"].as_array().is_some_and(|options| {
                        options
                            .iter()
                            .map(serde_json::Value::as_str)
                            .eq(mount_options.iter().map(|value| Some(value.as_str())))
                    })
            })
        });
        assert_eq!(cli_has, api_has, "CLI/API structured volume evidence diverges");
        if !cli_has {
            gaps.push(format!("volume-mount:{name}:{}", expected.target));
        }
    }
    Ok(gaps)
}

fn validate_podman_mounts(
    manifest: &ScenarioManifest,
    name: &str,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<Vec<String>, Box<dyn Error>> {
    let volumes = option_values(options, &["--volume", "-v"]);
    for requirement in &manifest.semantics.required_mounts {
        let (owner, rest) = requirement.split_once(':').ok_or("mount grammar")?;
        if owner != name {
            continue;
        }
        let (volume, destination) = rest.split_once(':').ok_or("mount grammar")?;
        let expected = format!("{volume}:{destination}:rw,copy");
        assert!(
            volumes.contains(&expected.as_str()),
            "required CLI volume option changed"
        );
        let native = body["volumes"].as_array().ok_or("native named volumes")?;
        assert!(
            native.iter().any(|entry| entry["Name"] == volume
                && entry["Dest"] == destination
                && entry["Options"] == serde_json::json!(["rw", "copy"])),
            "required native API volume changed"
        );
    }
    let mut gaps = Vec::new();
    for expected in manifest
        .semantics
        .required_bind_mounts
        .iter()
        .filter(|expected| expected.service == name)
    {
        let access = if expected.read_only { "ro" } else { "rw" };
        let relabel = match expected.selinux_relabel.as_str() {
            "shared" => "z",
            "private" => "Z",
            _ => return Err("unvalidated bind-mount relabel".into()),
        };
        let spelling = format!("{}:{}:{access},{relabel}", expected.source, expected.target);
        let cli_has = volumes.contains(&spelling.as_str());
        let body_text = body.to_string();
        let api_has = body_text.contains(&expected.source) && body_text.contains(&expected.target);
        assert_eq!(cli_has, api_has, "CLI/API bind-mount evidence diverges");
        if !cli_has {
            gaps.push(format!("bind-mount:{name}:{}", expected.target));
        }
    }
    Ok(gaps)
}

fn podman_network_gaps(
    manifest: &ScenarioManifest,
    creates: &[&serde_json::Value],
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_networks {
        let create = creates.iter().find(|operation| {
            operation["resource"]["kind"] == "network" && operation["resource"]["name"] == expected.name
        });
        if expected.ownership != "application" {
            if create.is_some() {
                return Err(format!("non-application network {} was created", expected.name).into());
            }
            continue;
        }
        let create = create.ok_or("missing application-owned network create")?;
        let argv = create["cli"]["argv"].as_array().ok_or("network create argv")?;
        let body = &create["libpod"]["body"]["json"];
        for (field, required, cli_has, api_has) in [
            (
                "internal",
                expected.internal.is_some(),
                argv.iter().any(|argument| argument == "--internal"),
                body.get("internal").is_some(),
            ),
            (
                "ipv6",
                expected.ipv6.is_some(),
                argv.iter().any(|argument| argument == "--ipv6"),
                body.get("ipv6_enabled").is_some(),
            ),
            (
                "ipam",
                expected.ipam.is_some(),
                argv.iter()
                    .any(|argument| matches!(argument.as_str(), Some("--subnet" | "--gateway" | "--ip-range"))),
                body.get("subnets").is_some(),
            ),
        ] {
            if cli_has != api_has {
                return Err(format!("Podman CLI/API network {field} evidence diverges for {}", expected.name).into());
            }
            if required && !cli_has {
                gaps.push(format!("network-{field}:{}", expected.name));
            }
        }
    }
    Ok(gaps)
}

#[test]
fn unavailable_typed_network_values_have_exact_actionable_import_evidence() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/podman-portable-intent");
    let manifest = read_manifest(&fixture)?;
    let mut cassette = PodmanCassette::load(&fixture.join("input-podman.cassette.json"))?;
    let network_path = "/v6.1.0/libpod/networks/n-portable/json";
    cassette.insert_body_field(network_path, "driver", serde_json::json!("bridge"))?;
    cassette.insert_body_field(network_path, "ipv6_enabled", serde_json::json!(true))?;
    cassette.insert_body_field(
        network_path,
        "ipam_options",
        serde_json::json!({"driver": "host-local"}),
    )?;
    let imported = import_podman_cassette(&manifest, "podman", cassette)?;
    let application = imported.application().ok_or("mutated Podman application")?;
    let network = application
        .networks()
        .iter()
        .find(|network| network.value().name().as_str() == "scenario-net")
        .ok_or("mutated scenario network")?
        .value();
    assert_eq!(
        network.ipv6().map(|value| *value.value()),
        Some(true),
        "typed IPv6 subnet must retain neutral IPv6 intent"
    );
    let remediation = "Author this network field separately on the target; no BoxFerry promotion option can recover it from current PodmanLens evidence.";
    let expected = [
        "networks.scenario-net.driver",
        "networks.scenario-net.ipam_driver",
        "networks.scenario-net.native_ipv6_enabled",
    ];
    for subject in expected {
        let diagnostic = imported
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code().as_str() == "BFP0002"
                    && diagnostic
                        .fields()
                        .iter()
                        .any(|field| field.name() == "subject" && field.value().redacted() == subject)
            })
            .ok_or_else(|| format!("missing diagnostic for {subject}"))?;
        for (name, value) in [
            ("decision", "omitted"),
            ("available_promotion", "none"),
            ("remediation", remediation),
        ] {
            assert!(
                diagnostic
                    .fields()
                    .iter()
                    .any(|field| { field.name() == name && field.value().redacted() == value }),
                "{subject} diagnostic field {name} changed"
            );
        }
        assert!(
            imported.outcomes().iter().any(|outcome| {
                outcome.subject() == subject
                    && outcome.kind() == ConversionKind::Unsupported
                    && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFP0002")
            }),
            "{subject} unsupported loss changed"
        );
    }
    assert_no_protected_environment_values(
        &manifest,
        &format!("{:?}", imported.diagnostics()),
        "mutated-cassette diagnostics",
    )?;
    Ok(())
}

#[test]
fn policy_rejection_and_unavailable_environment_are_executed_not_claimed() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let source = fixture.join("input-compose.yaml");
    let mut manifest = read_manifest(&fixture)?;
    for route in &mut manifest.evidence {
        route.loss_policy = "exact".into();
        if route.exporter == "podman" {
            route.outcome = Outcome::ExpectedRejection;
            route.reason = Some(NOT_EMITTED.into());
            route.artifacts.clear();
            route.semantic_gaps.clear();
            route.native_validation = Dimension::not_applicable(NOT_EMITTED);
            route.reimport = Dimension::not_applicable(NOT_EMITTED);
        }
    }
    validate_manifest(&manifest, &fixture)?;
    validate_exporter_coverage(&manifest, &capability_routes()?)?;
    let imported = import_compose(&fs::read_to_string(&source)?, &manifest.application.name)?;
    for route in &manifest.evidence {
        run_route(
            &manifest,
            route,
            std::slice::from_ref(&source),
            &fixture,
            imported.clone(),
        )?;
    }

    let mut manifest = read_manifest(&fixture)?;
    let prerequisite = "file:intentionally-absent-runtime-contract";
    assert!(!fixture.join("intentionally-absent-runtime-contract").exists());
    manifest.semantics.external_prerequisites = vec![prerequisite.into()];
    for route in &mut manifest.evidence {
        route.outcome = Outcome::UnsupportedEnvironment;
        route.reason = Some(MISSING_PREREQUISITE.into());
        route.artifacts.clear();
        route.diagnostics.clear();
        route.allowed_losses.clear();
        route.semantic_gaps.clear();
        route.unavailable_prerequisites = vec![prerequisite.into()];
        route.native_validation = Dimension::not_applicable(MISSING_PREREQUISITE);
        route.runtime_probe = Dimension {
            state: DimensionState::UnsupportedEnvironment,
            reason: Some(MISSING_PREREQUISITE.into()),
        };
        route.reimport = Dimension::not_applicable(MISSING_PREREQUISITE);
    }
    validate_manifest(&manifest, &fixture)?;
    validate_exporter_coverage(&manifest, &capability_routes()?)?;
    for route in &manifest.evidence {
        run_route(
            &manifest,
            route,
            std::slice::from_ref(&source),
            &fixture,
            imported.clone(),
        )?;
    }
    Ok(())
}

#[test]
fn native_assertions_associate_values_with_their_flags() -> Result<(), Box<dyn Error>> {
    let arguments = serde_json::json!([
        "--label",
        "APP_MODE=migration",
        "--env=OTHER=value",
        "--publish",
        "8080:8080",
        "--volume",
        "state:/data:rw,copy"
    ]);
    let arguments = arguments.as_array().ok_or("native argument fixture must be an array")?;
    assert_eq!(option_values(arguments, &["--env", "-e"]), ["OTHER=value"]);
    assert_eq!(option_values(arguments, &["--publish", "-p"]), ["8080:8080"]);
    assert_eq!(option_values(arguments, &["--volume", "-v"]), ["state:/data:rw,copy"]);
    Ok(())
}

#[test]
fn podman_plan_counterfactuals_fail_for_their_own_reason() -> Result<(), Box<dyn Error>> {
    let document = serde_json::json!({
        "connection": null,
        "external_preconditions": [
            { "kind": "network", "name": "app-net" }
        ],
        "operations": [
            { "action": "create", "resource": { "kind": "network", "name": "data-net" } },
            { "action": "create", "resource": { "kind": "container", "name": "api" } },
            {
                "action": "start_container",
                "resource": { "kind": "container", "name": "api" },
                "cli": { "argv": ["container", "start", "api"] }
            }
        ]
    });
    let operations = document["operations"].as_array().ok_or("Podman operations")?;
    let expectation = || support::PodmanPlanExpectation {
        creatable_resources: vec!["network:data-net".into(), "container:api".into()],
        external_preconditions: vec!["network:app-net".into()],
        image_operations: Vec::new(),
        start_order: vec!["api".into()],
    };

    validate_podman_plan_observation(&expectation(), &document, operations)?;

    let mut changed_creates = expectation();
    changed_creates.creatable_resources.pop();
    let mut changed_preconditions = expectation();
    changed_preconditions.external_preconditions.clear();
    let mut changed_start_order = expectation();
    changed_start_order.start_order[0] = "worker".into();

    for (name, changed, message) in [
        ("create", changed_creates, "creatable resources differ"),
        ("precondition", changed_preconditions, "external preconditions differ"),
        ("start order", changed_start_order, "start order differs"),
    ] {
        let error = validate_podman_plan_observation(&changed, &document, operations)
            .err()
            .ok_or_else(|| format!("mutated Podman {name} expectation must fail"))?;
        assert!(
            error.to_string().contains(message),
            "mutated Podman {name} expectation failed for the wrong reason: {error}"
        );
    }
    Ok(())
}

#[test]
fn podman_plan_schema_rejects_overlap_and_invalid_start_coverage() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/differential/podman-lens-complex-corpus");
    let baseline = read_manifest(&fixture)?;
    validate_manifest(&baseline, &fixture)?;

    let mut overlapping = read_manifest(&fixture)?;
    let plan = overlapping
        .evidence
        .iter_mut()
        .find_map(|route| route.podman_plan.as_mut())
        .ok_or("Podman plan expectation")?;
    let overlap = plan.creatable_resources.first().ok_or("creatable resource")?.clone();
    plan.external_preconditions.push(overlap);
    let error = validate_manifest(&overlapping, &fixture)
        .err()
        .ok_or("overlapping Podman plan resources must fail")?;
    assert!(
        error.contains("creatable resources and external preconditions must be disjoint"),
        "overlapping resources failed for the wrong reason: {error}"
    );

    let mut incomplete = read_manifest(&fixture)?;
    let plan = incomplete
        .evidence
        .iter_mut()
        .find_map(|route| route.podman_plan.as_mut())
        .ok_or("Podman plan expectation")?;
    let omitted = plan.start_order.pop().ok_or("start-order entry")?;
    assert!(
        plan.creatable_resources.contains(&format!("container:{omitted}")),
        "incomplete counterfactual must omit a creatable container"
    );
    let error = validate_manifest(&incomplete, &fixture)
        .err()
        .ok_or("incomplete Podman start order must fail")?;
    assert!(
        error.contains("start order must cover exactly the creatable containers"),
        "incomplete start order failed for the wrong reason: {error}"
    );

    let mut foreign = read_manifest(&fixture)?;
    let plan = foreign
        .evidence
        .iter_mut()
        .find_map(|route| route.podman_plan.as_mut())
        .ok_or("Podman plan expectation")?;
    *plan.start_order.first_mut().ok_or("start-order entry")? = "foreign".into();
    assert!(!plan.creatable_resources.contains(&"container:foreign".into()));
    let error = validate_manifest(&foreign, &fixture)
        .err()
        .ok_or("foreign Podman start-order entry must fail")?;
    assert!(
        error.contains("start order must cover exactly the creatable containers"),
        "foreign start-order entry failed for the wrong reason: {error}"
    );
    Ok(())
}

#[test]
fn podman_entrypoint_counterfactuals_detect_divergence_and_semantic_gap() -> Result<(), Box<dyn Error>> {
    let options = serde_json::json!(["--entrypoint", r#"["/usr/bin/complex-api"]"#]);
    let options = options.as_array().ok_or("entrypoint options")?;
    let body = serde_json::json!({ "entrypoint": ["/usr/bin/complex-api"] });
    let expected = vec!["/usr/bin/complex-api".to_owned()];
    assert_eq!(
        podman_entrypoint_gap("api", Some(("exec", &expected)), options, &body)?,
        None
    );

    let api_mismatch = serde_json::json!({ "entrypoint": ["/usr/bin/other"] });
    let error = podman_entrypoint_gap("api", Some(("exec", &expected)), options, &api_mismatch)
        .err()
        .ok_or("CLI/API entrypoint mismatch must fail")?;
    assert!(
        error.to_string().contains("CLI/API entrypoint evidence diverges"),
        "entrypoint mismatch failed for the wrong reason: {error}"
    );

    let changed_expectation = vec!["/usr/bin/other".to_owned()];
    assert_eq!(
        podman_entrypoint_gap("api", Some(("exec", &changed_expectation)), options, &body,)?,
        Some("entrypoint:api".into())
    );

    let empty_options = serde_json::json!(["--entrypoint", "[]"]);
    let empty_options = empty_options.as_array().ok_or("empty entrypoint options")?;
    let empty_body = serde_json::json!({ "entrypoint": [] });
    assert_eq!(
        podman_entrypoint_gap("api", Some(("empty", &[])), empty_options, &empty_body)?,
        None
    );
    assert_eq!(
        podman_entrypoint_gap("api", Some(("empty", &[])), &[], &serde_json::json!({}))?,
        Some("entrypoint:api".into())
    );
    Ok(())
}

#[test]
fn podman_input_selection_requires_all_xor_exact_selectors() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/differential/podman-lens-complex-corpus");
    let source = fs::read_to_string(fixture.join("scenario.toml"))?;
    let baseline: ScenarioManifest = toml::from_str(&source)?;
    validate_manifest(&baseline, &fixture)?;

    let both_source = source.replacen(
        "selectors = []",
        "selectors = [{ kind = \"container\", exact = \"api\" }]",
        1,
    );
    assert_ne!(both_source, source, "both-mode counterfactual must mutate the manifest");
    let both: ScenarioManifest = toml::from_str(&both_source)?;
    let error = validate_manifest(&both, &fixture)
        .err()
        .ok_or("Podman all plus exact selectors must fail")?;
    assert!(
        error.contains("select exactly one of all resources or exact selectors"),
        "all-plus-selectors failed for the wrong reason: {error}"
    );

    let neither_source = source.replacen("all = true", "all = false", 1);
    assert_ne!(
        neither_source, source,
        "neither-mode counterfactual must mutate the manifest"
    );
    let neither: ScenarioManifest = toml::from_str(&neither_source)?;
    let error = validate_manifest(&neither, &fixture)
        .err()
        .ok_or("Podman neither all nor exact selectors must fail")?;
    assert!(
        error.contains("select exactly one of all resources or exact selectors"),
        "empty selection failed for the wrong reason: {error}"
    );
    Ok(())
}

#[test]
fn sidecar_expectations_reject_mixing_and_uncontained_paths() -> Result<(), Box<dyn Error>> {
    let directory = TemporaryDirectory::new("sidecar-counterfactual")?;
    fs::write(directory.path().join("values.txt"), "expected\n")?;
    fs::write(
        directory.path().join("losses.tsv"),
        "BFP0007\tservices.app.mounts\tunsupported\tpodman@5.4.0..6.1.0\t1\n",
    )?;

    let inline = vec!["inline".to_owned()];
    let error = load_string_expectations(&inline, Some("values.txt"), directory.path(), "values")
        .err()
        .ok_or("inline and string sidecar must not mix")?;
    assert!(error.contains("cannot mix inline and sidecar"));

    let inline_loss = support::LossTuple {
        rule: "BFP0007".into(),
        subject: "services.app.mounts".into(),
        decision: "unsupported".into(),
        version_scope: "podman@5.4.0..6.1.0".into(),
        count: 1,
    };
    let error = load_loss_expectations(&[inline_loss], Some("losses.tsv"), directory.path(), "losses")
        .err()
        .ok_or("inline and loss sidecar must not mix")?;
    assert!(error.contains("cannot mix inline and sidecar"));

    for path in ["../outside", "/absolute", "missing.expectations"] {
        assert!(
            load_string_expectations(&[], Some(path), directory.path(), "unsafe sidecar").is_err(),
            "unsafe or missing sidecar {path} must fail"
        );
    }
    Ok(())
}

#[test]
fn exporter_coverage_rejects_unknown_and_per_input_drift() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let registered = capability_routes()?;

    let mut unknown_exporter = read_manifest(&fixture)?;
    unknown_exporter.evidence[0].exporter = "unregistered".into();
    validate_exporter_coverage(&unknown_exporter, &registered)
        .err()
        .ok_or("an unregistered exporter route must fail")?;

    let mut unknown_input = read_manifest(&fixture)?;
    unknown_input.evidence[0].input = "unknown-input".into();
    validate_exporter_coverage(&unknown_input, &registered)
        .err()
        .ok_or("a route for an unknown input must fail")?;

    let mut source_drift = read_manifest(&fixture)?;
    source_drift.source_capabilities.push("podman".into());
    validate_exporter_coverage(&source_drift, &registered)
        .err()
        .ok_or("source capability drift must fail")?;

    let mut target_drift = read_manifest(&fixture)?;
    target_drift.target_capabilities.retain(|target| target != "podman");
    validate_exporter_coverage(&target_drift, &registered)
        .err()
        .ok_or("target capability drift must fail")?;

    let matrix = repository_root().join("fixtures/differential/podman-lens-complex-corpus");
    let mut per_input = read_manifest(&matrix)?;
    let input = per_input.native_inputs[1].id.clone();
    let before = per_input.evidence.len();
    per_input
        .evidence
        .retain(|route| !(route.input == input && route.exporter == "compose"));
    assert_eq!(per_input.evidence.len() + 1, before);
    validate_exporter_coverage(&per_input, &registered)
        .err()
        .ok_or("one missing exporter route on the second input must fail")?;
    Ok(())
}

#[test]
fn catalogue_and_real_world_provenance_drift_fail_closed() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let catalogue_text = fs::read_to_string(root.join(SCENARIO_CATALOGUE))?;
    assert!(toml::from_str::<ScenarioCatalogue>(&format!("unknown = true\n{catalogue_text}")).is_err());
    let unknown_registration = catalogue_text.replacen("[[scenarios]]", "[[scenarios]]\nunknown = true", 1);
    assert!(toml::from_str::<ScenarioCatalogue>(&unknown_registration).is_err());

    let temporary_root = TemporaryDirectory::new("catalogue-counterfactual")?;
    for approved in APPROVED_SCENARIO_ROOTS {
        fs::create_dir_all(temporary_root.path().join(approved))?;
    }
    let first = "fixtures/scenarios/duplicate/scenario.toml";
    let second = "fixtures/conversion/duplicate/scenario.toml";
    for path in [first, second] {
        let path = temporary_root.path().join(path);
        fs::create_dir_all(path.parent().ok_or("temporary catalogue parent")?)?;
        fs::write(path, "")?;
    }
    let duplicate_id = ScenarioCatalogue {
        schema: 1,
        scenarios: vec![
            ScenarioRegistration {
                id: "duplicate".into(),
                manifest: first.into(),
            },
            ScenarioRegistration {
                id: "duplicate".into(),
                manifest: second.into(),
            },
        ],
    };
    let error = validate_scenario_catalogue(temporary_root.path(), &duplicate_id)
        .err()
        .ok_or("duplicate catalogue id on distinct paths must fail")?;
    assert!(error.contains("duplicate scenario catalogue id"));

    let corpus_path = root.join("fixtures/real-world/corpus.toml");
    let corpus_text = fs::read_to_string(corpus_path)?;
    let corpus: RealWorldCorpus = toml::from_str(&corpus_text)?;
    assert!(toml::from_str::<RealWorldCorpus>(&format!("unknown = true\n{corpus_text}")).is_err());
    let unknown_project = corpus_text.replacen("[[projects]]", "[[projects]]\nunknown = true", 1);
    assert!(toml::from_str::<RealWorldCorpus>(&unknown_project).is_err());

    let project = corpus
        .projects
        .iter()
        .find(|project| project.id == "appwrite")
        .ok_or("Appwrite corpus project")?;
    let fixture = root.join("fixtures/scenarios/real-world-compose-appwrite");
    let manifest = read_manifest(&fixture)?;
    validate_real_world_provenance_link("real-world-compose-appwrite", &manifest, project, &project.blob_sha)?;

    let mut missing_project = read_manifest(&fixture)?;
    missing_project.provenance.corpus_project = None;
    validate_manifest(&missing_project, &fixture)
        .err()
        .ok_or("real-world scenario without corpus project must fail")?;

    let mut wrong_application = read_manifest(&fixture)?;
    wrong_application.application.name = "other".into();
    validate_manifest(&wrong_application, &fixture)
        .err()
        .ok_or("real-world application/project identity drift must fail")?;

    let mut wrong_project = project.clone();
    wrong_project.revision = "0".repeat(40);
    let error = validate_real_world_provenance_link(
        "real-world-compose-appwrite",
        &manifest,
        &wrong_project,
        &project.blob_sha,
    )
    .err()
    .ok_or("revision drift must fail")?;
    assert!(error.contains("revision"));

    let mut wrong_license = project.clone();
    wrong_license.license = "MIT".into();
    let error = validate_real_world_provenance_link(
        "real-world-compose-appwrite",
        &manifest,
        &wrong_license,
        &project.blob_sha,
    )
    .err()
    .ok_or("license drift must fail")?;
    assert!(error.contains("license"));

    let error = validate_real_world_provenance_link("real-world-compose-appwrite", &manifest, project, &"0".repeat(40))
        .err()
        .ok_or("blob drift must fail")?;
    assert!(error.contains("blob"));

    validate_real_world_provenance_link("real-world-compose-other", &manifest, project, &project.blob_sha)
        .err()
        .ok_or("catalogue registration/provenance identity drift must fail")?;
    Ok(())
}

#[test]
fn six_independent_mutations_fail_for_their_own_reason() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let manifest = read_manifest(&fixture)?;
    let source = fs::read_to_string(fixture.join("input-compose.yaml"))?;
    for (name, mutation, message) in [
        (
            "removed mount",
            source.replace("    volumes:\n      - state:/var/lib/app\n", ""),
            "shared consumer",
        ),
        (
            "removed environment",
            source.replace("    environment:\n      APP_MODE: migration\n", ""),
            "required environment",
        ),
        (
            "published database",
            source.replace("  database:\n", "  database:\n    ports:\n      - \"5432:5432\"\n"),
            "must not publish",
        ),
        (
            "extra application",
            source.replace("    profiles: [unrelated]\n", ""),
            "excluded",
        ),
    ] {
        assert_ne!(source, mutation, "{name} must actually mutate its input");
        let imported = import_compose(&mutation, &manifest.application.name)?;
        let error = validate_neutral_application(
            &manifest,
            imported.application().ok_or("mutated import")?,
            &manifest.semantics.required_environment_order,
            &[],
        )
        .err()
        .ok_or("semantic mutation must fail")?;
        assert!(error.contains(message), "{name} failed for the wrong reason: {error}");
    }
    let diagnostic = boxferry::Diagnostic::new(
        boxferry::DiagnosticCode::new("BFP0007")?,
        boxferry::Severity::Warning,
        "unexpected",
    )
    .with_field(boxferry::DiagnosticField::new(
        "subject",
        boxferry::DiagnosticValue::plain("services.database.ports"),
    ));
    validate_diagnostics(&[], &diagnostic_facts(&[diagnostic]))
        .err()
        .ok_or("unexpected diagnostic must fail")?;
    let mut missing = read_manifest(&fixture)?;
    missing.evidence.retain(|route| route.exporter != "podman");
    validate_exporter_coverage(&missing, &capability_routes()?)
        .err()
        .ok_or("missing exporter must fail")?;
    Ok(())
}

#[test]
fn metadata_and_evidence_cannot_claim_unperformed_or_unscoped_success() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let text = fs::read_to_string(fixture.join("scenario.toml"))?;
    for (old, replacement) in [
        ("schema = 1", "schema = 9"),
        ("input-compose.yaml", "../input-compose.yaml"),
        ("input-compose.yaml", "missing.yaml"),
        ("version = \"1.0.0\"", "version = \"latest\""),
        ("sha256:aaaa", "sha256:AAAA"),
    ] {
        let manifest: ScenarioManifest = toml::from_str(&text.replace(old, replacement))?;
        validate_manifest(&manifest, &fixture)
            .err()
            .ok_or("invalid metadata must fail")?;
    }
    assert!(toml::from_str::<ScenarioManifest>(&format!("unknown-field = true\n{text}")).is_err());
    let manifest = read_manifest(&fixture)?;
    let route = manifest
        .evidence
        .iter()
        .find(|route| route.exporter == "compose")
        .ok_or("Compose route")?;
    let actual = RouteObservation {
        blocked: false,
        semantics_match: true,
        native_validation: Dimension::passed(),
        runtime_probe: Dimension::not_applicable(OFFLINE),
        reimport: Dimension::not_applicable("Not run."),
    };
    validate_evidence(route, &actual)
        .err()
        .ok_or("unperformed reimport cannot count as passed")?;
    let loss = boxferry::ConversionOutcome::loss(
        "services.database.ports",
        ConversionKind::Unsupported,
        boxferry::DiagnosticCode::new("BFP0007")?,
    )?;
    validate_losses(&[], &[loss], &route.version_scope())
        .err()
        .ok_or("unexpected loss must fail")?;
    let mut duplicate = read_manifest(&fixture)?;
    duplicate.native_inputs[0].files.push("input-compose.yaml".into());
    validate_manifest(&duplicate, &fixture)
        .err()
        .ok_or("duplicate source must fail")?;
    let mut changed = read_manifest(&fixture)?;
    changed.components[0].digest = Some(format!("sha256:{}", "d".repeat(64)));
    let imported = import_compose(
        &fs::read_to_string(fixture.join("input-compose.yaml"))?,
        &changed.application.name,
    )?;
    validate_neutral_application(
        &changed,
        imported.application().ok_or("application")?,
        &changed.semantics.required_environment_order,
        &[],
    )
    .err()
    .ok_or("a changed expected image must not silently pass")?;
    validate_diagnostics(
        &["BFP0007|services.app.ports".into()],
        &["BFP0007|services.database.ports".into()],
    )
    .err()
    .ok_or("the same code on a different subject is a different diagnostic")?;
    let mut minimal = read_manifest(&fixture)?;
    minimal.semantics.excluded_resources.clear();
    minimal.semantics.ownership_boundaries.clear();
    minimal.semantics.shared_boundaries.clear();
    minimal.semantics.required_mounts.clear();
    minimal.semantics.required_environment.clear();
    minimal.semantics.required_ports.clear();
    minimal.semantics.unpublished_services.clear();
    let error = validate_manifest(&minimal, &fixture)
        .err()
        .ok_or("selected volume without independent ownership and relationship assertions must fail")?;
    assert!(error.contains("selected volume ownership coverage differs"));

    let imported = import_compose(
        &fs::read_to_string(fixture.join("input-compose.yaml"))?,
        &manifest.application.name,
    )?;
    let mut changed_ownership = read_manifest(&fixture)?;
    changed_ownership.semantics.ownership_boundaries = vec!["volume:state:external".into()];
    let error = validate_neutral_application(
        &changed_ownership,
        imported.application().ok_or("application")?,
        &changed_ownership.semantics.required_environment_order,
        &[],
    )
    .err()
    .ok_or("counterfactual volume ownership must fail")?;
    assert!(error.contains("volume state ownership changed"));
    Ok(())
}

#[test]
fn protected_values_are_explicit_and_required_environment_stays_observable() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/conversion/compose-to-quadlet-interpolation");
    let manifest = read_manifest(&fixture)?;

    assert!(
        manifest
            .semantics
            .required_environment
            .iter()
            .any(|requirement| requirement == "app:UNAUTHORIZED=safe-default")
    );
    assert_no_protected_environment_values(&manifest, "safe-default", "ordinary environment evidence")?;
    let error =
        assert_no_protected_environment_values(&manifest, "prefix explicit-secret suffix", "counterfactual report")
            .err()
            .ok_or("declared protected value must fail redaction evidence")?;
    assert!(error.to_string().contains("disclosed a protected scenario value"));

    let mut duplicate = read_manifest(&fixture)?;
    duplicate.protected_values.push("explicit-secret".into());
    let error = validate_manifest(&duplicate, &fixture)
        .err()
        .ok_or("duplicate protected values must fail schema validation")?;
    assert!(error.contains("duplicate protected values"));

    let mut empty = read_manifest(&fixture)?;
    empty.protected_values = vec![String::new()];
    let error = validate_manifest(&empty, &fixture)
        .err()
        .ok_or("empty protected values must fail schema validation")?;
    assert!(error.contains("protected values must be non-empty"));
    Ok(())
}

#[test]
fn prerequisite_metadata_rejects_undeclared_unsafe_and_successful_missing_inputs() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    for prerequisite in ["socket:runtime", "file:../outside", "file:"] {
        let mut manifest = read_manifest(&fixture)?;
        manifest.semantics.external_prerequisites = vec![prerequisite.into()];
        validate_manifest(&manifest, &fixture)
            .err()
            .ok_or("invalid prerequisite must fail schema validation")?;
    }
    let mut manifest = read_manifest(&fixture)?;
    manifest.evidence[0].unavailable_prerequisites = vec!["file:missing".into()];
    let error = validate_manifest(&manifest, &fixture)
        .err()
        .ok_or("undeclared prerequisite must fail")?;
    assert!(error.contains("not declared"));
    manifest.semantics.external_prerequisites = vec!["file:missing".into()];
    let error = validate_manifest(&manifest, &fixture)
        .err()
        .ok_or("missing prerequisite cannot be success")?;
    assert!(error.contains("unsupported-environment"));
    Ok(())
}

#[test]
fn sourced_live_helpers_validate_catalogues_without_provisioning() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let module = root.join("scripts/lib/scenario-contract.sh");
    let directory = TemporaryDirectory::new("catalogue")?;
    let valid = fs::read_to_string(root.join("fixtures/conformance/podman-live/scenarios.tsv"))?;
    let first = valid
        .lines()
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or("catalogue row")?;
    for (name, source, succeeds) in [
        ("valid", valid.clone(), true),
        ("duplicate", format!("{valid}\n{first}\n"), false),
        ("invalid", "only-one-column\n".into(), false),
    ] {
        let file = directory.path().join(name);
        fs::write(&file, source)?;
        let output = Command::new("bash")
            .args([
                "-c",
                "source \"$1\"\nscenario_catalogue_validate \"$2\"",
                "scenario-test",
            ])
            .arg(&module)
            .arg(&file)
            .output()?;
        assert_eq!(output.status.success(), succeeds, "catalogue {name}");
    }
    let output = Command::new("bash")
        .args(["-c", "source \"$1\"", "validator-test"])
        .arg(root.join("scripts/lib/scenario-validators.sh"))
        .output()?;
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "sourcing validators must perform no checks"
    );
    Ok(())
}

fn import_native_input(
    manifest: &ScenarioManifest,
    input: &NativeInput,
    fixture: &Path,
) -> Result<ImportResult, Box<dyn Error>> {
    match input.importer.as_str() {
        "compose" => import_compose_files(input, fixture, &manifest.application.name),
        "quadlet" => import_quadlet_files(input, fixture, &manifest.application.name),
        "podman" => {
            let source = input.files.first().ok_or("missing Podman cassette")?;
            import_podman(manifest, &input.id, &fixture.join(source))
        }
        _ => Err("unimplemented scenario input loader".into()),
    }
}

fn import_compose_files(input: &NativeInput, fixture: &Path, name: &str) -> Result<ImportResult, Box<dyn Error>> {
    if input.format_version != "compose-specification" {
        return Err("Compose scenario format version must be compose-specification".into());
    }
    let source_ids = input
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| Ok((ComposeSourceId::new(u32::try_from(index + 1)?), file)))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let documents = source_ids
        .iter()
        .map(|(source_id, file)| {
            Ok(DocumentInput::new(
                *source_id,
                DocumentOrigin::new(file.as_str(), fixture),
                fs::read_to_string(fixture.join(file))?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let loaded = LoadedProject::load(documents)?;
    let interpolation = if input.interpolate {
        let mut environment = MapEnvironment::new();
        for file in &input.environment_files {
            for assignment in parse_scenario_environment_file(&fixture.join(file))? {
                let (key, value) = assignment
                    .split_once('=')
                    .ok_or("scenario environment-file assignment requires NAME=VALUE")?;
                let _ = environment.insert_sensitive(key, value);
            }
        }
        for assignment in &input.environment {
            let (key, value) = assignment
                .split_once('=')
                .ok_or("scenario environment must be NAME=VALUE")?;
            if key.is_empty() {
                return Err("scenario environment name cannot be empty".into());
            }
            let _ = environment.insert_sensitive(key, value);
        }
        Some(loaded.interpolate(&environment))
    } else {
        None
    };
    let project = merge_project(&loaded, interpolation.as_ref())
        .project()
        .ok_or("merged Compose project")?
        .clone();
    let profile_request = if input.all_profiles {
        ProfileRequest::all()
    } else {
        ProfileRequest::new()
    };
    let selection = select_profiles(&project, &profile_request);
    let mut source = ComposeSource::new(project, Identifier::new(name)?)?.with_profile_selection(selection);
    for (source_id, file) in source_ids {
        source = source.with_source_id(source_id, SourceId::new(file)?);
    }
    Ok(ComposeImporter::new()?.import(&source))
}

fn parse_scenario_environment_file(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, _) = line
                .split_once('=')
                .ok_or("scenario environment file requires NAME=VALUE lines")?;
            if name.is_empty() {
                return Err("scenario environment file contains an empty name".into());
            }
            Ok(line.to_owned())
        })
        .collect()
}

fn import_quadlet_files(input: &NativeInput, fixture: &Path, name: &str) -> Result<ImportResult, Box<dyn Error>> {
    let units = input
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            Ok(QuadletDocumentInput::new(
                file,
                boxferry::quadlet::quadlet_lens::source::SourceId::new(u32::try_from(index + 1)?),
                fs::read_to_string(fixture.join(file))?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let parsed = QuadletSource::parse(Identifier::new(name)?, units)?;
    Ok(QuadletImporter::new()?.import(parsed.source()))
}

fn import_compose(text: &str, name: &str) -> Result<ImportResult, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(129);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("scenario.yaml", name),
        text,
    )])?;
    let project = merge_project(&loaded, None)
        .project()
        .ok_or("merged Compose project")?
        .clone();
    let selection = select_profiles(&project, &ProfileRequest::new());
    let source = ComposeSource::new(project, Identifier::new(name)?)?
        .with_source_id(source_id, SourceId::new("scenario.yaml")?)
        .with_profile_selection(selection);
    Ok(ComposeImporter::new()?.import(&source))
}

fn assert_no_protected_environment_values(
    manifest: &ScenarioManifest,
    text: &str,
    evidence: &str,
) -> Result<(), Box<dyn Error>> {
    for value in &manifest.protected_values {
        if text.contains(value) {
            return Err(format!("{evidence} disclosed a protected scenario value").into());
        }
    }
    Ok(())
}

fn import_podman(manifest: &ScenarioManifest, input_id: &str, path: &Path) -> Result<ImportResult, Box<dyn Error>> {
    import_podman_cassette(manifest, input_id, PodmanCassette::load(path)?)
}

fn import_podman_cassette(
    manifest: &ScenarioManifest,
    input_id: &str,
    cassette: PodmanCassette,
) -> Result<ImportResult, Box<dyn Error>> {
    let input = manifest
        .native_inputs
        .iter()
        .find(|input| input.id == input_id)
        .ok_or("missing Podman scenario input")?;
    let podman = input.podman.as_ref().ok_or("missing Podman input metadata")?;
    if cassette.scenario_id() != podman.cassette_scenario_id.as_deref().unwrap_or(&manifest.id)
        || cassette.engine_version() != input.format_version
        || cassette.execution_context() != podman.root_mode.as_deref().unwrap_or(&manifest.deployment.root_mode)
    {
        return Err("Podman cassette identity differs from scenario metadata".into());
    }
    let server = PodmanCassetteServer::start(cassette)?;
    let transport = ReadOnlyUnixTransport::new(
        UnixConnection::new(server.socket())?,
        TransportLimits::default(),
        ReadOnlyUnixTransportTimeouts::default(),
    )?;
    let request = podman_discovery_request(manifest, input_id)?;
    let policy = podman_promotion_policy(manifest, input_id)?;
    let acquisition = if podman.include_environment_values {
        AcquisitionOptions::include_environment_values()
    } else {
        AcquisitionOptions::redacted()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let source = runtime.block_on(acquire_podman_source(
        Identifier::new(&podman.application)?,
        &transport,
        acquisition,
        &request,
        policy,
    ))?;
    let inventory_snapshot = serde_json::to_string(&source.redacted_inventory_snapshot())?;
    let graph_snapshot = serde_json::to_string(&source.redacted_graph_snapshot())?;
    assert_no_protected_environment_values(manifest, &inventory_snapshot, "inventory snapshot")?;
    assert_no_protected_environment_values(manifest, &graph_snapshot, "graph snapshot")?;
    assert_no_protected_environment_values(manifest, &format!("{source:?}"), "Podman source debug")?;
    server.finish()?;
    Ok(PodmanImporter::new()?.import(&source))
}

fn podman_discovery_request(manifest: &ScenarioManifest, input_id: &str) -> Result<DiscoveryRequest, Box<dyn Error>> {
    let input = manifest
        .native_inputs
        .iter()
        .find(|input| input.id == input_id)
        .ok_or("missing Podman scenario input")?;
    let podman = input.podman.as_ref().ok_or("missing Podman input metadata")?;
    let mut request = DiscoveryRequest::new();
    if podman.all {
        request.select_all();
    }
    for selector in &podman.selectors {
        let kind = match selector.kind.as_str() {
            "container" => PodmanResourceKind::Container,
            "pod" => PodmanResourceKind::Pod,
            "network" => PodmanResourceKind::Network,
            "volume" => PodmanResourceKind::Volume,
            "image" => PodmanResourceKind::Image,
            "secret" => PodmanResourceKind::Secret,
            _ => return Err("unvalidated Podman selector kind".into()),
        };
        request.add_root(ResourceSelector::exact(kind, &selector.exact)?);
    }
    Ok(request)
}

fn podman_promotion_policy(
    manifest: &ScenarioManifest,
    input_id: &str,
) -> Result<PodmanPromotionPolicy, Box<dyn Error>> {
    let input = manifest
        .native_inputs
        .iter()
        .find(|input| input.id == input_id)
        .ok_or("missing Podman scenario input")?;
    let policy = input
        .podman
        .as_ref()
        .ok_or("missing Podman input metadata")?
        .promotion_policy;
    Ok(PodmanPromotionPolicy::conservative()
        .with_effective_bind_mounts(policy.effective_bind_mounts)
        .with_effective_named_volume_mounts(policy.effective_named_volume_mounts)
        .with_effective_named_networks(policy.effective_named_networks)
        .with_portable_effective_settings(policy.portable_effective_settings))
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps CLI parity construction shared across importers"
)]
fn convert_cli(
    manifest: &ScenarioManifest,
    sources: &[PathBuf],
    route: &RouteExpectation,
    destination: &Path,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let input = manifest
        .native_inputs
        .iter()
        .find(|input| input.id == route.input)
        .ok_or("missing scenario route input")?;
    let report_directory = TemporaryDirectory::new_in(&repository_root().join("target"), "scenario-full-report")?;
    let report_path = report_directory.path().join("report.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command.args(["convert", &input.importer, &route.exporter]);
    let mut server = None;
    match input.importer.as_str() {
        "compose" => {
            for source in sources {
                command.arg("--input-file").arg(source);
            }
            command.args(["--project-name", &manifest.application.name]);
            if input.interpolate {
                command.arg("--interpolate");
                for file in &input.environment_files {
                    command.arg("--env-file").arg(
                        sources
                            .first()
                            .and_then(|source| source.parent())
                            .ok_or("scenario source directory")?
                            .join(file),
                    );
                }
                for assignment in &input.environment {
                    command.arg(format!("--env={assignment}"));
                }
            }
            if input.all_profiles {
                command.arg("--all-profiles");
            }
        }
        "quadlet" => {
            for source in sources {
                command.arg("--input-file").arg(source);
            }
            command.args(["--application-name", &manifest.application.name]);
        }
        "podman" => {
            let podman = input.podman.as_ref().ok_or("missing Podman input metadata")?;
            let source = sources.first().ok_or("missing Podman cassette")?;
            let cassette_server = PodmanCassetteServer::start(PodmanCassette::load(source)?)?;
            command
                .arg("--podman-socket")
                .arg(cassette_server.socket())
                .args(["--application-name", &podman.application]);
            if podman.all {
                command.arg("--podman-all");
            } else {
                for selector in &podman.selectors {
                    command.args(["--podman-resource", &format!("{}={}", selector.kind, selector.exact)]);
                }
            }
            let policy = podman.promotion_policy;
            for (enabled, argument) in [
                (policy.effective_bind_mounts, "--promote-podman-effective-bind-mounts"),
                (
                    policy.effective_named_volume_mounts,
                    "--promote-podman-effective-named-volumes",
                ),
                (
                    policy.effective_named_networks,
                    "--promote-podman-effective-named-networks",
                ),
                (
                    policy.portable_effective_settings,
                    "--promote-podman-portable-effective-settings",
                ),
            ] {
                if enabled {
                    command.arg(argument);
                }
            }
            server = Some(cassette_server);
        }
        _ => return Err("unimplemented scenario CLI input".into()),
    }
    command
        .args(["--loss-policy", &route.loss_policy, "--output-directory"])
        .arg(destination);
    if route.exporter == "quadlet" {
        command.args([
            "--podman-minimum-version",
            &route.target_minimum,
            "--podman-maximum-version",
            &route.target_maximum,
        ]);
        if let Some(grouping) = &route.grouping {
            command.args(["--quadlet-grouping", grouping]);
        }
        if let Some(pod_name) = &route.pod_name {
            command.args(["--pod-name", pod_name]);
        }
    }
    if route.exporter == "podman" {
        let requested_context = input
            .podman
            .as_ref()
            .and_then(|podman| podman.root_mode.as_deref())
            .unwrap_or(&manifest.deployment.root_mode);
        let context = match requested_context {
            "rootful" | "rootless" => requested_context,
            _ => "unknown",
        };
        command.args([
            "--podman-target-context",
            context,
            "--podman-max-version",
            &route.target_maximum,
        ]);
    }
    let result = command
        .args(["--console-format", "json", "--report-file"])
        .arg(&report_path)
        .output()?;
    assert_no_protected_environment_values(manifest, &String::from_utf8_lossy(&result.stdout), "CLI stdout")?;
    if let Some(server) = server {
        server.finish()?;
    }
    assert!(
        result.stderr.is_empty(),
        "unexpected CLI failure: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report_bytes = fs::read(&report_path)?;
    assert_no_protected_environment_values(manifest, &String::from_utf8_lossy(&report_bytes), "CLI JSON report")?;
    let mut report: serde_json::Value = serde_json::from_slice(&report_bytes).map_err(|error| {
        format!(
            "{} -> {} full report is invalid at {} bytes: {error}",
            route.input,
            route.exporter,
            report_bytes.len()
        )
    })?;
    let status = if result.status.success() {
        "success"
    } else if report["exit_category"] == "policy-blocked" {
        "blocked"
    } else {
        "failure"
    };
    report["status"] = serde_json::Value::String(status.into());
    Ok(report)
}

fn capability_routes() -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["capabilities", "--console-format", "json"])
        .output()?;
    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout)?["routes"]
        .as_array()
        .ok_or("capability routes")?
        .iter()
        .map(|route| {
            Ok((
                route["input_type"].as_str().ok_or("input")?.into(),
                route["output_type"].as_str().ok_or("output")?.into(),
            ))
        })
        .collect()
}
fn read_manifest(fixture: &Path) -> Result<ScenarioManifest, Box<dyn Error>> {
    read_manifest_file(&fixture.join("scenario.toml"))
}
fn read_manifest_file(manifest: &Path) -> Result<ScenarioManifest, Box<dyn Error>> {
    Ok(toml::from_str(&fs::read_to_string(manifest)?)?)
}
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct TemporaryDirectory {
    path: PathBuf,
}
impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        Self::new_in(&std::env::temp_dir(), label)
    }
    fn new_in(parent: &Path, label: &str) -> Result<Self, std::io::Error> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("boxferry-scenario-{}-{label}-{id}", std::process::id()));
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
