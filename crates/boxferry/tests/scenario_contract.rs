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
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::{
    ComposeExporter, ComposeImporter, ComposeSource, ConversionResult, Identifier, ImportAdapter, ImportResult,
    PodmanExporter, QuadletDocumentInput, QuadletExporter, QuadletImporter, QuadletSource, SourceId, convert_imported,
};
use support::{
    Dimension, DimensionState, Outcome, RouteExpectation, RouteObservation, ScenarioManifest, diagnostic_facts,
    observe_prerequisites, validate_diagnostics, validate_evidence, validate_exporter_coverage, validate_losses,
    validate_manifest, validate_neutral_application,
};

const OFFLINE: &str = "Offline authored fixture; no runtime operation is claimed.";
const NO_PODMAN_REIMPORT: &str = "Deployment plans are not observed Podman inventories.";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn authored_scenarios_execute_every_exporter_and_record_independent_evidence() -> Result<(), Box<dyn Error>> {
    let root = repository_root().join("fixtures/scenarios");
    let mut paths = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    assert!(!paths.is_empty(), "scenario tree cannot silently disappear");
    for fixture in paths {
        let manifest = read_manifest(&fixture)?;
        validate_manifest(&manifest, &fixture)?;
        validate_exporter_coverage(&manifest, &capability_routes()?)?;
        for input in &manifest.native_inputs {
            // A new importer case must get an implementation, never a silent skip.
            if input.importer != "compose" {
                return Err("unimplemented scenario input loader".into());
            }
            assert_eq!(
                input.files.len(),
                1,
                "authored loader currently accepts one explicit source"
            );
            assert_eq!(input.format_version, "compose-specification");
            let source_path = fixture.join(&input.files[0]);
            let imported = import_compose(&fs::read_to_string(&source_path)?, &manifest.application.name)?;
            validate_diagnostics(
                &manifest.semantics.expected_diagnostics,
                &diagnostic_facts(imported.diagnostics()),
            )?;
            validate_neutral_application(&manifest, imported.application().ok_or("missing imported Application")?)?;
            for route in manifest.evidence.iter().filter(|route| route.input == input.id) {
                run_route(&manifest, route, &source_path, &fixture, imported.clone())?;
            }
        }
    }
    Ok(())
}

fn run_route(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    source: &Path,
    fixture: &Path,
    imported: ImportResult,
) -> Result<(), Box<dyn Error>> {
    let missing = observe_prerequisites(manifest, fixture)?;
    validate_diagnostics(&route.unavailable_prerequisites, &missing)?;
    if !missing.is_empty() {
        assert!(route.artifacts.is_empty());
        validate_diagnostics(&route.diagnostics, &[])?;
        validate_losses(&route.allowed_losses, &[], &route.version_scope())?;
        validate_diagnostics(&route.semantic_gaps, &[])?;
        return Ok(validate_evidence(route, &unavailable_observation())?);
    }
    let target = route.target()?;
    match route.exporter.as_str() {
        "compose" => validate_result(
            route,
            &convert_imported(imported, &ComposeExporter::new()?, &target, route.policy()?)?,
        )?,
        "quadlet" => validate_result(
            route,
            &convert_imported(imported, &QuadletExporter::new()?, &target, route.policy()?)?,
        )?,
        "podman" => validate_result(
            route,
            &convert_imported(imported, &PodmanExporter::new()?, &target, route.policy()?)?,
        )?,
        _ => return Err("registered exporter has no scenario runner".into()),
    }
    let output = TemporaryDirectory::new(&route.exporter)?;
    let report = convert_cli(source, route, output.path())?;
    let diagnostics = report["diagnostics"]
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
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    validate_diagnostics(&route.diagnostics, &diagnostics)?;
    let mut artifacts = fs::read_dir(output.path())?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("non-UTF-8 artifact"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    artifacts.sort();
    let mut expected = route.artifacts.clone();
    expected.sort();
    assert_eq!(artifacts, expected, "artifact inventory changed");
    if report["status"] == "blocked" {
        return Ok(validate_evidence(route, &rejected_observation())?);
    }
    assert_eq!(
        report["status"], "success",
        "CLI failure is not an expected policy rejection"
    );

    let (semantics_match, reimport) = observe_native_output(manifest, route, &artifacts, output.path())?;
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

fn observe_native_output(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    artifacts: &[String],
    output: &Path,
) -> Result<(bool, Dimension), Box<dyn Error>> {
    Ok(match route.exporter.as_str() {
        "compose" => {
            let reimport = import_compose(
                &fs::read_to_string(output.join("compose.yaml"))?,
                &manifest.application.name,
            )?;
            validate_diagnostics(&[], &diagnostic_facts(reimport.diagnostics()))?;
            validate_neutral_application(manifest, reimport.application().ok_or("Compose reimport failed")?)?;
            (true, Dimension::passed())
        }
        "quadlet" => {
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
            validate_diagnostics(&[], &diagnostic_facts(reimport.diagnostics()))?;
            validate_neutral_application(manifest, reimport.application().ok_or("Quadlet reimport failed")?)?;
            (true, Dimension::passed())
        }
        "podman" => {
            let native: serde_json::Value = serde_json::from_str(&fs::read_to_string(output.join("podman.json"))?)?;
            let observed_gaps = podman_semantics(manifest, &native)?;
            validate_diagnostics(&route.semantic_gaps, &observed_gaps)?;
            (observed_gaps.is_empty(), Dimension::not_applicable(NO_PODMAN_REIMPORT))
        }
        _ => return Err("missing native validator".into()),
    })
}

const NOT_EMITTED: &str = "Loss policy blocked output; no native artifact was emitted.";
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

fn validate_result<T>(route: &RouteExpectation, result: &ConversionResult<T>) -> Result<(), String> {
    if result.is_blocked() != (route.outcome == Outcome::ExpectedRejection) {
        return Err("actual policy authorization differs from the expected route outcome".into());
    }
    validate_losses(&route.allowed_losses, result.outcomes(), &route.version_scope())?;
    validate_diagnostics(&route.diagnostics, &diagnostic_facts(result.diagnostics()))
}

/// Validate the native JSON shape and independent desired create operations. A
/// deployment artifact cannot be passed to the inventory importer as a round trip.
fn podman_semantics(manifest: &ScenarioManifest, value: &serde_json::Value) -> Result<Vec<String>, Box<dyn Error>> {
    assert_eq!(value["schema_version"], 1);
    let operations = value["operations"].as_array().ok_or("native deployment operations")?;
    let creates = operations
        .iter()
        .filter(|operation| operation["action"] == "create")
        .collect::<Vec<_>>();
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
        let image = format!("{}@{}", component.image, component.digest);
        // This authored fixture requests no command override: the image must be
        // the final argument, not a label value or a later command operand.
        assert_eq!(argv.last().and_then(serde_json::Value::as_str), Some(image.as_str()));
        assert_eq!(body["image"], image);
        let options = &argv[..argv.len() - 1];
        gaps.extend(podman_environment_and_ports(manifest, &component.name, options, body)?);
        validate_podman_mounts(manifest, &component.name, options, body)?;
    }
    Ok(gaps)
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

fn podman_environment_and_ports(
    manifest: &ScenarioManifest,
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

fn validate_podman_mounts(
    manifest: &ScenarioManifest,
    name: &str,
    options: &[serde_json::Value],
    body: &serde_json::Value,
) -> Result<(), Box<dyn Error>> {
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
        run_route(&manifest, route, &source, &fixture, imported.clone())?;
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
        run_route(&manifest, route, &source, &fixture, imported.clone())?;
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
            "selected resources",
        ),
    ] {
        assert_ne!(source, mutation, "{name} must actually mutate its input");
        let imported = import_compose(&mutation, &manifest.application.name)?;
        let error = validate_neutral_application(&manifest, imported.application().ok_or("mutated import")?)
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
        boxferry::ConversionKind::Unsupported,
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
    changed.components[0].digest = format!("sha256:{}", "d".repeat(64));
    let imported = import_compose(
        &fs::read_to_string(fixture.join("input-compose.yaml"))?,
        &changed.application.name,
    )?;
    validate_neutral_application(&changed, imported.application().ok_or("application")?)
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
    validate_manifest(&minimal, &fixture)?;
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

fn convert_cli(
    input: &Path,
    route: &RouteExpectation,
    destination: &Path,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command
        .args(["convert", "compose", &route.exporter, "--input-file"])
        .arg(input)
        .args([
            "--project-name",
            "authored-core",
            "--loss-policy",
            &route.loss_policy,
            "--output-directory",
        ])
        .arg(destination);
    if route.exporter == "quadlet" {
        command.args([
            "--podman-minimum-version",
            &route.target_minimum,
            "--podman-maximum-version",
            &route.target_maximum,
        ]);
    }
    if route.exporter == "podman" {
        command.args([
            "--podman-target-context",
            "unknown",
            "--podman-max-version",
            &route.target_maximum,
        ]);
    }
    let result = command.args(["--console-format", "json"]).output()?;
    assert!(
        result.stderr.is_empty(),
        "unexpected CLI failure: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(result.status.success(), report["status"] == "success");
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
    Ok(toml::from_str(&fs::read_to_string(fixture.join("scenario.toml"))?)?)
}
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct TemporaryDirectory {
    path: PathBuf,
}
impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-scenario-{}-{label}-{id}", std::process::id()));
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
