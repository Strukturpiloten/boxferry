//! Offline acceptance: authored intent, every exporter, separate executed evidence.
#![cfg(all(
    feature = "cli",
    feature = "compose",
    feature = "docker",
    feature = "podman",
    feature = "quadlet"
))]

#[allow(dead_code, unused_imports)]
mod support;

#[cfg(feature = "docker")]
use std::collections::BTreeMap;
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
    AcquisitionOptions, DiscoveryRequest, LabelSelector, ReadOnlyUnixTransport, ReadOnlyUnixTransportTimeouts,
    ResourceKind as PodmanResourceKind, ResourceSelector, TransportLimits, UnixConnection,
};
use boxferry::{
    ComposeExporter, ComposeImporter, ComposeSource, ConversionKind, ConversionResult, ExportAdapter, Identifier,
    ImportAdapter, ImportResult, PodmanExporter, PodmanImporter, PodmanPromotionPolicy, QuadletDocumentInput,
    QuadletExporter, QuadletGroupingPolicy, QuadletImporter, QuadletSource, SourceId, acquire_podman_source,
    convert_imported,
};
#[cfg(feature = "docker")]
use boxferry::{DockerEnvironmentPolicy, DockerExporter};
use serde::Deserialize;
#[cfg(feature = "docker")]
use sha2::{Digest, Sha256};
use support::{
    Dimension, DimensionState, NativeInput, Outcome, PodmanCassette, PodmanCassetteServer, RouteExpectation,
    RouteObservation, ScenarioManifest, diagnostic_facts, load_loss_expectations, load_string_expectations,
    observe_prerequisites, validate_diagnostics, validate_evidence, validate_exporter_coverage, validate_losses,
    validate_manifest, validate_neutral_application, validate_withheld_reimport,
};

const OFFLINE: &str = "Offline authored fixture; no runtime operation is claimed.";
const NO_PODMAN_REIMPORT: &str = "Deployment plans are not observed Podman inventories.";
#[cfg(feature = "docker")]
const NO_DOCKER_REIMPORT: &str = "Inert Docker create requests are desired intent, not observed Docker inventory.";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "docker")]
fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    encoded
}

#[cfg(feature = "docker")]
#[test]
fn docker_dependency_explicitness_deserializes_authored_booleans() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        serde_json::from_value::<support::DockerDependencyExplicitness>(serde_json::json!(true))?,
        support::DockerDependencyExplicitness::Explicit
    );
    assert_eq!(
        serde_json::from_value::<support::DockerDependencyExplicitness>(serde_json::json!(false))?,
        support::DockerDependencyExplicitness::Default
    );
    assert!(serde_json::from_value::<support::DockerDependencyExplicitness>(serde_json::json!(1)).is_err());
    Ok(())
}

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
    assert_eq!(registered.len(), 33);
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
        validate_manifest(&manifest, &registration.manifest).map_err(|error| {
            format!(
                "scenario {} manifest {}: {error}",
                manifest.id,
                registration.manifest.display()
            )
        })?;
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
            let imported = import_native_input(&manifest, input, fixture)
                .map_err(|error| format!("scenario {} input {}: {error}", manifest.id, input.id))?;
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

#[test]
fn scenario_artifact_environment_inclusion_requires_explicit_reviewed_route() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let manifest = read_manifest(&fixture)?;
    validate_manifest(&manifest, &fixture)?;
    let source = fixture.join("input-compose.yaml");
    let included_output = TemporaryDirectory::new("scenario-included-environment")?;
    let included_report = convert_cli(
        &manifest,
        std::slice::from_ref(&source),
        &manifest.evidence[0],
        included_output.path(),
    )?;
    assert_eq!(included_report["status"], "success");
    assert!(!report_diagnostic_facts(&included_report)?.contains(&"BFC0007|services.app.environment.APP_MODE".into()));
    let included_artifact = fs::read_to_string(included_output.path().join("compose.yaml"))?;
    assert!(included_artifact.contains("APP_MODE") && included_artifact.contains("migration"));

    let protected_fixture = repository_root().join("fixtures/scenarios/forgejo-application");
    let protected_manifest = read_manifest(&protected_fixture)?;
    let protected_output = TemporaryDirectory::new("scenario-included-protected-canary")?;
    let protected_report = convert_cli(
        &protected_manifest,
        &[protected_fixture.join("compose.yaml")],
        &protected_manifest.evidence[0],
        protected_output.path(),
    )?;
    assert_eq!(protected_report["status"], "success");
    let artifact = fs::read_to_string(protected_output.path().join("compose.yaml"))?;
    for canary in &protected_manifest.protected_values {
        assert!(
            artifact.contains(canary),
            "explicitly authorized fake canary was withheld"
        );
    }

    let mut withheld = read_manifest(&fixture)?;
    withheld.evidence[0].set_environment_values_for_test(false);
    validate_manifest(&withheld, &fixture)?;
    let output = TemporaryDirectory::new("scenario-withheld-environment")?;
    let report = convert_cli(
        &withheld,
        std::slice::from_ref(&source),
        &withheld.evidence[0],
        output.path(),
    )?;
    assert_eq!(report["status"], "success");
    assert!(report_diagnostic_facts(&report)?.contains(&"BFC0007|services.app.environment.APP_MODE".into()));
    assert!(!fs::read_to_string(output.path().join("compose.yaml"))?.contains("migration"));

    let mut unreviewed = read_manifest(&fixture)?;
    unreviewed.provenance.source = "external".into();
    assert!(
        validate_manifest(&unreviewed, &fixture)
            .err()
            .is_some_and(|error| error.contains("reviewed authored input with value acquisition"))
    );

    let mut podman_output = read_manifest(&fixture)?;
    podman_output.evidence[2].set_environment_values_for_test(true);
    assert!(
        validate_manifest(&podman_output, &fixture)
            .err()
            .is_some_and(|error| error.contains("reviewed authored input with value acquisition"))
    );

    validate_podman_value_route_mutations()?;
    Ok(())
}

fn validate_podman_value_route_mutations() -> Result<(), Box<dyn Error>> {
    let podman_input_fixture = repository_root().join("fixtures/scenarios/nextcloud-application");
    let mut podman_input = read_manifest(&podman_input_fixture)?;
    podman_input
        .native_inputs
        .iter_mut()
        .find(|input| input.id == "podman")
        .and_then(|input| input.podman.as_mut())
        .ok_or("missing authored Podman acquisition policy")?
        .include_environment_values = false;
    assert!(
        validate_manifest(&podman_input, &podman_input_fixture)
            .err()
            .is_some_and(|error| error.contains("reviewed authored input with value acquisition"))
    );

    let mut unpromoted = read_manifest(&podman_input_fixture)?;
    unpromoted
        .native_inputs
        .iter_mut()
        .find(|input| input.id == "podman")
        .and_then(|input| input.podman.as_mut())
        .ok_or("missing authored Podman promotion policy")?
        .promotion_policy
        .portable_effective_settings = false;
    assert!(
        validate_manifest(&unpromoted, &podman_input_fixture)
            .err()
            .is_some_and(|error| error.contains("reviewed authored input with value acquisition"))
    );

    let withheld_podman_route = read_manifest(&podman_input_fixture)?;
    let route = withheld_podman_route
        .evidence
        .iter()
        .find(|route| route.input == "podman" && route.exporter == "podman")
        .ok_or("missing withheld Podman-to-Podman route")?;
    let expected = route.cli_diagnostics(&withheld_podman_route, &podman_input_fixture)?;
    assert!(expected.contains(&"BFP0002|services.app.environment.POSTGRES_PASSWORD".into()));
    assert!(!expected.contains(&"BFP0003|services.app.environment".into()));
    let mut incomplete = read_manifest(&podman_input_fixture)?;
    incomplete.semantics.required_environment.pop();
    assert!(
        validate_manifest(&incomplete, &podman_input_fixture)
            .err()
            .is_some_and(|error| error.contains("withheld diagnostic delta must cover every"))
    );
    Ok(())
}

#[test]
fn authored_podman_value_routes_have_complete_withholding_contracts() -> Result<(), Box<dyn Error>> {
    let registrations = registered_scenarios(&repository_root())?;
    let mut reviewed = 0;
    for registration in registrations {
        let manifest = read_manifest_file(&registration.manifest)?;
        if manifest.provenance.source != "authored"
            || !manifest.native_inputs.iter().any(|input| {
                input.importer == "podman"
                    && input
                        .podman
                        .as_ref()
                        .is_some_and(|podman| podman.include_environment_values)
            })
        {
            continue;
        }
        validate_manifest(&manifest, &registration.manifest).map_err(|error| format!("{}: {error}", manifest.id))?;
        reviewed += 1;
    }
    assert_eq!(reviewed, 8, "reviewed authored Podman value corpus changed");
    Ok(())
}

#[test]
fn supabase_dual_network_replay_keeps_topology_and_deduplicates_only_cli_notes() -> Result<(), Box<dyn Error>> {
    const KONG_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let root = repository_root();
    let fixture = root.join("fixtures/scenarios/supabase-application");
    let manifest = read_manifest(&fixture)?;
    let route = manifest
        .evidence
        .iter()
        .find(|route| route.input == "podman" && route.exporter == "compose")
        .ok_or("missing Supabase Compose route")?;
    // Independently authored network-only projection of the real 6.1.2 rootless
    // inspect shape: both attachments, distinct aliases, and all attachment fields.
    // The original 6.1.0 cassette and its expectations remain unchanged; this replay
    // is not evidence of the full live application or its environment inventory.
    let text = fs::read_to_string(fixture.join("input-podman.cassette.json"))?.replace("c-kong", KONG_ID);
    let mut projection: serde_json::Value = serde_json::from_str(&text)?;
    let kong = projection["interactions"]
        .as_array_mut()
        .ok_or("cassette interactions")?
        .iter_mut()
        .find(|entry| entry["request"]["path"] == format!("/v6.1.0/libpod/containers/{KONG_ID}/json"))
        .ok_or("Kong inspect interaction")?;
    let attachment = |id: &str, alias: &str| {
        serde_json::json!({
            "NetworkID": id, "Aliases": [alias, &KONG_ID[..12]],
            "EndpointID": "", "Gateway": "", "IPAddress": "", "IPPrefixLen": 0,
            "IPv6Gateway": "", "GlobalIPv6Address": "", "GlobalIPv6PrefixLen": 0,
            "MacAddress": "", "DriverOpts": null, "IPAMConfig": null, "Links": null,
        })
    };
    kong["response"]["body"]["HostConfig"]["NetworkMode"] = serde_json::json!("bridge");
    kong["response"]["body"]["NetworkSettings"]["Networks"] = serde_json::json!({
        "backend": attachment("n-backend", "kong"),
        "edge": attachment("n-edge", "supabase"),
    });
    // Exact-order replay follows sorted runtime identities; the realistic hex ID
    // now sorts ahead of the original cassette's remaining c-* identities.
    let interactions = projection["interactions"]
        .as_array_mut()
        .ok_or("cassette interactions")?;
    let index = interactions
        .iter()
        .position(|entry| entry["request"]["path"] == format!("/v6.1.0/libpod/containers/{KONG_ID}/json"))
        .ok_or("Kong inspect interaction")?;
    let interaction = interactions.remove(index);
    let index = interactions
        .iter()
        .position(|entry| entry["request"]["path"] == "/v6.1.0/libpod/containers/c-auth/json")
        .ok_or("first container inspect interaction")?;
    interactions.insert(index, interaction);
    let imported = import_podman_cassette(&manifest, "podman", serde_json::from_value(projection.clone())?)?;
    supabase_assert_kong_networks(&imported)?;
    let notes = imported
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.code().as_str() == "BFP0009"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().redacted() == "services.kong.networks")
        })
        .collect::<Vec<_>>();
    assert_eq!(notes.len(), 2, "library retains per-attachment notes");
    assert_eq!(notes[0], notes[1], "only identical complete diagnostics collapse");
    let conversion = convert_imported(imported, &ComposeExporter::new()?, &route.target()?, route.policy()?)?;
    assert_eq!(
        diagnostic_facts(conversion.diagnostics())
            .iter()
            .filter(|fact| fact.as_str() == "BFP0009|services.kong.networks")
            .count(),
        2,
        "facade conversion retains both library notes"
    );
    let generated = conversion.output().ok_or("facade Compose output")?;
    supabase_assert_kong_networks(&import_compose(generated.text(), "supabase")?)?;
    let temporary = TemporaryDirectory::new("supabase-dual-network-notes")?;
    let source = temporary.path().join("network-projection.cassette.json");
    fs::write(&source, serde_json::to_vec(&projection)?)?;
    let destination = temporary.path().join("compose-output");
    let report = convert_cli(&manifest, &[source], route, &destination)?;
    assert_eq!(report["status"], "success");
    assert_eq!(
        report_diagnostic_facts(&report)?
            .iter()
            .filter(|fact| fact.as_str() == "BFP0009|services.kong.networks")
            .count(),
        1,
        "CLI deduplicates visible notes, not topology or library outcomes"
    );
    let reimported = import_compose(&fs::read_to_string(destination.join("compose.yaml"))?, "supabase")?;
    supabase_assert_kong_networks(&reimported)?;
    Ok(())
}

fn supabase_assert_kong_networks(imported: &ImportResult) -> Result<(), Box<dyn Error>> {
    let application = imported.application().ok_or("Supabase application")?;
    let kong = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "kong")
        .ok_or("Supabase Kong service")?
        .value();
    let actual = kong
        .networks()
        .iter()
        .map(|network| (network.value().network().as_str(), network.value().aliases().to_vec()))
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        ("backend", vec!["kong".to_owned()]),
        ("edge", vec!["supabase".to_owned()]),
    ]);
    assert_eq!(actual, expected, "independent backend/edge topology and aliases");
    Ok(())
}

#[test]
fn supabase_withheld_podman_report_matches_live_environment_contract() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let fixture = root.join("fixtures/scenarios/supabase-application");
    let manifest = read_manifest(&fixture)?;
    let route = manifest
        .evidence
        .iter()
        .find(|route| route.input == "podman" && route.exporter == "podman")
        .ok_or("missing Supabase Podman-to-Podman route")?;
    assert!(!route.includes_environment_values());
    // Production CLI acquisition consumes an authored read-only cassette through
    // the published PodmanLens; this is offline evidence, never a native claim.
    let output = TemporaryDirectory::new("supabase-withheld-environment-report")?;
    let report = convert_cli(
        &manifest,
        &[fixture.join("input-podman.cassette.json")],
        route,
        output.path(),
    )?;
    assert_eq!(report["status"], "success");
    validate_cli_loss_aggregate(&route.cli_losses(&fixture)?, route, &report)?;
    let expected = manifest
        .semantics
        .required_environment
        .iter()
        .map(|assignment| {
            let (service, assignment) = assignment.split_once(':').ok_or("missing environment service")?;
            let (name, _) = assignment.split_once('=').ok_or("missing environment assignment")?;
            Ok(format!("BFP0002|services.{service}.environment.{name}"))
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    let facts = report_diagnostic_facts(&report)?;
    let actual = facts
        .iter()
        .filter(|fact| fact.starts_with("BFP0002|services.") && fact.contains(".environment."))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        actual.len(),
        expected.len(),
        "one omission per independently required key"
    );
    assert_eq!(actual.into_iter().collect::<BTreeSet<_>>(), expected);
    assert!(
        !facts
            .iter()
            .any(|fact| fact.starts_with("BFP0003|services.") && fact.ends_with(".environment")),
        "redacted values cannot produce aggregate environment promotion"
    );
    let declared = Command::new("jq")
        .args(["--null-input", "--arg", "input", "podman", "--arg", "output", "podman"])
        .args(["--arg", "selection", "exact", "--arg", "resource_prefix", ""])
        .args(["--arg", "podman_acquisition", "cli", "--arg", "provisioner_mode", "cli"])
        .args([
            "--argjson",
            "include_system_network",
            "false",
            "--argjson",
            "emit_expected",
            "true",
        ])
        .arg("--from-file")
        .arg(root.join("fixtures/conformance/supabase-application/success-contract.jq"))
        .output()?;
    assert!(declared.status.success(), "live environment contract must compile");
    let declared: Vec<serde_json::Value> = serde_json::from_slice(&declared.stdout)?;
    let projection = declared
        .iter()
        .filter(|tuple| tuple["code"] == "BFP0002")
        .filter_map(|tuple| {
            tuple["subject"]
                .as_str()
                .map(|subject| (tuple, format!("BFP0002|{subject}")))
        })
        .filter(|(_, fact)| expected.contains(fact))
        .map(|(tuple, fact)| {
            assert_eq!(tuple["severity"], "warning");
            assert_eq!(tuple["decision"], "omitted");
            fact
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        projection, expected,
        "live contract must admit the real redacted importer tuples"
    );
    assert_no_protected_environment_values(&manifest, &report.to_string(), "withheld replay report")?;
    Ok(())
}

#[test]
fn reviewed_podman_cassette_requires_route_authorization_for_compose_values() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/nextcloud-application");
    let manifest = read_manifest(&fixture)?;
    validate_manifest(&manifest, &fixture)?;
    let source = fixture.join("input-podman.cassette.json");
    let route = manifest
        .evidence
        .iter()
        .find(|route| route.input == "podman" && route.exporter == "compose")
        .ok_or("missing Podman-to-Compose scenario route")?;
    assert!(route.includes_environment_values());

    let included_output = TemporaryDirectory::new("scenario-podman-included-environment")?;
    let included_report = convert_cli(&manifest, std::slice::from_ref(&source), route, included_output.path())?;
    assert_eq!(included_report["status"], "success");
    let expected = load_string_expectations(
        &route.diagnostics,
        route.diagnostics_file.as_deref(),
        &fixture,
        "reviewed Podman-to-Compose diagnostics",
    )?;
    validate_diagnostics(&expected, &report_diagnostic_facts(&included_report)?)?;
    let included_artifact = fs::read_to_string(included_output.path().join("compose.yaml"))?;
    assert!(included_artifact.contains("boxferry-test-db-password-not-secret"));

    let mut withheld = read_manifest(&fixture)?;
    let withheld_route_index = withheld
        .evidence
        .iter()
        .position(|route| route.input == "podman" && route.exporter == "compose")
        .ok_or("missing Podman-to-Compose scenario route")?;
    withheld.evidence[withheld_route_index].set_environment_values_for_test(false);
    validate_manifest(&withheld, &fixture)?;
    let withheld_output = TemporaryDirectory::new("scenario-podman-withheld-environment")?;
    let withheld_report = convert_cli(
        &withheld,
        std::slice::from_ref(&source),
        &withheld.evidence[withheld_route_index],
        withheld_output.path(),
    )?;
    assert_eq!(withheld_report["status"], "success");
    assert!(
        report_diagnostic_facts(&withheld_report)?
            .contains(&"BFC0007|services.app.environment.POSTGRES_PASSWORD".into())
    );
    assert!(
        !fs::read_to_string(withheld_output.path().join("compose.yaml"))?
            .contains("boxferry-test-db-password-not-secret")
    );
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
    let expected_diagnostics = route.cli_diagnostics(manifest, fixture)?;
    let expected_losses = route.cli_losses(fixture)?;
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
    // Retain only the neutral application needed for independent target mount order.
    let source_application = if route.exporter == "docker" {
        imported.application().cloned()
    } else {
        None
    };
    let cli_imported = imported.clone();
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
        #[cfg(feature = "docker")]
        "docker" => {
            let mut exporter = DockerExporter::new(route.docker_profile()?)?;
            if route.includes_environment_values() {
                exporter = exporter.with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral);
            }
            validate_result(
                route,
                fixture,
                &convert_imported(imported, &exporter, &target, route.policy()?)?,
            )?;
        }
        _ => return Err("registered exporter has no scenario runner".into()),
    }
    if route.cli_withheld_diagnostics_delta_file.is_some() {
        validate_cli_policy_losses(manifest, route, sources, cli_imported, &target, &expected_losses)?;
    }
    let output = TemporaryDirectory::new(&route.exporter)?;
    let report = convert_cli(manifest, sources, route, output.path())?;
    let diagnostics = report_diagnostic_facts(&report)?;
    validate_diagnostics(&expected_diagnostics, &diagnostics)?;
    if route.cli_withheld_losses_delta_file.is_some() {
        validate_cli_loss_aggregate(&expected_losses, route, &report)?;
    }
    let artifacts = assert_route_artifact_inventory(output.path(), expected_artifacts)?;
    if report["status"] != "success" {
        if route.outcome != Outcome::ExpectedRejection {
            return Err("CLI failure is not an expected rejection".into());
        }
        return Ok(validate_evidence(route, &rejected_observation())?);
    }

    let (semantics_match, reimport) = observe_native_output(
        manifest,
        route,
        &artifacts,
        output.path(),
        fixture,
        source_application.as_ref(),
    )?;
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

fn assert_route_artifact_inventory(output: &Path, mut expected: Vec<String>) -> Result<Vec<String>, Box<dyn Error>> {
    let mut artifacts = fs::read_dir(output)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("non-UTF-8 artifact"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    artifacts.sort();
    expected.sort();
    assert_eq!(artifacts, expected, "artifact inventory changed");
    Ok(artifacts)
}

fn validate_cli_policy_losses(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    sources: &[PathBuf],
    imported: ImportResult,
    target: &boxferry::TargetProfile,
    expected: &[support::LossTuple],
) -> Result<(), Box<dyn Error>> {
    let imported = if route.includes_environment_values() {
        imported
    } else {
        let imported = if manifest
            .native_inputs
            .iter()
            .any(|input| input.id == route.input && input.importer == "podman")
        {
            import_podman_cassette_with_environment_values(
                manifest,
                &route.input,
                PodmanCassette::load(sources.first().ok_or("missing Podman cassette")?)?,
                false,
            )?
        } else {
            imported
        };
        let (mut application, outcomes, diagnostics) = imported.into_parts();
        if let Some(application) = &mut application {
            application.withhold_literal_environment_values();
        }
        ImportResult::new(application, outcomes, diagnostics)
    };
    match route.exporter.as_str() {
        "compose" => validate_losses(
            expected,
            convert_imported(imported, &ComposeExporter::new()?, target, route.policy()?)?.outcomes(),
            &route.version_scope(),
        )?,
        "quadlet" => validate_losses(
            expected,
            convert_imported(imported, &quadlet_exporter(route)?, target, route.policy()?)?.outcomes(),
            &route.version_scope(),
        )?,
        "podman" => validate_losses(
            expected,
            convert_imported(imported, &PodmanExporter::new()?, target, route.policy()?)?.outcomes(),
            &route.version_scope(),
        )?,
        _ => return Err("registered exporter has no CLI policy runner".into()),
    }
    Ok(())
}

fn validate_cli_loss_aggregate(
    expected: &[support::LossTuple],
    route: &RouteExpectation,
    report: &serde_json::Value,
) -> Result<(), String> {
    let mut approximate = 0usize;
    let mut unsupported = 0usize;
    let mut invalid = 0usize;
    for loss in expected {
        match loss.decision.as_str() {
            "approximate" => approximate += loss.count,
            "unsupported" => unsupported += loss.count,
            "invalid" => invalid += loss.count,
            other => return Err(format!("unrecognized expected CLI loss decision {other}")),
        }
    }
    let fidelity = report["fidelity"]
        .as_object()
        .ok_or("CLI report lacks fidelity counts")?;
    for (name, expected_count) in [
        ("approximate", approximate),
        ("unsupported", unsupported),
        ("invalid", invalid),
        ("other", 0),
    ] {
        let actual = fidelity
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("CLI report lacks fidelity {name}"))?;
        if actual != expected_count as u64 {
            return Err(format!(
                "CLI fidelity {name} differs: expected {expected_count}, observed {actual}"
            ));
        }
    }
    let policy = report["choices"]
        .as_array()
        .ok_or("CLI report lacks choices")?
        .iter()
        .find(|choice| choice["name"] == "loss_policy")
        .and_then(|choice| choice["value"].as_str())
        .ok_or("CLI report lacks loss policy choice")?;
    if policy != route.loss_policy {
        return Err(format!(
            "CLI loss policy differs: expected {}, observed {policy}",
            route.loss_policy
        ));
    }
    Ok(())
}

#[test]
fn cli_loss_delta_and_report_reject_wrong_decision_count_and_policy() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/nextcloud-application");
    let manifest = read_manifest(&fixture)?;
    let route = manifest
        .evidence
        .iter()
        .find(|route| route.input == "podman" && route.exporter == "podman")
        .ok_or("missing Podman withholding route")?;
    let expected = route.cli_losses(&fixture)?;
    assert!(expected.iter().all(|loss| loss.subject != "services.app.environment"));

    let sandbox = TemporaryDirectory::new("cli-loss-delta-mutations")?;
    fs::write(
        sandbox.path().join("expected.podman-podman.losses.tsv"),
        fs::read(fixture.join("expected.podman-podman.losses.tsv"))?,
    )?;
    let original = fs::read_to_string(fixture.join("expected.podman-podman.withheld-losses.delta"))?;
    let first = original.lines().next().ok_or("empty authored loss delta")?;
    for mutation in [
        first.replacen("\tapproximate\t", "\tunsupported\t", 1),
        first.replace("\t1", "\t2"),
        first.trim_start_matches('-').to_owned(),
    ] {
        fs::write(
            sandbox.path().join("expected.podman-podman.withheld-losses.delta"),
            mutation,
        )?;
        let error = route
            .cli_losses(sandbox.path())
            .err()
            .ok_or("invalid CLI loss delta accepted")?;
        assert!(
            error.contains("absent from reviewed library evidence") || error.contains("needs + or -"),
            "unexpected loss mutation result: {error}"
        );
    }

    let report = serde_json::json!({
        "fidelity": {"approximate": 0, "unsupported": 0, "invalid": 0, "other": 0},
        "choices": [{"name": "loss_policy", "value": "exact"}]
    });
    let error = validate_cli_loss_aggregate(&[], route, &report)
        .err()
        .ok_or("wrong CLI loss policy accepted")?;
    assert!(error.contains("CLI loss policy differs"));
    let error = validate_cli_loss_aggregate(
        &[support::LossTuple {
            rule: "BFP0003".into(),
            subject: "services.app.environment".into(),
            decision: "approximate".into(),
            version_scope: route.version_scope(),
            count: 1,
        }],
        route,
        &serde_json::json!({
            "fidelity": {"approximate": 0, "unsupported": 0, "invalid": 0, "other": 0},
            "choices": [{"name": "loss_policy", "value": "partial"}]
        }),
    )
    .err()
    .ok_or("wrong CLI loss count accepted")?;
    assert!(error.contains("CLI fidelity approximate differs"));
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

#[test]
fn observability_live_diagnostic_matcher_is_route_specific_and_exact() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let fixture = root.join("fixtures/scenarios/observability-application");
    let manifest = read_manifest_file(&fixture.join("scenario.toml"))?;
    let podman_sources = [fixture.join("input-podman.cassette.json")];
    let exports = observability_live_exports(&manifest, &podman_sources)?;

    observability_assert_reimport_routes(&exports)?;
    observability_assert_report_adversaries(&exports.compose_report)?;
    observability_assert_live_template_contracts()?;
    Ok(())
}

struct ObservabilityLiveExports {
    compose_output: TemporaryDirectory,
    compose_report: serde_json::Value,
    quadlet_output: TemporaryDirectory,
}

fn observability_live_exports(
    manifest: &ScenarioManifest,
    podman_sources: &[PathBuf],
) -> Result<ObservabilityLiveExports, Box<dyn Error>> {
    let podman_compose = TemporaryDirectory::new("observability-podman-compose")?;
    let podman_compose_report = convert_cli(
        manifest,
        podman_sources,
        observability_route(manifest, "podman", "compose")?,
        podman_compose.path(),
    )?;
    assert_observability_diagnostic_match("podman-compose", "podman", "compose", &podman_compose_report, true)?;

    let podman_quadlet = TemporaryDirectory::new("observability-podman-quadlet")?;
    let podman_quadlet_report = convert_cli(
        manifest,
        podman_sources,
        observability_route(manifest, "podman", "quadlet")?,
        podman_quadlet.path(),
    )?;
    assert_observability_diagnostic_match("podman-quadlet", "podman", "quadlet", &podman_quadlet_report, true)?;

    Ok(ObservabilityLiveExports {
        compose_output: podman_compose,
        compose_report: podman_compose_report,
        quadlet_output: podman_quadlet,
    })
}

fn observability_assert_reimport_routes(exports: &ObservabilityLiveExports) -> Result<(), Box<dyn Error>> {
    let compose_podman = TemporaryDirectory::new("observability-compose-podman")?;
    let compose_podman_report = observability_file_conversion(
        "compose",
        "podman",
        &[exports.compose_output.path().join("compose.yaml")],
        compose_podman.path(),
    )?;
    assert_observability_diagnostic_match("compose-podman", "compose", "podman", &compose_podman_report, true)?;
    observability_assert_missing_plugin_flag_diagnostics("compose", "podman", "BFP0007", &compose_podman_report)?;

    let quadlet_sources = observability_output_files(exports.quadlet_output.path())?;
    let quadlet_podman = TemporaryDirectory::new("observability-quadlet-podman")?;
    let quadlet_podman_report =
        observability_file_conversion("quadlet", "podman", &quadlet_sources, quadlet_podman.path())?;
    assert_observability_diagnostic_match("quadlet-podman", "quadlet", "podman", &quadlet_podman_report, true)?;
    observability_assert_missing_plugin_flag_diagnostics("quadlet", "podman", "BFP0007", &quadlet_podman_report)?;

    let compose_quadlet = TemporaryDirectory::new("observability-compose-quadlet")?;
    let compose_quadlet_report = observability_file_conversion(
        "compose",
        "quadlet",
        &[exports.compose_output.path().join("compose.yaml")],
        compose_quadlet.path(),
    )?;
    assert_observability_diagnostic_match("compose-quadlet", "compose", "quadlet", &compose_quadlet_report, true)?;
    observability_assert_missing_plugin_flag_diagnostics("compose", "quadlet", "BFQ0003", &compose_quadlet_report)?;
    let mut missing_withholding = compose_quadlet_report.clone();
    missing_withholding["diagnostics"] = serde_json::json!([]);
    assert_observability_diagnostic_match(
        "compose-quadlet-missing-withholding",
        "compose",
        "quadlet",
        &missing_withholding,
        false,
    )?;
    Ok(())
}

fn observability_assert_missing_plugin_flag_diagnostics(
    input: &str,
    output: &str,
    code: &str,
    report: &serde_json::Value,
) -> Result<(), Box<dyn Error>> {
    for name in ["GF_PLUGINS_PREINSTALL_AUTO_UPDATE", "GF_PLUGINS_PREINSTALL_DISABLED"] {
        let subject = format!("services.grafana.environment.{name}");
        let mut missing = report.clone();
        let diagnostics = missing["diagnostics"]
            .as_array_mut()
            .ok_or("observability diagnostics are not an array")?;
        let original_length = diagnostics.len();
        diagnostics.retain(|diagnostic| {
            diagnostic["code"] != code
                || !diagnostic["fields"].as_array().is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| field["name"] == "subject" && field["value"] == subject)
                })
        });
        assert_eq!(
            diagnostics.len() + 1,
            original_length,
            "exactly one {name} diagnostic is required"
        );
        assert_observability_diagnostic_match(
            &format!("{input}-{output}-missing-{name}"),
            input,
            output,
            &missing,
            false,
        )?;
    }
    Ok(())
}

fn observability_assert_report_adversaries(report: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let mut wrong_decision = report.clone();
    assert!(mutate_observability_diagnostic_field(
        &mut wrong_decision,
        "decision",
        "approximated",
        "not-promoted",
    ));
    assert_observability_diagnostic_match("wrong-decision", "podman", "compose", &wrong_decision, false)?;

    let mut wrong_severity = report.clone();
    let severity = wrong_severity["diagnostics"]
        .as_array_mut()
        .and_then(|diagnostics| {
            diagnostics
                .iter_mut()
                .find(|diagnostic| diagnostic["severity"] == "warning")
        })
        .ok_or("observability report has no warning diagnostic")?;
    severity["severity"] = serde_json::Value::String("error".into());
    assert_observability_diagnostic_match("wrong-severity", "podman", "compose", &wrong_severity, false)?;

    let mut wrong_policy = report.clone();
    assert!(mutate_observability_diagnostic_field(
        &mut wrong_policy,
        "required_loss_policy",
        "partial",
        "exact",
    ));
    assert_observability_diagnostic_match("wrong-policy", "podman", "compose", &wrong_policy, false)?;

    let mut duplicate_field = report.clone();
    assert!(duplicate_observability_diagnostic_field(
        &mut duplicate_field,
        "decision"
    ));
    assert_observability_diagnostic_match("duplicate-field", "podman", "compose", &duplicate_field, false)?;

    let mut missing_diagnostic = report.clone();
    assert!(
        missing_diagnostic["diagnostics"]
            .as_array_mut()
            .is_some_and(|diagnostics| diagnostics.pop().is_some())
    );
    assert_observability_diagnostic_match("missing-diagnostic", "podman", "compose", &missing_diagnostic, false)?;

    let mut injected_diagnostic = report.clone();
    let duplicate = injected_diagnostic["diagnostics"]
        .as_array()
        .and_then(|diagnostics| diagnostics.first())
        .cloned()
        .ok_or("observability report has no diagnostic to duplicate")?;
    injected_diagnostic["diagnostics"]
        .as_array_mut()
        .ok_or("observability report diagnostics are not an array")?
        .push(duplicate);
    assert_observability_diagnostic_match("injected-diagnostic", "podman", "compose", &injected_diagnostic, false)?;

    let mut wrong_volume_subject = report.clone();
    assert!(mutate_observability_subject_prefix(
        &mut wrong_volume_subject,
        "volumes.",
        "volume."
    ));
    assert_observability_diagnostic_match(
        "wrong-volume-subject",
        "podman",
        "compose",
        &wrong_volume_subject,
        false,
    )?;
    assert_observability_diagnostic_match("wrong-source-route", "compose", "podman", report, false)?;
    assert!(
        !serde_json::to_string(report)?.contains("boxferry-public-observability-admin-canary"),
        "the diagnostic report leaked the protected-value canary"
    );
    observability_assert_malformed_report_adversaries(report)
}

fn observability_assert_malformed_report_adversaries(report: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let mut non_array_diagnostics = report.clone();
    non_array_diagnostics["diagnostics"] = serde_json::json!({});
    assert_observability_diagnostic_match(
        "non-array-diagnostics",
        "podman",
        "compose",
        &non_array_diagnostics,
        false,
    )?;

    let mut non_array_fields = report.clone();
    non_array_fields["diagnostics"][0]["fields"] = serde_json::json!({});
    assert_observability_diagnostic_match("non-array-fields", "podman", "compose", &non_array_fields, false)?;

    let mut non_string_subject = report.clone();
    non_string_subject["diagnostics"][0]["fields"][0]["value"] = serde_json::json!(7);
    assert_observability_diagnostic_match("non-string-subject", "podman", "compose", &non_string_subject, false)?;

    let mut duplicate_subject = report.clone();
    let subject = duplicate_subject["diagnostics"][0]["fields"][0].clone();
    duplicate_subject["diagnostics"][0]["fields"]
        .as_array_mut()
        .ok_or("observability fields are not an array")?
        .push(subject);
    assert_observability_diagnostic_match("duplicate-subject", "podman", "compose", &duplicate_subject, false)?;

    let mut missing_subject = report.clone();
    missing_subject["diagnostics"][0]["fields"]
        .as_array_mut()
        .ok_or("observability fields are not an array")?
        .retain(|field| field["name"] != "subject");
    assert_observability_diagnostic_match("missing-subject", "podman", "compose", &missing_subject, false)?;

    let mut trailing_malformed = report.clone();
    trailing_malformed["diagnostics"]
        .as_array_mut()
        .ok_or("observability report diagnostics are not an array")?
        .push(serde_json::json!({"code": 7, "severity": "warning", "fields": []}));
    assert_observability_diagnostic_match("trailing-malformed", "podman", "compose", &trailing_malformed, false)
}

type ObservabilityTemplateContract = (
    &'static str,
    &'static str,
    &'static str,
    usize,
    &'static [(&'static str, usize)],
);

fn observability_assert_live_template_contracts() -> Result<(), Box<dyn Error>> {
    let expected_contracts: [ObservabilityTemplateContract; 12] = [
        (
            "cli",
            "exact",
            "compose",
            237,
            &[("BFC0007", 11), ("BFP0002", 52), ("BFP0003", 156), ("BFP0009", 18)],
        ),
        (
            "cli",
            "exact",
            "quadlet",
            227,
            &[("BFP0002", 52), ("BFP0003", 156), ("BFP0009", 18), ("BFQ0003", 1)],
        ),
        (
            "cli",
            "exact",
            "podman",
            330,
            &[("BFP0002", 101), ("BFP0003", 150), ("BFP0009", 18), ("BFP0007", 61)],
        ),
        (
            "cli",
            "all",
            "compose",
            257,
            &[("BFC0007", 12), ("BFP0002", 59), ("BFP0003", 166), ("BFP0009", 20)],
        ),
        (
            "cli",
            "all",
            "quadlet",
            246,
            &[("BFP0002", 59), ("BFP0003", 166), ("BFP0009", 20), ("BFQ0003", 1)],
        ),
        (
            "cli",
            "all",
            "podman",
            368,
            &[("BFP0002", 117), ("BFP0003", 159), ("BFP0009", 20), ("BFP0007", 72)],
        ),
        (
            "compose",
            "exact",
            "compose",
            226,
            &[("BFC0007", 4), ("BFP0002", 46), ("BFP0003", 158), ("BFP0009", 18)],
        ),
        (
            "compose",
            "exact",
            "quadlet",
            223,
            &[("BFP0002", 46), ("BFP0003", 158), ("BFP0009", 18), ("BFQ0003", 1)],
        ),
        (
            "compose",
            "exact",
            "podman",
            326,
            &[("BFP0002", 95), ("BFP0003", 152), ("BFP0009", 18), ("BFP0007", 61)],
        ),
        (
            "compose",
            "all",
            "compose",
            246,
            &[("BFC0007", 5), ("BFP0002", 53), ("BFP0003", 168), ("BFP0009", 20)],
        ),
        (
            "compose",
            "all",
            "quadlet",
            242,
            &[("BFP0002", 53), ("BFP0003", 168), ("BFP0009", 20), ("BFQ0003", 1)],
        ),
        (
            "compose",
            "all",
            "podman",
            364,
            &[("BFP0002", 111), ("BFP0003", 161), ("BFP0009", 20), ("BFP0007", 72)],
        ),
    ];
    for contract in expected_contracts {
        observability_assert_template_contract(contract)?;
    }
    observability_assert_template_selection_contract()?;
    observability_assert_template_files()?;
    observability_assert_reimport_template_contracts()?;
    assert_observability_expected_generation_failure_rejected()
}

fn observability_assert_reimport_template_contracts() -> Result<(), Box<dyn Error>> {
    let expectations = [
        ("cli", "exact", "compose", "compose", 0),
        ("cli", "exact", "compose", "quadlet", 1),
        ("cli", "exact", "compose", "podman", 58),
        ("cli", "exact", "quadlet", "compose", 8),
        ("cli", "exact", "quadlet", "quadlet", 0),
        ("cli", "exact", "quadlet", "podman", 65),
        ("cli", "label", "compose", "compose", 0),
        ("cli", "label", "compose", "quadlet", 1),
        ("cli", "label", "compose", "podman", 58),
        ("cli", "label", "quadlet", "compose", 8),
        ("cli", "label", "quadlet", "quadlet", 0),
        ("cli", "label", "quadlet", "podman", 65),
        ("cli", "all", "compose", "compose", 0),
        ("cli", "all", "compose", "quadlet", 1),
        ("cli", "all", "compose", "podman", 68),
        ("cli", "all", "quadlet", "compose", 9),
        ("cli", "all", "quadlet", "quadlet", 0),
        ("cli", "all", "quadlet", "podman", 76),
        ("compose", "exact", "compose", "compose", 0),
        ("compose", "exact", "compose", "quadlet", 1),
        ("compose", "exact", "compose", "podman", 58),
        ("compose", "exact", "quadlet", "compose", 1),
        ("compose", "exact", "quadlet", "quadlet", 0),
        ("compose", "exact", "quadlet", "podman", 58),
        ("compose", "label", "compose", "compose", 0),
        ("compose", "label", "compose", "quadlet", 1),
        ("compose", "label", "compose", "podman", 58),
        ("compose", "label", "quadlet", "compose", 1),
        ("compose", "label", "quadlet", "quadlet", 0),
        ("compose", "label", "quadlet", "podman", 58),
        ("compose", "all", "compose", "compose", 0),
        ("compose", "all", "compose", "quadlet", 1),
        ("compose", "all", "compose", "podman", 68),
        ("compose", "all", "quadlet", "compose", 2),
        ("compose", "all", "quadlet", "quadlet", 0),
        ("compose", "all", "quadlet", "podman", 69),
    ];
    for (mode, selection, input, output, expected_rows) in expectations {
        let actual = observability_live_reimport_expected_diagnostics_for(
            mode,
            selection,
            input,
            output,
            "live-observability-",
        )?;
        assert_eq!(
            actual.lines().count(),
            expected_rows,
            "{mode}/{selection}/{input}-to-{output}"
        );
        assert!(
            !actual.contains("live-observability-observability-"),
            "{mode}/{selection}/{input}-to-{output} duplicated the application prefix"
        );
        assert!(!actual.contains("boxferry-public-observability-admin-canary"));
        observability_assert_reimport_alias_contract(&actual, mode, selection, input, output);
        observability_assert_environment_contract(&actual, selection, output, false)?;
    }
    for mode in ["cli", "compose"] {
        for input in ["compose", "quadlet"] {
            for output in ["compose", "quadlet", "podman"] {
                assert_eq!(
                    observability_live_reimport_expected_diagnostics_for(
                        mode,
                        "exact",
                        input,
                        output,
                        "live-observability-",
                    )?,
                    observability_live_reimport_expected_diagnostics_for(
                        mode,
                        "label",
                        input,
                        output,
                        "live-observability-",
                    )?,
                    "{mode}/{input}-to-{output} exact and label contracts differ",
                );
            }
        }
    }
    observability_reimport_template_dimensions_fail_closed()
}

fn observability_assert_reimport_alias_contract(
    diagnostics: &str,
    mode: &str,
    selection: &str,
    input: &str,
    output: &str,
) {
    assert!(!diagnostics.lines().any(|line| {
        ["BFP0002\t", "BFP0003\t", "BFP0009\t"]
            .iter()
            .any(|code| line.starts_with(code))
            || line.contains(".logging\t")
    }));
    assert_eq!(
        diagnostics.lines().filter(|line| line.contains(".aliases\t")).count(),
        0,
        "{mode}/{selection}/{input}-to-{output} must not recreate importer alias-promotion diagnostics"
    );
    let expected_quadlet_loss = "BFQ0003\tservices.live-observability-grafana.networks\twarning\t\t";
    assert_eq!(
        diagnostics
            .lines()
            .filter(|line| *line == expected_quadlet_loss)
            .count(),
        usize::from(input == "compose" && output == "quadlet"),
        "{mode}/{selection}/{input}-to-{output} Grafana Quadlet alias-loss row count"
    );
}

fn observability_assert_template_contract(
    (mode, selection, output, expected_rows, expected_codes): ObservabilityTemplateContract,
) -> Result<(), Box<dyn Error>> {
    let diagnostics = observability_live_expected_diagnostics_for(mode, selection, output, "live-observability-")?;
    assert_eq!(
        diagnostics.lines().count(),
        expected_rows,
        "{mode}/{selection}/{output} rows"
    );
    for (code, expected_count) in expected_codes {
        assert_eq!(
            diagnostics.lines().filter(|line| line.starts_with(code)).count(),
            *expected_count,
            "{mode}/{selection}/{output} {code} rows"
        );
    }
    observability_assert_promoted_alias_contract(&diagnostics, mode, selection, output);
    observability_assert_reconstruction_contract(&diagnostics, selection);
    observability_assert_environment_contract(&diagnostics, selection, output, true)?;
    let label = observability_live_expected_diagnostics_for(mode, "label", output, "live-observability-")?;
    let exact = observability_live_expected_diagnostics_for(mode, "exact", output, "live-observability-")?;
    assert_eq!(exact, label, "{mode}/{output} exact and label contracts differ");
    Ok(())
}

fn observability_assert_environment_contract(
    diagnostics: &str,
    selection: &str,
    output: &str,
    native_import: bool,
) -> Result<(), Box<dyn Error>> {
    let mut services = vec![
        "alloy",
        "grafana",
        "log-producer",
        "loki",
        "metrics-producer",
        "prometheus",
    ];
    if selection == "all" {
        services.push("boundary-peer");
    }
    let aggregate_count = usize::from(native_import && output != "podman");
    for service in services {
        let expected =
            format!("BFP0003\tservices.live-observability-{service}.environment\twarning\tapproximated\tapproximate");
        assert_eq!(
            diagnostics.lines().filter(|line| *line == expected).count(),
            aggregate_count
        );
    }
    let root = repository_root().join("fixtures/conformance/observability-application/diagnostics");
    let mut files = vec![("podman-export.tsv", 49)];
    if selection == "all" {
        files.push(("all-podman-export.tsv", 9));
    }
    let mut key_count = 0;
    for (file, expected_keys) in files {
        let template = fs::read_to_string(root.join(file))?;
        let subjects = template
            .lines()
            .filter_map(|line| line.split('\t').nth(1))
            .filter(|subject| subject.contains(".environment."))
            .collect::<Vec<_>>();
        assert_eq!(subjects.len(), expected_keys, "{file} environment key vocabulary");
        key_count += subjects.len();
        for subject in subjects {
            let subject = subject.replace("{{resource_prefix}}", "live-observability-");
            for (code, expected_count) in [
                ("BFP0002", usize::from(native_import && output == "podman")),
                ("BFP0007", usize::from(output == "podman")),
            ] {
                let expected = format!("{code}\t{subject}\twarning\tomitted\tpartial");
                assert_eq!(
                    diagnostics.lines().filter(|line| *line == expected).count(),
                    expected_count,
                    "{subject}"
                );
            }
        }
    }
    for code in ["BFP0002", "BFP0007"] {
        let expected = key_count * usize::from(output == "podman" && (code == "BFP0007" || native_import));
        assert_eq!(
            diagnostics
                .lines()
                .filter(|line| line.starts_with(code) && line.contains(".environment."))
                .count(),
            expected,
        );
    }
    Ok(())
}

fn observability_assert_reconstruction_contract(diagnostics: &str, selection: &str) {
    let mut subjects = [
        "alloy.mounts[0]",
        "alloy.mounts[1]",
        "grafana.mounts[0]",
        "log-producer.mounts[0]",
        "loki.mounts[0]",
        "prometheus.mounts[0]",
    ]
    .map(|subject| format!("services.live-observability-{subject}"))
    .to_vec();
    for service in [
        "alloy",
        "grafana",
        "log-producer",
        "loki",
        "metrics-producer",
        "prometheus",
    ] {
        for field in ["networks", "restart_policy"] {
            subjects.push(format!("services.live-observability-{service}.{field}"));
        }
    }
    if selection == "all" {
        for field in ["networks", "restart_policy"] {
            subjects.push(format!("services.live-observability-boundary-peer.{field}"));
        }
    }
    for subject in &subjects {
        let expected = format!("BFP0009\t{subject}\tnote\treconstructed\t");
        assert_eq!(
            diagnostics.lines().filter(|line| *line == expected).count(),
            1,
            "{subject}"
        );
        assert!(
            !diagnostics
                .lines()
                .any(|line| line.starts_with(&format!("BFP0003\t{subject}\t")))
        );
    }
    assert_eq!(
        diagnostics.lines().filter(|line| line.starts_with("BFP0009\t")).count(),
        subjects.len(),
    );
    let mut ownership = vec!["networks.live-observability-backend.ownership".to_owned()];
    for volume in [
        "alloy-data",
        "grafana-data",
        "loki-data",
        "prometheus-data",
        "telemetry-logs",
    ] {
        ownership.push(format!("volumes.live-observability-{volume}.ownership"));
    }
    if selection == "all" {
        ownership.push("networks.podman.ownership".to_owned());
    }
    for subject in &ownership {
        let expected = format!("BFP0003\t{subject}\twarning\tinferred-application-ownership\tapproximate");
        assert_eq!(
            diagnostics.lines().filter(|line| *line == expected).count(),
            1,
            "{subject}"
        );
    }
    assert_eq!(
        diagnostics.lines().filter(|line| line.contains(".ownership\t")).count(),
        ownership.len(),
    );
    assert!(!diagnostics.contains("networks.live-observability-edge.ownership\t"));
}

fn observability_assert_promoted_alias_contract(diagnostics: &str, mode: &str, selection: &str, output: &str) {
    let mut attachments = vec![
        ("grafana", "backend"),
        ("grafana", "edge"),
        ("loki", "backend"),
        ("metrics-producer", "backend"),
        ("prometheus", "backend"),
    ];
    if mode == "compose" {
        attachments.extend([("alloy", "backend"), ("log-producer", "backend")]);
    }
    for &(service, network) in &attachments {
        let expected = format!(
            "BFP0003\tservices.live-observability-{service}.networks.live-observability-{network}.aliases\twarning\tapproximated\tapproximate"
        );
        assert_eq!(
            diagnostics.lines().filter(|line| *line == expected).count(),
            1,
            "{mode}/{selection}/{output} must contain exactly one {service}/{network} alias promotion"
        );
    }
    assert_eq!(
        diagnostics.lines().filter(|line| line.contains(".aliases\t")).count(),
        attachments.len(),
        "{mode}/{selection}/{output} alias-promotion row count"
    );

    let expected_quadlet_loss = "BFQ0003\tservices.live-observability-grafana.networks\twarning\t\t";
    assert_eq!(
        diagnostics
            .lines()
            .filter(|line| *line == expected_quadlet_loss)
            .count(),
        usize::from(output == "quadlet"),
        "{mode}/{selection}/{output} Grafana Quadlet alias-loss row count"
    );
}

fn observability_assert_template_selection_contract() -> Result<(), Box<dyn Error>> {
    let exact = observability_live_expected_diagnostics_for("cli", "exact", "quadlet", "live-observability-")?;
    let all = observability_live_expected_diagnostics_for("cli", "all", "quadlet", "live-observability-")?;
    assert!(!exact.contains("boundary-peer"));
    assert_eq!(all.lines().filter(|line| line.contains("boundary-peer")).count(), 12);
    assert_eq!(all.lines().count() - exact.lines().count(), 19);

    let temporary = TemporaryDirectory::new("observability-unsafe-prefix")?;
    let unsafe_prefix = Command::new("bash")
        .args([
            "-c",
            "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics live-native-export cli exact podman compose \"$3\" \"$4\"",
            "observability-unsafe-prefix",
        ])
        .arg(repository_root())
        .arg(repository_root().join("scripts/lib/observability-application.sh"))
        .arg("unsafe/prefix-")
        .arg(temporary.path().join("expected.tsv"))
        .output()?;
    assert!(
        !unsafe_prefix.status.success(),
        "unsafe resource prefix must not be substituted"
    );
    Ok(())
}

fn observability_assert_template_files() -> Result<(), Box<dyn Error>> {
    let template_root = repository_root().join("fixtures/conformance/observability-application/diagnostics");
    for (name, expected_rows) in [
        ("base-compose-provisioned.tsv", 214),
        ("non-podman-environment-promotions.tsv", 6),
        ("podman-import-withheld-environment.tsv", 49),
        ("all-non-podman-environment-promotion.tsv", 1),
        ("all-podman-import-withheld-environment.tsv", 9),
        ("compose-service-identities.tsv", 2),
        ("cli-creation-evidence.tsv", 6),
        ("compose-export-network.tsv", 4),
        ("cli-compose-export-dependencies.tsv", 7),
        ("podman-export.tsv", 61),
        ("all-importer.tsv", 18),
        ("all-compose-export.tsv", 1),
        ("all-podman-export.tsv", 11),
        ("quadlet-network-alias.tsv", 1),
        ("reimport-compose-podman.tsv", 58),
        ("reimport-compose-podman-all.tsv", 10),
        ("reimport-quadlet-compose.tsv", 1),
        ("reimport-quadlet-compose-cli.tsv", 7),
        ("reimport-quadlet-compose-all.tsv", 1),
        ("reimport-quadlet-podman.tsv", 58),
        ("reimport-quadlet-podman-cli.tsv", 7),
        ("reimport-quadlet-podman-all.tsv", 11),
    ] {
        let template = fs::read_to_string(template_root.join(name))?;
        assert_eq!(template.lines().count(), expected_rows, "{name} row count");
        assert!(
            !template.contains("boxferry-public-observability-admin-canary"),
            "{name} leaked protected-value canary"
        );
        for (line_number, line) in template.lines().enumerate() {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 5, "{name}:{} must have five fields", line_number + 1);
            let subject_without_marker = fields[1].replace("{{resource_prefix}}", "");
            assert!(
                !subject_without_marker.contains('{') && !subject_without_marker.contains('}'),
                "{name}:{} contains an unsafe template marker",
                line_number + 1
            );
        }
    }
    for (label, malformed) in [
        ("unknown-code", "BFP9999\tsubject\twarning\tomitted\tpartial\n"),
        ("invalid-tuple", "BFP0003\tsubject\twarning\tomitted\tpartial\n"),
        (
            "reconstruction-warning",
            "BFP0009\tsubject\twarning\treconstructed\t-\n",
        ),
        ("reconstruction-decision", "BFP0009\tsubject\tnote\tapproximated\t-\n"),
        (
            "reconstruction-policy",
            "BFP0009\tsubject\tnote\treconstructed\tapproximate\n",
        ),
        (
            "ownership-note",
            "BFP0003\tsubject\tnote\tinferred-application-ownership\tapproximate\n",
        ),
        (
            "ownership-policy",
            "BFP0003\tsubject\twarning\tinferred-application-ownership\t-\n",
        ),
        ("implicit-native-empty", "BFC0007\tsubject\twarning\t\t\n"),
        (
            "duplicate-marker",
            "BFP0002\tcontainer:{{resource_prefix}}{{resource_prefix}}peer\twarning\tomitted\tpartial\n",
        ),
        (
            "unbalanced-index",
            "BFP0003\tservices.{{resource_prefix}}peer.mounts[0\twarning\tnot-promoted\tpartial\n",
        ),
        ("missing-field", "BFP0002\tsubject\twarning\tomitted\n"),
        ("extra-field", "BFP0002\tsubject\twarning\tomitted\tpartial\textra\n"),
        ("empty-template", ""),
    ] {
        assert_observability_template_rejected(label, malformed)?;
    }
    Ok(())
}

fn observability_route<'a>(
    manifest: &'a ScenarioManifest,
    input: &str,
    output: &str,
) -> Result<&'a RouteExpectation, Box<dyn Error>> {
    manifest
        .evidence
        .iter()
        .find(|route| route.input == input && route.exporter == output)
        .ok_or_else(|| format!("missing observability {input}-to-{output} route").into())
}

fn observability_file_conversion(
    input: &str,
    output: &str,
    sources: &[PathBuf],
    destination: &Path,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let report_directory =
        TemporaryDirectory::new_in(&repository_root().join("target"), "observability-reimport-report")?;
    let report = report_directory.path().join("report.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command.args(["convert", input, output]);
    for source in sources {
        command.arg("--input-file").arg(source);
    }
    if input == "compose" {
        command.args(["--project-name", "observability-application"]);
    } else {
        command.args(["--application-name", "observability-application"]);
    }
    command.args(["--loss-policy", "partial"]);
    if output == "podman" {
        command.args(["--podman-target-context", "rootless", "--podman-max-version", "6.1.0"]);
    }
    command
        .arg("--output-directory")
        .arg(destination)
        .args(["--console-format", "json", "--report-file"])
        .arg(&report);
    let result = command.output()?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err(format!(
            "observability {input}-to-{output} reimport failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&fs::read(report)?)?)
}

fn observability_output_files(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = fs::read_dir(directory)?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    files.retain(|path| path.is_file());
    files.sort();
    Ok(files)
}

fn assert_observability_diagnostic_match(
    label: &str,
    source: &str,
    output: &str,
    report: &serde_json::Value,
    expected_success: bool,
) -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-diagnostic-match")?;
    let report_path = temporary.path().join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(report)?)?;
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    let result = Command::new("bash")
        .args([
            "-c",
            "repository_root=$1; source \"$2\"; observability_assert_reviewed_diagnostics offline-scenario cli exact \"$3\" \"$4\" \"\" \"$5\"",
            "observability-diagnostic-match",
        ])
        .arg(&root)
        .arg(&helper)
        .arg(source)
        .arg(output)
        .arg(&report_path)
        .output()?;
    if result.status.success() != expected_success {
        return Err(format!(
            "observability matcher result for {label} differed: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(())
}

fn mutate_observability_diagnostic_field(report: &mut serde_json::Value, name: &str, old: &str, new: &str) -> bool {
    let Some(diagnostics) = report["diagnostics"].as_array_mut() else {
        return false;
    };
    for diagnostic in diagnostics {
        let Some(fields) = diagnostic["fields"].as_array_mut() else {
            continue;
        };
        if let Some(field) = fields
            .iter_mut()
            .find(|field| field["name"] == name && field["value"] == old)
        {
            field["value"] = serde_json::Value::String(new.to_owned());
            return true;
        }
    }
    false
}

fn duplicate_observability_diagnostic_field(report: &mut serde_json::Value, name: &str) -> bool {
    let Some(diagnostics) = report["diagnostics"].as_array_mut() else {
        return false;
    };
    for diagnostic in diagnostics {
        let Some(fields) = diagnostic["fields"].as_array_mut() else {
            continue;
        };
        let Some(field) = fields.iter().find(|field| field["name"] == name).cloned() else {
            continue;
        };
        fields.push(field);
        return true;
    }
    false
}

fn mutate_observability_subject_prefix(report: &mut serde_json::Value, old: &str, new: &str) -> bool {
    let Some(diagnostics) = report["diagnostics"].as_array_mut() else {
        return false;
    };
    for diagnostic in diagnostics {
        let Some(fields) = diagnostic["fields"].as_array_mut() else {
            continue;
        };
        if let Some(field) = fields.iter_mut().find(|field| {
            field["name"] == "subject" && field["value"].as_str().is_some_and(|value| value.starts_with(old))
        }) {
            let Some(value) = field["value"].as_str() else {
                continue;
            };
            field["value"] = serde_json::Value::String(value.replacen(old, new, 1));
            return true;
        }
    }
    false
}

fn observability_live_expected_diagnostics_for(
    mode: &str,
    selection: &str,
    output: &str,
    prefix: &str,
) -> Result<String, Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-live-diagnostics")?;
    let destination = temporary.path().join("expected.tsv");
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    let result = Command::new("bash")
        .args([
            "-c",
            "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics live-native-export \"$3\" \"$4\" podman \"$5\" \"$6\" \"$7\"",
            "observability-live-diagnostics",
        ])
        .arg(&root)
        .arg(&helper)
        .arg(mode)
        .arg(selection)
        .arg(output)
        .arg(prefix)
        .arg(&destination)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "live diagnostic derivation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(fs::read_to_string(destination)?)
}

fn observability_live_reimport_expected_diagnostics_for(
    mode: &str,
    selection: &str,
    input: &str,
    output: &str,
    prefix: &str,
) -> Result<String, Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-live-reimport-diagnostics")?;
    let destination = temporary.path().join("expected.tsv");
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    let result = Command::new("bash")
        .args([
            "-c",
            "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics live-reimport \"$3\" \"$4\" \"$5\" \"$6\" \"$7\" \"$8\"",
            "observability-live-reimport-diagnostics",
        ])
        .arg(&root)
        .arg(&helper)
        .arg(mode)
        .arg(selection)
        .arg(input)
        .arg(output)
        .arg(prefix)
        .arg(&destination)
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "live reimport diagnostic derivation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    Ok(fs::read_to_string(destination)?)
}

fn observability_reimport_template_dimensions_fail_closed() -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-live-reimport-invalid-dimensions")?;
    let destination = temporary.path().join("expected.tsv");
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    for (scope, mode, selection, input, output) in [
        ("unsupported", "cli", "exact", "compose", "podman"),
        ("live-reimport", "invalid", "exact", "compose", "podman"),
        ("live-reimport", "cli", "invalid", "compose", "podman"),
        ("live-reimport", "cli", "exact", "podman", "podman"),
        ("live-reimport", "cli", "exact", "compose", "invalid"),
        ("live-native-export", "cli", "exact", "compose", "podman"),
        ("offline-scenario", "invalid", "exact", "podman", "compose"),
        ("offline-scenario", "cli", "invalid", "podman", "compose"),
    ] {
        let result = Command::new("bash")
            .args([
                "-c",
                "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics \"$3\" \"$4\" \"$5\" \"$6\" \"$7\" live-observability- \"$8\"",
                "observability-live-reimport-invalid-dimensions",
            ])
            .arg(&root)
            .arg(&helper)
            .arg(scope)
            .arg(mode)
            .arg(selection)
            .arg(input)
            .arg(output)
            .arg(&destination)
            .output()?;
        assert!(
            !result.status.success(),
            "accepted {scope}/{mode}/{selection}/{input}-to-{output}"
        );
    }
    let unwritable_destination = temporary.path().join("missing-parent/expected.tsv");
    for output in ["compose", "podman"] {
        let result = Command::new("bash")
            .args([
                "-c",
                "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics live-reimport cli exact compose \"$3\" live-observability- \"$4\"",
                "observability-live-reimport-unwritable-destination",
            ])
            .arg(&root)
            .arg(&helper)
            .arg(output)
            .arg(&unwritable_destination)
            .output()?;
        assert!(!result.status.success(), "accepted an unwritable {output} destination");
    }
    Ok(())
}

fn assert_observability_template_rejected(label: &str, contents: &str) -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-malformed-live-diagnostics")?;
    let template = temporary.path().join("template.tsv");
    let destination = temporary.path().join("expected.tsv");
    fs::write(&template, contents)?;
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    let result = Command::new("bash")
        .args([
            "-c",
            "source \"$1\"; observability_append_live_diagnostic_template \"$2\" live-observability- \"$3\"",
            "observability-malformed-live-diagnostics",
        ])
        .arg(&helper)
        .arg(&template)
        .arg(&destination)
        .output()?;
    assert!(
        !result.status.success(),
        "malformed live diagnostic template `{label}` was accepted"
    );
    Ok(())
}

fn assert_observability_expected_generation_failure_rejected() -> Result<(), Box<dyn Error>> {
    let temporary = TemporaryDirectory::new("observability-expected-generation-failure")?;
    let report = temporary.path().join("report.json");
    fs::write(&report, br#"{"diagnostics":[]}"#)?;
    let root = repository_root();
    let helper = root.join("scripts/lib/observability-application.sh");
    let result = Command::new("bash")
        .args([
            "-c",
            "repository_root=$1; source \"$2\"; observability_write_expected_diagnostics() { : > \"$7\"; return 2; }; observability_assert_reviewed_diagnostics offline-scenario cli exact podman compose \"\" \"$3\"",
            "observability-expected-generation-failure",
        ])
        .arg(&root)
        .arg(&helper)
        .arg(&report)
        .output()?;
    assert!(
        !result.status.success(),
        "expected-template generation failure was ignored"
    );
    Ok(())
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
    source_application: Option<&boxferry::Application>,
) -> Result<(bool, Dimension), Box<dyn Error>> {
    #[cfg(not(feature = "docker"))]
    let _ = source_application;
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
            let reimported = reimport.application().ok_or("Compose reimport failed")?;
            if route.includes_environment_values() {
                validate_neutral_application(
                    manifest,
                    reimported,
                    &route.environment_order,
                    &route.reimport_semantic_gaps,
                )?;
            } else {
                validate_withheld_reimport(
                    manifest,
                    reimported,
                    &route.environment_order,
                    &route.reimport_semantic_gaps,
                )?;
            }
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
            let reimported = reimport.application().ok_or("Quadlet reimport failed")?;
            if route.includes_environment_values() {
                validate_neutral_application(
                    manifest,
                    reimported,
                    &route.environment_order,
                    &route.reimport_semantic_gaps,
                )?;
            } else {
                validate_withheld_reimport(
                    manifest,
                    reimported,
                    &route.environment_order,
                    &route.reimport_semantic_gaps,
                )?;
            }
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
        #[cfg(feature = "docker")]
        "docker" => {
            let observed_gaps = docker_semantics(
                manifest,
                route,
                output,
                source_application.ok_or("Docker source application missing")?,
            )?;
            validate_diagnostics(&route.semantic_gaps, &observed_gaps)?;
            (observed_gaps.is_empty(), Dimension::not_applicable(NO_DOCKER_REIMPORT))
        }
        _ => return Err("missing native validator".into()),
    })
}

#[cfg(feature = "docker")]
struct DockerPlanInputs<'a> {
    requests: &'a [serde_json::Value],
    api_prefix: String,
    prerequisite_identities: BTreeSet<(&'a str, &'a str)>,
    bind_prerequisites: BTreeMap<(&'a str, usize), DockerBindPrerequisite<'a>>,
    network_internal: BTreeMap<&'a str, bool>,
}

#[cfg(feature = "docker")]
struct DockerBindPrerequisite<'a> {
    identity: &'a str,
    mount_index: usize,
    source: &'a str,
    target: &'a str,
    read_only: bool,
    relabel: &'a str,
}

#[cfg(feature = "docker")]
struct DockerPrerequisites<'a> {
    identities: BTreeSet<(&'a str, &'a str)>,
    binds: BTreeMap<(&'a str, usize), DockerBindPrerequisite<'a>>,
    network_internal: BTreeMap<&'a str, bool>,
}

#[cfg(feature = "docker")]
fn docker_closed_fields(value: &serde_json::Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected.iter().copied().collect()
    })
}

/// Independently authored test row; it does not come from native rendering or capability evidence.
#[cfg(feature = "docker")]
fn authored_docker_bind_row(
    identity: &str,
    mount_index: usize,
    source: &str,
    target: &str,
    read_only: bool,
    relabel: &str,
    reference: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kind":"bind_source", "identity":identity, "mount_index":mount_index.to_string(), "reference":reference,
        "source":source, "target":target, "read_only":read_only, "relabel":relabel,
        "source_conditions":["exists","type_reviewed","contents_reviewed","ownership_reviewed","permissions_reviewed"],
        "selinux_effect":"unverified",
        "selinux_conditions":["daemon_selinux_enabled","container_mount_label_present","policy_filesystem_support","relabel_authority"]
    })
}

#[cfg(feature = "docker")]
fn validate_authored_docker_bind_plan(
    manifest: &ScenarioManifest,
    application: &boxferry::Application,
    plan: &serde_json::Value,
) -> Result<(), Box<dyn Error>> {
    // This pure seam checks bind association and explicit external expectations, not native admission or an application run.
    let parsed = parse_docker_prerequisites(plan)?;
    validate_docker_external_internal_expectations(manifest, &parsed.network_internal)?;
    let inputs = DockerPlanInputs {
        requests: plan["requests"].as_array().ok_or("authored bind requests missing")?,
        api_prefix: "/v1.56/".into(),
        prerequisite_identities: parsed.identities,
        bind_prerequisites: parsed.binds,
        network_internal: parsed.network_internal,
    };
    let index = index_docker_requests(inputs.requests, &inputs.api_prefix)?;
    validate_docker_bind_mounts(&expected_docker_bind_mounts(manifest, application)?, &inputs, &index)
}

#[cfg(feature = "docker")]
fn synthetic_docker_bind_contract(
    read_only: bool,
    relabel: &str,
    literal: &str,
) -> Result<(ScenarioManifest, boxferry::Application, serde_json::Value), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/nextcloud-application");
    let mut manifest = read_manifest_file(&fixture.join("scenario.toml"))?;
    manifest.semantics.required_bind_mounts = serde_json::from_value(serde_json::json!([{
        "service": "app",
        "source": "/fake/private-source-canary",
        "target": "/data",
        "read-only": read_only,
        "selinux-relabel": relabel,
    }]))?;
    manifest.semantics.required_runtime_names = vec!["app:native-app".into()];
    let mut application = boxferry::Application::new(Identifier::new("authored-bind-control")?);
    let mut service = boxferry::Service::new(Identifier::new("app")?);
    service.add_mount(boxferry::Sourced::generated(boxferry::Mount::new(
        boxferry::MountSource::Volume(Identifier::new("state")?),
        "/state",
        false,
    )?));
    let mut bind = boxferry::Mount::new(
        boxferry::MountSource::HostPath("/fake/private-source-canary".into()),
        "/data",
        read_only,
    )?;
    bind.set_selinux_relabel(match relabel {
        "shared" => boxferry::SelinuxRelabel::Shared,
        "private" => boxferry::SelinuxRelabel::Private,
        _ => return Err("unreviewed authored relabel control".into()),
    });
    service.add_mount(boxferry::Sourced::generated(bind));
    application.add_service(boxferry::Sourced::generated(service))?;
    let plan = serde_json::json!({
        "schema_version":2, "context":{},
        "requests":[{"method":"POST","path":"/v1.56/containers/create?name=native-app","body":{
            "HostConfig":{"Binds":[literal],"Mounts":[{"Type":"volume","Source":"state","Target":"/state","ReadOnly":false}]}
        }}],
        "prerequisites":[authored_docker_bind_row("native-app", 1, "/fake/private-source-canary", "/data", read_only, relabel, "7")]
    });
    Ok((manifest, application, plan))
}

#[cfg(feature = "docker")]
fn assert_docker_context_rejected(
    profile: &boxferry::docker::docker_lens::version::TargetProfile,
    plan: &serde_json::Value,
) -> Result<(), Box<dyn Error>> {
    let error = validate_docker_plan_inputs(profile, plan)
        .err()
        .ok_or("changed native context was accepted")?;
    assert!(!error.to_string().contains("canary"));
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_schema2_bind_contract_preserves_independent_access_and_relabel_literals() -> Result<(), Box<dyn Error>> {
    for (read_only, relabel, literal) in [
        (true, "shared", "/fake/private-source-canary:/data:ro,z"),
        (false, "shared", "/fake/private-source-canary:/data:rw,z"),
        (true, "private", "/fake/private-source-canary:/data:ro,Z"),
        (false, "private", "/fake/private-source-canary:/data:rw,Z"),
    ] {
        let (manifest, application, plan) = synthetic_docker_bind_contract(read_only, relabel, literal)?;
        validate_authored_docker_bind_plan(&manifest, &application, &plan)?;
        let expected = expected_docker_bind_mounts(&manifest, &application)?;
        assert_eq!(expected[0].mount_index, 1, "named-volume sibling precedes the bind");
        for reference in ["0", "18446744073709551615", "42"] {
            let mut renamed = plan.clone();
            renamed["prerequisites"][0]["reference"] = serde_json::json!(reference);
            validate_authored_docker_bind_plan(&manifest, &application, &renamed)?;
        }
        for (field, replacement) in [
            ("source", serde_json::json!("/fake/other-private-canary")),
            ("target", serde_json::json!("/other-private-canary")),
            ("read_only", serde_json::json!(!read_only)),
            (
                "relabel",
                serde_json::json!(if relabel == "shared" { "private" } else { "shared" }),
            ),
        ] {
            let mut changed = plan.clone();
            changed["prerequisites"][0][field] = replacement;
            let error = validate_authored_docker_bind_plan(&manifest, &application, &changed)
                .err()
                .ok_or("contradictory Docker bind intent was accepted")?;
            assert!(!error.to_string().contains("canary"));
        }
        let mut changed = plan.clone();
        changed["requests"][0]["body"]["HostConfig"]["Binds"] =
            serde_json::json!(["/fake/private-source-canary:/data:ro"]);
        let error = validate_authored_docker_bind_plan(&manifest, &application, &changed)
            .err()
            .ok_or("stripped relabel literal was accepted")?;
        assert!(!error.to_string().contains("canary"));
    }
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_prerequisite_schema_and_opaque_reference_groups_are_closed() -> Result<(), Box<dyn Error>> {
    let (_, _, mut plan) = synthetic_docker_bind_contract(true, "private", "/fake/private-source-canary:/data:ro,Z")?;
    plan["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
        .extend([
            authored_docker_bind_row("native-app", 2, "/fake/extra", "/extra", false, "shared", "7"),
            serde_json::json!({"kind":"network","reference":"17","identity":"edge","expected_driver":"bridge"}),
            serde_json::json!({"kind":"volume","reference":"18","identity":"external-data"}),
        ]);
    assert_eq!(parse_docker_prerequisites(&plan)?.binds.len(), 2);
    let mut renamed = plan.clone();
    for row in renamed["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
    {
        if row["kind"] == "bind_source" {
            row["reference"] = serde_json::json!("18446744073709551615");
        }
    }
    parse_docker_prerequisites(&renamed)?;
    let mut overlap = plan.clone();
    for row in overlap["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
    {
        if row["kind"] == "bind_source" {
            row["reference"] = serde_json::json!("17");
        }
    }
    let error = parse_docker_prerequisites(&overlap)
        .err()
        .ok_or("bind references overlapped an external reference")?;
    assert!(error.to_string().contains("overlap"));
    let mut schema_one = plan.clone();
    schema_one["schema_version"] = serde_json::json!(1);
    schema_one["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
        .retain(|row| row["kind"] != "bind_source");
    let parsed = parse_docker_prerequisites(&schema_one)?;
    assert!(parsed.binds.is_empty());
    assert_eq!(
        parsed.identities,
        BTreeSet::from([("network", "edge"), ("volume", "external-data")])
    );
    for (case, replacement) in [
        ("split-group", serde_json::json!("8")),
        ("external-overlap", serde_json::json!("17")),
        ("numeric", serde_json::json!(7)),
        ("boolean", serde_json::json!(true)),
        ("leading-zero", serde_json::json!("07")),
        ("signed", serde_json::json!("-1")),
        ("overflow", serde_json::json!("18446744073709551616")),
    ] {
        let mut changed = plan.clone();
        changed["prerequisites"][1]["reference"] = replacement;
        let error = parse_docker_prerequisites(&changed)
            .err()
            .ok_or("invalid reference group was accepted")?;
        assert!(!error.to_string().contains("canary"), "{case}");
    }
    let mut collision = plan.clone();
    collision["prerequisites"][1]["identity"] = serde_json::json!("other-private-canary");
    let error = parse_docker_prerequisites(&collision)
        .err()
        .ok_or("different identities shared a reference")?;
    assert!(!error.to_string().contains("canary"));
    for (source, version) in [
        (&plan, serde_json::json!(1)),
        (&schema_one, serde_json::json!(2)),
        (&plan, serde_json::json!(true)),
        (&schema_one, serde_json::json!(3)),
        (&plan, serde_json::json!(2.0)),
        (&plan, serde_json::json!("2")),
    ] {
        let mut changed = source.clone();
        changed["schema_version"] = version;
        assert!(parse_docker_prerequisites(&changed).is_err());
    }
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_bind_association_obligations_and_literals_fail_without_value_disclosure() -> Result<(), Box<dyn Error>> {
    let (manifest, application, plan) =
        synthetic_docker_bind_contract(true, "private", "/fake/private-source-canary:/data:ro,Z")?;
    for (field, replacement) in [
        ("kind", serde_json::json!("unknown-private-canary")),
        ("identity", serde_json::json!("other-private-canary")),
        ("mount_index", serde_json::json!("0")),
        ("mount_index", serde_json::json!(1)),
        ("mount_index", serde_json::json!("01")),
        ("mount_index", serde_json::json!("18446744073709551616")),
        ("read_only", serde_json::json!(1)),
        ("selinux_effect", serde_json::json!("verified")),
        ("source_conditions_satisfied", serde_json::json!(true)),
        ("relabel_authority", serde_json::json!(true)),
        ("source_conditions", serde_json::json!(["exists"])),
        ("selinux_conditions", serde_json::json!(["daemon_selinux_enabled"])),
    ] {
        let mut changed = plan.clone();
        changed["prerequisites"][0][field] = replacement;
        let error = validate_authored_docker_bind_plan(&manifest, &application, &changed)
            .err()
            .ok_or("invalid Docker bind metadata was accepted")?;
        assert!(!error.to_string().contains("canary"));
    }
    for field in ["source_conditions", "selinux_conditions"] {
        let original = plan["prerequisites"][0][field]
            .as_array()
            .ok_or("authored conditions missing")?;
        for ordinal in 0..original.len() {
            let mut conditions = original.clone();
            conditions.remove(ordinal);
            let mut changed = plan.clone();
            changed["prerequisites"][0][field] = serde_json::json!(conditions);
            assert!(validate_authored_docker_bind_plan(&manifest, &application, &changed).is_err());
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_bind_bijection_and_native_literals_fail_closed() -> Result<(), Box<dyn Error>> {
    let (manifest, application, plan) =
        synthetic_docker_bind_contract(true, "private", "/fake/private-source-canary:/data:ro,Z")?;
    let mut missing = plan.clone();
    missing["prerequisites"] = serde_json::json!([]);
    missing["schema_version"] = serde_json::json!(1);
    assert!(validate_authored_docker_bind_plan(&manifest, &application, &missing).is_err());
    let mut duplicate = plan.clone();
    duplicate["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
        .push(plan["prerequisites"][0].clone());
    assert!(validate_authored_docker_bind_plan(&manifest, &application, &duplicate).is_err());
    let mut extra = plan.clone();
    extra["prerequisites"]
        .as_array_mut()
        .ok_or("authored prerequisites missing")?
        .push(authored_docker_bind_row(
            "native-app",
            2,
            "/fake/extra",
            "/extra",
            true,
            "private",
            "7",
        ));
    assert!(validate_authored_docker_bind_plan(&manifest, &application, &extra).is_err());
    for literals in [
        serde_json::json!([]),
        serde_json::json!(["/fake/private-source-canary:/data:ro,z"]),
        serde_json::json!(["/fake/private-source-canary:/data:rw,Z"]),
        serde_json::json!(["/fake/private-source-canary:/data:ro,Z,rshared"]),
        serde_json::json!([
            "/fake/private-source-canary:/data:ro,Z",
            "/fake/private-source-canary:/data:ro,Z"
        ]),
    ] {
        let mut changed = plan.clone();
        changed["requests"][0]["body"]["HostConfig"]["Binds"] = literals;
        let error = validate_authored_docker_bind_plan(&manifest, &application, &changed)
            .err()
            .ok_or("invalid Docker bind literals were accepted")?;
        assert!(!error.to_string().contains("canary"));
    }
    let mut duplicate = plan.clone();
    duplicate["requests"][0]["body"]["HostConfig"]["Mounts"]
        .as_array_mut()
        .ok_or("authored mounts missing")?
        .push(
            serde_json::json!({"Type":"bind","Source":"/fake/private-source-canary","Target":"/data","ReadOnly":true}),
        );
    assert!(validate_authored_docker_bind_plan(&manifest, &application, &duplicate).is_err());
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_request_index_rejects_private_malformed_requests_without_panics_or_disclosure() -> Result<(), Box<dyn Error>>
{
    let (manifest, application, plan) =
        synthetic_docker_bind_contract(true, "private", "/fake/private-source-canary:/data:ro,Z")?;
    for (field, replacement) in [
        (
            "path",
            serde_json::json!("/v1.56/private-operation-canary?token=private-path-canary"),
        ),
        ("method", serde_json::json!("PRIVATE-METHOD-CANARY")),
        ("method", serde_json::json!({"private-method-canary":true})),
        ("path", serde_json::json!("/v99.99/private-version-canary")),
        ("body", serde_json::json!("private-body-canary")),
    ] {
        let mut changed = plan.clone();
        changed["requests"][0][field] = replacement;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            validate_authored_docker_bind_plan(&manifest, &application, &changed)
        }))
        .map_err(|_| "fallible Docker request indexing panicked")?;
        let error = outcome.err().ok_or("malformed Docker request was accepted")?;
        let message = error.to_string();
        assert!(!message.to_ascii_lowercase().contains("canary"));
        assert!(!message.contains("/v1.56/") && !message.contains("/v99.99/") && !message.contains("token="));
    }
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_bind_order_corroboration_does_not_replace_manifest_semantics() -> Result<(), Box<dyn Error>> {
    for field in ["source", "target", "access", "relabel"] {
        let (mut manifest, application, _) =
            synthetic_docker_bind_contract(true, "private", "/fake/private-source-canary:/data:ro,Z")?;
        let required = &mut manifest.semantics.required_bind_mounts[0];
        match field {
            "source" => required.source = "/fake/changed-private-canary".into(),
            "target" => required.target = "/changed-private-canary".into(),
            "access" => required.read_only = false,
            "relabel" => required.selinux_relabel = "shared".into(),
            _ => return Err("unknown authored mismatch control".into()),
        }
        let error = expected_docker_bind_mounts(&manifest, &application)
            .err()
            .ok_or("imported values replaced independent bind expectations")?;
        assert!(!error.to_string().contains("canary"));
    }
    Ok(())
}

#[cfg(feature = "docker")]
#[test]
fn docker_schema1_context_matches_actual_native_upstream_and_debian_serialization() -> Result<(), Box<dyn Error>> {
    use boxferry::docker::docker_lens::version::{DaemonMode, EngineBuild, TargetCapabilityCatalog};
    let catalogue = TargetCapabilityCatalog::reviewed();
    for profile in catalogue.profiles() {
        let debian = matches!(profile.identity().build(), EngineBuild::DebianPackage(_));
        let opposite_mode = match profile.identity().mode() {
            DaemonMode::Rootful => "rootless",
            DaemonMode::Rootless => "rootful",
            DaemonMode::Unknown => return Err("native context control requires a reviewed root mode".into()),
        };
        let version: boxferry::PlatformVersion = profile
            .release()
            .as_str()
            .split('+')
            .next()
            .ok_or("native numeric release missing")?
            .parse()?;
        let target = boxferry::TargetProfile::new("docker", version, Some(version))?;
        let mut application = boxferry::Application::new(Identifier::new("native-context-control")?);
        let mut service = boxferry::Service::new(Identifier::new("probe")?);
        service.set_image(boxferry::Sourced::generated(boxferry::ImageReference::parse(
            "fixture:1",
        )?));
        application.add_service(boxferry::Sourced::generated(service))?;
        // Only the existing plain-container native slice is planned; no relabel capability is invented.
        let planned = DockerExporter::new(profile.clone())?.plan(&application, &target)?;
        let raw = planned
            .candidate()
            .ok_or("plain native context candidate missing")?
            .complete_bytes()
            .map_err(|error| format!("{error:?}"))?;
        let plan: serde_json::Value = serde_json::from_slice(&raw)?;
        let inputs = validate_docker_plan_inputs(profile, &plan)?;
        assert!(inputs.bind_prerequisites.is_empty());
        assert_eq!(plan["schema_version"], 1);
        assert_eq!(
            plan["context"]["build"]["kind"],
            if debian { "debian_package" } else { "upstream" }
        );
        for (field, replacement) in [
            ("kind", serde_json::json!("observed")),
            ("engine_release", serde_json::json!("0.0.0")),
            ("advertised_api_version", serde_json::json!("1.99")),
            ("acquisition_api_version", serde_json::json!(true)),
            ("rendering_api_version", serde_json::json!("1.00")),
            ("daemon_mode", serde_json::json!(opposite_mode)),
            ("evidence_sha256", serde_json::json!("0".repeat(64))),
            ("extra", serde_json::json!("private-context-canary")),
        ] {
            assert_ne!(
                plan["context"][field], replacement,
                "context mutation must change the selected profile"
            );
            let mut changed = plan.clone();
            changed["context"][field] = replacement;
            assert_docker_context_rejected(profile, &changed)?;
        }
        let mut extra = plan.clone();
        extra["context"]["build"]["extra"] = serde_json::json!("private-build-canary");
        assert_docker_context_rejected(profile, &extra)?;
        if debian {
            assert_ne!(plan["context"]["build"]["revision"], "wrong-private-canary");
            let mut changed = plan.clone();
            changed["context"]["build"]["revision"] = serde_json::json!("wrong-private-canary");
            assert_docker_context_rejected(profile, &changed)?;
            changed = plan.clone();
            changed["context"]["build"]
                .as_object_mut()
                .ok_or("native build object missing")?
                .remove("revision");
            assert_docker_context_rejected(profile, &changed)?;
            changed = plan.clone();
            changed["context"]["build"]["kind"] = serde_json::json!("debian-package");
            assert_docker_context_rejected(profile, &changed)?;
        } else {
            let mut changed = plan.clone();
            changed["context"]["build"]["revision"] = serde_json::json!("unreviewed-private-canary");
            assert_docker_context_rejected(profile, &changed)?;
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn docker_canonical_decimal(value: &serde_json::Value) -> Result<u64, Box<dyn Error>> {
    let text = value.as_str().ok_or("Docker prerequisite decimal must be a string")?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) || text.len() > 1 && text.starts_with('0') {
        return Err("Docker prerequisite decimal is not canonical".into());
    }
    text.parse()
        .map_err(|_| "Docker prerequisite decimal exceeds u64".into())
}

#[cfg(feature = "docker")]
fn parse_docker_bind_prerequisite(
    value: &serde_json::Value,
) -> Result<(u64, DockerBindPrerequisite<'_>), Box<dyn Error>> {
    if !docker_closed_fields(
        value,
        &[
            "kind",
            "reference",
            "identity",
            "mount_index",
            "source",
            "target",
            "read_only",
            "relabel",
            "source_conditions",
            "selinux_effect",
            "selinux_conditions",
        ],
    ) || value["kind"] != "bind_source"
    {
        return Err("Docker bind prerequisite has unreviewed fields".into());
    }
    let identity = value["identity"]
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or("Docker bind prerequisite identity missing")?;
    let source = value["source"]
        .as_str()
        .ok_or("Docker bind prerequisite source missing")?;
    let target = value["target"]
        .as_str()
        .ok_or("Docker bind prerequisite target missing")?;
    if [source, target]
        .iter()
        .any(|path| !path.starts_with('/') || path.contains(':') || path.contains('\0'))
    {
        return Err("Docker relabel bind requires closed absolute paths".into());
    }
    let relabel = value["relabel"]
        .as_str()
        .filter(|value| matches!(*value, "shared" | "private"))
        .ok_or("Docker bind prerequisite relabel is unreviewed")?;
    if value["source_conditions"]
        != serde_json::json!([
            "exists",
            "type_reviewed",
            "contents_reviewed",
            "ownership_reviewed",
            "permissions_reviewed",
        ])
        || value["selinux_conditions"]
            != serde_json::json!([
                "daemon_selinux_enabled",
                "container_mount_label_present",
                "policy_filesystem_support",
                "relabel_authority",
            ])
        || value["selinux_effect"] != "unverified"
    {
        return Err("Docker bind prerequisite obligations or unverified effect differ".into());
    }
    Ok((
        docker_canonical_decimal(&value["reference"])?,
        DockerBindPrerequisite {
            identity,
            mount_index: usize::try_from(docker_canonical_decimal(&value["mount_index"])?)
                .map_err(|_| "Docker bind prerequisite mount index exceeds usize")?,
            source,
            target,
            read_only: value["read_only"]
                .as_bool()
                .ok_or("Docker bind prerequisite access missing")?,
            relabel,
        },
    ))
}

#[cfg(feature = "docker")]
fn parse_docker_prerequisites(plan: &serde_json::Value) -> Result<DockerPrerequisites<'_>, Box<dyn Error>> {
    if !docker_closed_fields(plan, &["schema_version", "context", "requests", "prerequisites"]) {
        return Err("Docker complete artifact fields differ".into());
    }
    let rows = plan["prerequisites"]
        .as_array()
        .ok_or("Docker prerequisites must be explicit")?;
    let mut identities = BTreeSet::new();
    let mut external_references = BTreeSet::new();
    let mut binds = BTreeMap::new();
    let mut bind_references = BTreeMap::new();
    let mut reference_owners = BTreeMap::new();
    let mut network_internal = BTreeMap::new();
    for row in rows {
        let kind = row["kind"].as_str().ok_or("Docker prerequisite kind missing")?;
        if kind == "bind_source" {
            let (reference, bind) = parse_docker_bind_prerequisite(row)?;
            if bind_references
                .get(bind.identity)
                .is_some_and(|previous| *previous != reference)
                || reference_owners
                    .get(&reference)
                    .is_some_and(|previous| *previous != bind.identity)
            {
                return Err("Docker bind prerequisite reference groups disagree".into());
            }
            bind_references.insert(bind.identity, reference);
            reference_owners.insert(reference, bind.identity);
            if binds.insert((bind.identity, bind.mount_index), bind).is_some() {
                return Err("Docker bind prerequisite identity and mount index repeat".into());
            }
            continue;
        }
        let fields: &[&str] = match kind {
            "network" if row.get("expected_internal").is_some() => {
                &["kind", "reference", "identity", "expected_driver", "expected_internal"]
            }
            "network" => &["kind", "reference", "identity", "expected_driver"],
            "volume" => &["kind", "reference", "identity"],
            _ => return Err("Docker prerequisite kind is unreviewed".into()),
        };
        if !docker_closed_fields(row, fields) || kind == "network" && row["expected_driver"] != "bridge" {
            return Err("Docker external prerequisite fields or driver differ".into());
        }
        let identity = row["identity"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("Docker prerequisite identity missing")?;
        if kind == "network"
            && let Some(internal) = row.get("expected_internal")
        {
            network_internal.insert(
                identity,
                internal
                    .as_bool()
                    .ok_or("Docker external isolation expectation must be a boolean")?,
            );
        }
        if !external_references.insert(docker_canonical_decimal(&row["reference"])?)
            || !identities.insert((kind, identity))
        {
            return Err("Docker external prerequisite reference or identity repeats".into());
        }
    }
    if external_references
        .iter()
        .any(|reference| reference_owners.contains_key(reference))
    {
        return Err("Docker bind and external prerequisite references overlap".into());
    }
    if plan["schema_version"].as_u64()
        != Some(if !network_internal.is_empty() {
            3
        } else if binds.is_empty() {
            1
        } else {
            2
        })
    {
        return Err("Docker complete artifact schema differs from its prerequisite kinds".into());
    }
    Ok(DockerPrerequisites {
        identities,
        binds,
        network_internal,
    })
}

#[cfg(feature = "docker")]
struct DockerRequestIndex<'a> {
    creates: BTreeMap<String, &'a serde_json::Value>,
    created_networks: BTreeSet<&'a str>,
    created_volumes: BTreeSet<&'a str>,
    network_connections: BTreeSet<(String, String)>,
}

#[cfg(feature = "docker")]
fn expected_docker_plan_context(
    profile: &boxferry::docker::docker_lens::version::TargetProfile,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let identity = profile.identity();
    let build = match identity.build() {
        boxferry::docker::docker_lens::version::EngineBuild::Upstream => serde_json::json!({"kind":"upstream"}),
        boxferry::docker::docker_lens::version::EngineBuild::DebianPackage(revision) => {
            serde_json::json!({"kind":"debian_package","revision":revision.as_str()})
        }
    };
    let mode = match identity.mode() {
        boxferry::docker::docker_lens::version::DaemonMode::Rootful => "rootful",
        boxferry::docker::docker_lens::version::DaemonMode::Rootless => "rootless",
        boxferry::docker::docker_lens::version::DaemonMode::Unknown => {
            return Err("unreviewed Docker target mode".into());
        }
    };
    let api =
        |version: boxferry::docker::docker_lens::version::ApiVersion| format!("{}.{}", version.major, version.minor);
    Ok(serde_json::json!({
        "kind":"target", "build":build, "engine_release":identity.release().as_str(),
        "advertised_api_version":api(identity.advertised_api_version()),
        "acquisition_api_version":api(identity.acquisition_api_version()),
        "rendering_api_version":api(identity.rendering_api_version()), "daemon_mode":mode,
        "evidence_sha256":lowercase_hex(profile.evidence_key().as_sha256_bytes())
    }))
}

#[cfg(feature = "docker")]
fn validate_docker_plan_inputs<'a>(
    profile: &boxferry::docker::docker_lens::version::TargetProfile,
    plan: &'a serde_json::Value,
) -> Result<DockerPlanInputs<'a>, Box<dyn Error>> {
    let identity = profile.identity();
    let prerequisites = parse_docker_prerequisites(plan)?;
    if plan["context"] != expected_docker_plan_context(profile)? {
        return Err("Docker complete artifact context differs from the independently selected target".into());
    }
    let requests = plan["requests"].as_array().ok_or("Docker native requests missing")?;
    assert!(
        !requests.is_empty(),
        "a successful Docker route needs native create intent"
    );
    let api_prefix = format!(
        "/v{}.{}/",
        identity.rendering_api_version().major,
        identity.rendering_api_version().minor
    );
    Ok(DockerPlanInputs {
        requests,
        api_prefix,
        prerequisite_identities: prerequisites.identities,
        bind_prerequisites: prerequisites.binds,
        network_internal: prerequisites.network_internal,
    })
}

#[cfg(feature = "docker")]
fn index_docker_requests<'a>(
    requests: &'a [serde_json::Value],
    api_prefix: &str,
) -> Result<DockerRequestIndex<'a>, Box<dyn Error>> {
    let mut creates = BTreeMap::new();
    let mut created_networks = BTreeSet::new();
    let mut created_volumes = BTreeSet::new();
    let mut network_connections = BTreeSet::new();
    for request in requests {
        if request["method"] != "POST" {
            return Err("Docker native request method is unreviewed".into());
        }
        let path = request["path"].as_str().ok_or("Docker native request path")?;
        if !path.starts_with(api_prefix) {
            return Err("Docker request uses the wrong reviewed rendering API".into());
        }
        if !request["body"].is_object() {
            return Err("Docker native request body must be an object".into());
        }
        if !request["body"]["DependsOn"].is_null() {
            return Err("Engine create has no dependency choreography".into());
        }
        if let Some(name) = path.strip_prefix(&format!("{api_prefix}containers/create?name=")) {
            if name.is_empty() || creates.insert(name.to_owned(), &request["body"]).is_some() {
                return Err("Docker container create identity is empty or repeated".into());
            }
        } else if path == format!("{api_prefix}networks/create") {
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("Docker network create identity missing")?;
            if name.is_empty() || !created_networks.insert(name) {
                return Err("Docker network create identity is empty or repeated".into());
            }
        } else if path == format!("{api_prefix}volumes/create") {
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("Docker volume create identity missing")?;
            if name.is_empty() || !created_volumes.insert(name) {
                return Err("Docker volume create identity is empty or repeated".into());
            }
        } else if let Some(network) = path
            .strip_prefix(&format!("{api_prefix}networks/"))
            .and_then(|suffix| suffix.strip_suffix("/connect"))
        {
            let container = request["body"]["Container"]
                .as_str()
                .ok_or("Docker network connect lacks container identity")?;
            if network.is_empty() || container.is_empty() {
                return Err("Docker network connect lacks a target identity".into());
            }
            if !network_connections.insert((container.to_owned(), network.to_owned())) {
                return Err("Docker network connect is repeated".into());
            }
        } else {
            return Err("Docker plan contains an unreviewed operation path".into());
        }
    }
    Ok(DockerRequestIndex {
        creates,
        created_networks,
        created_volumes,
        network_connections,
    })
}

/// Docker output is an offline target request, never an observed source snapshot.
#[cfg(feature = "docker")]
fn docker_semantics(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    output: &Path,
    source_application: &boxferry::Application,
) -> Result<Vec<String>, Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let profile = route.docker_profile()?;
    let plan_path = output.join("docker-plan.json");
    let bytes = fs::read(&plan_path)?;
    let plan: serde_json::Value = serde_json::from_slice(&bytes)?;
    let inputs = validate_docker_plan_inputs(&profile, &plan)?;
    let index = index_docker_requests(inputs.requests, &inputs.api_prefix)?;
    validate_docker_volume_labels(route, &inputs, &index)?;
    validate_docker_networks(manifest, Some(route), &inputs, &index)?;
    let logical_creates = index_docker_components(manifest, &index)?;
    let mut gaps = Vec::new();
    validate_docker_network_memberships(manifest, &inputs, &index, &logical_creates)?;
    validate_docker_environment_and_ports(manifest, route, &logical_creates, &mut gaps)?;
    validate_docker_mounts(manifest, &inputs, &index, &logical_creates)?;
    let bind_expectations = expected_docker_bind_mounts(manifest, source_application)?;
    validate_docker_bind_mounts(&bind_expectations, &inputs, &index)?;
    if route.docker_exact_native_inventory {
        validate_exact_docker_inventory_for_route(manifest, Some(route), &plan, &bind_expectations)?;
    }
    validate_docker_process_semantics(manifest, &logical_creates)?;
    validate_docker_restart_policies(manifest, &logical_creates)?;
    assert_eq!(fs::metadata(output)?.permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&plan_path)?.permissions().mode() & 0o777, 0o600);
    if !route.includes_environment_values() {
        assert_no_protected_environment_values(manifest, &String::from_utf8_lossy(&bytes), "Docker native plan")?;
    }
    validate_docker_dependency_sidecar(manifest, route, output, &bytes, &index)?;
    gaps.extend(
        route
            .docker_dependencies
            .iter()
            .map(|edge| format!("dependency:{}:{}", edge.service, edge.dependency)),
    );
    Ok(gaps)
}

#[cfg(feature = "docker")]
fn validate_docker_volume_labels(
    route: &RouteExpectation,
    inputs: &DockerPlanInputs<'_>,
    _index: &DockerRequestIndex<'_>,
) -> Result<(), Box<dyn Error>> {
    for volume in &route.docker_empty_volume_labels {
        let create = inputs
            .requests
            .iter()
            .find(|request| {
                request["path"] == format!("{}volumes/create", inputs.api_prefix)
                    && request["body"]["Name"].as_str() == Some(volume.as_str())
            })
            .ok_or("Docker volume-label assertion lacks a create request")?;
        let labels = &create["body"]["Labels"];
        assert!(
            labels.is_null() || labels.as_object().is_some_and(serde_json::Map::is_empty),
            "Docker volume create invented labels absent from the authored scenario"
        );
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_networks(
    manifest: &ScenarioManifest,
    route: Option<&RouteExpectation>,
    inputs: &DockerPlanInputs<'_>,
    index: &DockerRequestIndex<'_>,
) -> Result<(), Box<dyn Error>> {
    validate_docker_external_internal_for_route(manifest, route, &inputs.network_internal)?;
    let external_internal = expected_docker_external_internal(manifest, route)?;
    for expected in &manifest.semantics.required_networks {
        match expected.ownership.as_str() {
            "application" => {
                assert!(
                    index.created_networks.contains(expected.name.as_str()),
                    "Docker plan lost an owned network create"
                );
                assert!(
                    !inputs
                        .prerequisite_identities
                        .contains(&("network", expected.name.as_str())),
                    "owned Docker network became an external prerequisite"
                );
                if let Some(internal) = expected.internal {
                    let create = inputs
                        .requests
                        .iter()
                        .find(|request| {
                            request["path"] == format!("{}networks/create", inputs.api_prefix)
                                && request["body"]["Name"].as_str() == Some(expected.name.as_str())
                        })
                        .ok_or("required Docker network create is missing")?;
                    validate_created_docker_bridge_internal(&create["body"], internal)?;
                }
            }
            "external" => {
                if inputs.network_internal.get(expected.name.as_str()).copied()
                    != external_internal.get(expected.name.as_str()).copied()
                {
                    return Err("Docker external prerequisite changed authored isolation expectation".into());
                }
                assert!(
                    inputs
                        .prerequisite_identities
                        .contains(&("network", expected.name.as_str())),
                    "Docker plan lost an external network prerequisite"
                );
                assert!(
                    !index.created_networks.contains(expected.name.as_str()),
                    "external Docker network was created"
                );
            }
            _ => return Err("successful Docker route has unowned network intent".into()),
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_external_internal_expectations(
    manifest: &ScenarioManifest,
    actual: &BTreeMap<&str, bool>,
) -> Result<(), Box<dyn Error>> {
    validate_docker_external_internal_for_route(manifest, None, actual)
}

#[cfg(feature = "docker")]
fn expected_docker_external_internal<'a>(
    manifest: &'a ScenarioManifest,
    route: Option<&'a RouteExpectation>,
) -> Result<BTreeMap<&'a str, bool>, Box<dyn Error>> {
    let mut expected = manifest
        .semantics
        .required_networks
        .iter()
        .filter(|network| network.ownership == "external")
        .filter_map(|network| network.internal.map(|internal| (network.name.as_str(), internal)))
        .collect::<BTreeMap<_, _>>();
    if let Some(route) = route {
        route.validate_docker_external_network_internal(manifest)?;
        for (name, internal) in &route.docker_external_network_internal {
            expected.insert(name.as_str(), *internal);
        }
    }
    Ok(expected)
}

#[cfg(feature = "docker")]
fn validate_docker_external_internal_for_route(
    manifest: &ScenarioManifest,
    route: Option<&RouteExpectation>,
    actual: &BTreeMap<&str, bool>,
) -> Result<(), Box<dyn Error>> {
    let expected = expected_docker_external_internal(manifest, route)?;
    if *actual != expected {
        return Err("Docker external prerequisite changed authored isolation expectations".into());
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_created_docker_bridge_internal(body: &serde_json::Value, expected: bool) -> Result<(), Box<dyn Error>> {
    match body.get("Internal") {
        Some(serde_json::Value::Bool(actual)) if *actual == expected => Ok(()),
        None if !expected && body.get("Driver").and_then(serde_json::Value::as_str) == Some("bridge") => Ok(()),
        _ => Err("created Docker bridge changed authored isolation".into()),
    }
}

#[cfg(feature = "docker")]
fn index_docker_components<'m, 'r>(
    manifest: &'m ScenarioManifest,
    index: &DockerRequestIndex<'r>,
) -> Result<BTreeMap<&'m str, &'r serde_json::Value>, Box<dyn Error>> {
    let mut logical_creates = BTreeMap::new();
    for component in &manifest.components {
        let name = manifest
            .semantics
            .required_runtime_names
            .iter()
            .find_map(|value| {
                value
                    .split_once(':')
                    .filter(|(service, _)| *service == component.name)
                    .map(|(_, runtime)| runtime)
            })
            .unwrap_or(&component.name);
        let body = index
            .creates
            .get(name)
            .ok_or_else(|| format!("missing Docker create for {name}"))?;
        logical_creates.insert(component.name.as_str(), *body);
        let digest_reference = component
            .image
            .as_ref()
            .zip(component.digest.as_ref())
            .map(|(image, digest)| format!("{image}@{digest}"));
        let expected_image = component.configured_image.as_deref().or(digest_reference.as_deref());
        if expected_image.is_some_and(|expected_image| body["Image"] != expected_image) {
            return Err("Docker create image differs from authored component".into());
        }
    }
    if index.creates.len() != logical_creates.len() {
        return Err("Docker plan contains an undeclared container create".into());
    }
    Ok(logical_creates)
}

#[cfg(feature = "docker")]
fn validate_docker_network_memberships(
    manifest: &ScenarioManifest,
    inputs: &DockerPlanInputs<'_>,
    index: &DockerRequestIndex<'_>,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for requirement in &manifest.semantics.exact_network_memberships {
        let (service, names) = requirement
            .split_once(':')
            .ok_or("invalid network membership expectation")?;
        let body = logical_creates
            .get(service)
            .ok_or("network membership service lacks Docker create")?;
        let runtime_name = manifest
            .semantics
            .required_runtime_names
            .iter()
            .find_map(|value| {
                value
                    .split_once(':')
                    .filter(|(logical, _)| *logical == service)
                    .map(|(_, native)| native)
            })
            .unwrap_or(service);
        let endpoints = body["NetworkingConfig"]["EndpointsConfig"]
            .as_object()
            .ok_or("Docker create lacks required network attachments")?;
        let expected = names.split(',').collect::<BTreeSet<_>>();
        let actual = endpoints
            .keys()
            .map(String::as_str)
            .chain(
                index
                    .network_connections
                    .iter()
                    .filter_map(|(container, network)| (container == runtime_name).then_some(network.as_str())),
            )
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected, "Docker plan changed authored network memberships");
    }
    for (container, network) in &index.network_connections {
        assert!(
            index.creates.contains_key(container),
            "Docker network connect references an undeclared container"
        );
        assert!(
            index.created_networks.contains(network.as_str())
                || inputs.prerequisite_identities.contains(&("network", network.as_str())),
            "Docker network connect references an undeclared network"
        );
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_environment_and_ports(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
    gaps: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    for requirement in &manifest.semantics.required_environment {
        let (service, assignment) = requirement.split_once(':').ok_or("invalid environment expectation")?;
        let (name, _) = assignment.split_once('=').ok_or("invalid environment assignment")?;
        let body = logical_creates
            .get(service)
            .ok_or("environment service lacks Docker create")?;
        let retained = body["Env"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value == assignment));
        if route.includes_environment_values() {
            assert!(retained, "required Docker environment assignment was lost");
        } else {
            assert!(!retained, "withheld environment assignment entered Docker plan");
            gaps.push(format!("environment:{service}:{name}"));
        }
    }
    for requirement in &manifest.semantics.required_ports {
        let (service, rest) = requirement.split_once(':').ok_or("invalid published port")?;
        let (host, container_protocol) = rest.split_once(':').ok_or("invalid published port")?;
        let body = logical_creates
            .get(service)
            .ok_or("publication service lacks Docker create")?;
        assert!(
            body["ExposedPorts"][container_protocol].is_object(),
            "Docker plan lost exposed container port"
        );
        assert!(
            body["HostConfig"]["PortBindings"][container_protocol]
                .as_array()
                .is_some_and(|bindings| { bindings.iter().any(|binding| binding["HostPort"] == host) }),
            "Docker plan lost host publication"
        );
    }
    validate_docker_port_bindings(manifest, logical_creates)?;
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_port_bindings(
    manifest: &ScenarioManifest,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for required in &manifest.semantics.required_port_bindings {
        let body = logical_creates
            .get(required.service.as_str())
            .ok_or("port-binding service lacks Docker create")?;
        let key = format!("{}/{}", required.container, required.protocol);
        let expected_port = required.published.map_or_else(String::new, |port| port.to_string());
        assert!(
            body["HostConfig"]["PortBindings"][&key]
                .as_array()
                .is_some_and(|bindings| {
                    bindings.iter().any(|binding| {
                        binding["HostPort"].as_str() == Some(expected_port.as_str())
                            && required
                                .host_address
                                .as_ref()
                                .is_none_or(|address| binding["HostIp"].as_str() == Some(address.as_str()))
                    })
                }),
            "Docker plan changed an authored host-IP/port binding"
        );
    }
    for service in &manifest.semantics.unpublished_services {
        let body = logical_creates
            .get(service.as_str())
            .ok_or("unpublished service lacks Docker create")?;
        let bindings = &body["HostConfig"]["PortBindings"];
        assert!(
            bindings.is_null() || bindings.as_object().is_some_and(serde_json::Map::is_empty),
            "Docker plan published an authored private service"
        );
    }
    Ok(())
}

#[cfg(feature = "docker")]
struct DockerBindExpectation {
    identity: String,
    mount_index: usize,
    source: String,
    target: String,
    read_only: bool,
    relabel: &'static str,
}

/// The manifest owns semantics; the imported neutral application contributes only
/// corroboration and original target-intent mount order. No candidate or host path is read.
#[cfg(feature = "docker")]
fn expected_docker_bind_mounts(
    manifest: &ScenarioManifest,
    application: &boxferry::Application,
) -> Result<Vec<DockerBindExpectation>, Box<dyn Error>> {
    let mut expected = Vec::new();
    for required in &manifest.semantics.required_bind_mounts {
        let service = application
            .services()
            .iter()
            .find(|service| service.value().name().as_str() == required.service)
            .ok_or("Docker bind expectation lacks a source service")?
            .value();
        let mut positions = service
            .mounts()
            .iter()
            .enumerate()
            .filter(|(_, mount)| mount.value().target() == required.target);
        let (mount_index, neutral) = positions
            .next()
            .ok_or("Docker bind expectation lacks original mount order")?;
        if positions.next().is_some() {
            return Err("Docker bind source target is ambiguous".into());
        }
        let (relabel, neutral_relabel) = match required.selinux_relabel.as_str() {
            "shared" => ("shared", boxferry::SelinuxRelabel::Shared),
            "private" => ("private", boxferry::SelinuxRelabel::Private),
            _ => return Err("Docker bind expectation relabel is unreviewed".into()),
        };
        if !matches!(neutral.value().source(), boxferry::MountSource::HostPath(source) if source == &required.source)
            || neutral.value().read_only() != required.read_only
            || neutral.value().selinux_relabel() != Some(neutral_relabel)
            || [&required.source, &required.target]
                .iter()
                .any(|path| !path.starts_with('/') || path.contains(':') || path.contains('\0'))
        {
            return Err("Docker neutral bind disagrees with independently authored semantics".into());
        }
        let identity = manifest
            .semantics
            .required_runtime_names
            .iter()
            .find_map(|name| {
                name.split_once(':')
                    .filter(|(service, _)| *service == required.service)
                    .map(|(_, native)| native)
            })
            .unwrap_or(&required.service);
        expected.push(DockerBindExpectation {
            identity: identity.to_owned(),
            mount_index,
            source: required.source.clone(),
            target: required.target.clone(),
            read_only: required.read_only,
            relabel,
        });
    }
    Ok(expected)
}

/// Configured literals and retained obligations do not establish `SELinux` effects.
#[cfg(feature = "docker")]
fn validate_docker_bind_mounts(
    expected: &[DockerBindExpectation],
    inputs: &DockerPlanInputs<'_>,
    index: &DockerRequestIndex<'_>,
) -> Result<(), Box<dyn Error>> {
    let mut expected_keys = BTreeSet::new();
    let mut groups: BTreeMap<&str, Vec<&DockerBindExpectation>> = BTreeMap::new();
    for required in expected {
        let key = (required.identity.as_str(), required.mount_index);
        if !expected_keys.insert(key) {
            return Err("Docker bind expectations repeat an identity and mount index".into());
        }
        let observed = inputs
            .bind_prerequisites
            .get(&key)
            .ok_or("Docker required bind prerequisite missing")?;
        if observed.source != required.source
            || observed.target != required.target
            || observed.read_only != required.read_only
            || observed.relabel != required.relabel
        {
            return Err("Docker bind prerequisite differs from independently authored intent".into());
        }
        groups.entry(required.identity.as_str()).or_default().push(required);
    }
    if expected_keys != inputs.bind_prerequisites.keys().copied().collect() {
        return Err("Docker bind prerequisite inventory differs".into());
    }
    for (identity, requirements) in &mut groups {
        requirements.sort_by_key(|required| required.mount_index);
        let body = index
            .creates
            .get(*identity)
            .ok_or("Docker required bind lacks a container create")?;
        let literals = requirements
            .iter()
            .map(|required| {
                let access = if required.read_only { "ro" } else { "rw" };
                let relabel = if required.relabel == "shared" { "z" } else { "Z" };
                format!("{}:{}:{access},{relabel}", required.source, required.target)
            })
            .collect::<Vec<_>>();
        // Literal equality is independent of Docker's native mode parser.
        if body["HostConfig"]["Binds"] != serde_json::json!(literals) {
            return Err("Docker bind literals differ from independently authored intent".into());
        }
        if body["HostConfig"]["Mounts"].as_array().is_some_and(|mounts| {
            mounts.iter().any(|mount| {
                requirements
                    .iter()
                    .any(|required| mount["Target"].as_str() == Some(required.target.as_str()))
            })
        }) {
            return Err("Docker relabelled bind is duplicated by a structured mount".into());
        }
    }
    for (identity, body) in &index.creates {
        if body["HostConfig"].get("Binds").is_some() && !groups.contains_key(identity.as_str()) {
            return Err("Docker plan contains undeclared bind literals".into());
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_mounts(
    manifest: &ScenarioManifest,
    inputs: &DockerPlanInputs<'_>,
    index: &DockerRequestIndex<'_>,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for requirement in &manifest.semantics.required_mounts {
        let (service, rest) = requirement.split_once(':').ok_or("invalid named mount")?;
        let (volume, target) = rest.split_once(':').ok_or("invalid named mount")?;
        let body = logical_creates
            .get(service)
            .ok_or("mount service lacks Docker create")?;
        assert!(
            body["HostConfig"]["Mounts"].as_array().is_some_and(|mounts| mounts
                .iter()
                .any(|mount| { mount["Type"] == "volume" && mount["Source"] == volume && mount["Target"] == target })),
            "Docker plan lost named volume mount"
        );
        assert!(
            index.created_volumes.contains(volume) || inputs.prerequisite_identities.contains(&("volume", volume)),
            "Docker named mount lacks a create request or external prerequisite"
        );
    }
    for required in &manifest.semantics.required_volume_mounts {
        let body = logical_creates
            .get(required.service.as_str())
            .ok_or("volume mount service lacks Docker create")?;
        assert!(
            body["HostConfig"]["Mounts"]
                .as_array()
                .is_some_and(|mounts| mounts.iter().any(|mount| {
                    mount["Type"] == "volume"
                        && mount["Source"] == required.source
                        && mount["Target"] == required.target
                        && mount["ReadOnly"] == required.read_only
                })),
            "Docker plan changed a reviewed volume mount"
        );
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_process_semantics(
    manifest: &ScenarioManifest,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for required in &manifest.semantics.required_commands {
        let body = logical_creates
            .get(required.service.as_str())
            .ok_or("command service lacks Docker create")?;
        if required.kind == "exec" {
            assert_eq!(
                body["Cmd"],
                serde_json::json!(required.values),
                "Docker plan changed an authored exec command"
            );
        } else if required.kind == "absent" {
            assert!(body["Cmd"].is_null(), "Docker plan invented a command override");
        }
    }
    for required in &manifest.semantics.required_entrypoints {
        let body = logical_creates
            .get(required.service.as_str())
            .ok_or("entrypoint service lacks Docker create")?;
        if required.kind == "exec" {
            assert_eq!(
                body["Entrypoint"],
                serde_json::json!(required.values),
                "Docker plan changed an authored exec entrypoint"
            );
        } else if required.kind == "absent" {
            assert!(
                body["Entrypoint"].is_null(),
                "Docker plan invented an entrypoint override"
            );
        }
    }
    validate_docker_healthchecks(manifest, logical_creates)
}

#[cfg(feature = "docker")]
fn validate_docker_healthchecks(
    manifest: &ScenarioManifest,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for required in &manifest.semantics.required_healthchecks {
        let body = logical_creates
            .get(required.service.as_str())
            .ok_or("healthcheck service lacks Docker create")?;
        let prefix = match required.command_kind.as_str() {
            "exec" => "CMD",
            "shell" => "CMD-SHELL",
            _ => return Err("unreviewed Docker healthcheck expectation".into()),
        };
        let expected = std::iter::once(prefix.to_owned())
            .chain(required.command.iter().cloned())
            .collect::<Vec<_>>();
        assert_eq!(
            body["Healthcheck"]["Test"],
            serde_json::json!(expected),
            "Docker plan changed an authored healthcheck"
        );
    }
    for component in &manifest.components {
        if !manifest
            .semantics
            .required_healthchecks
            .iter()
            .any(|health| health.service == component.name)
        {
            let body = logical_creates
                .get(component.name.as_str())
                .ok_or("healthcheck service lacks Docker create")?;
            assert!(
                body["Healthcheck"].is_null(),
                "Docker plan invented a healthcheck override"
            );
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_restart_policies(
    manifest: &ScenarioManifest,
    logical_creates: &BTreeMap<&str, &serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    for requirement in &manifest.semantics.required_restart {
        let (service, policy) = requirement.split_once(':').ok_or("invalid restart expectation")?;
        let body = logical_creates
            .get(service)
            .ok_or("restart service lacks Docker create")?;
        let actual = &body["HostConfig"]["RestartPolicy"];
        match policy {
            "absent" => assert!(actual.is_null(), "Docker plan invented a restart policy"),
            "never" => assert_eq!(actual["Name"], "no"),
            "always" | "unless-stopped" | "on-failure" => assert_eq!(actual["Name"], policy),
            _ => return Err("unreviewed Docker restart expectation".into()),
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_dependency_sidecar(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    output: &Path,
    bytes: &[u8],
    index: &DockerRequestIndex<'_>,
) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;
    let dependency_path = output.join("docker-dependencies.json");
    if dependency_path.exists() == route.docker_dependencies.is_empty() {
        return Err("Docker dependency sidecar presence changed".into());
    }
    if dependency_path.exists() {
        let sidecar_bytes = fs::read(&dependency_path)?;
        let sidecar: serde_json::Value = serde_json::from_slice(&sidecar_bytes)?;
        validate_docker_dependency_document(manifest, route, &sidecar, bytes, index)?;
        assert_eq!(fs::metadata(&dependency_path)?.permissions().mode() & 0o777, 0o600);
        assert_no_protected_environment_values(
            manifest,
            &String::from_utf8_lossy(&sidecar_bytes),
            "Docker dependency sidecar",
        )?;
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn authored_docker_runtime_name(manifest: &ScenarioManifest, service: &str) -> Result<String, Box<dyn Error>> {
    if !manifest.components.iter().any(|component| component.name == service) {
        return Err("Docker dependency names an unauthored service".into());
    }
    Ok(manifest
        .semantics
        .required_runtime_names
        .iter()
        .find_map(|name| {
            name.split_once(':')
                .filter(|(logical, _)| *logical == service)
                .map(|(_, native)| native)
        })
        .unwrap_or(service)
        .to_owned())
}

#[cfg(feature = "docker")]
fn validate_docker_dependency_document(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    sidecar: &serde_json::Value,
    plan_bytes: &[u8],
    index: &DockerRequestIndex<'_>,
) -> Result<(), Box<dyn Error>> {
    if !docker_closed_fields(
        sidecar,
        &[
            "schema_version",
            "kind",
            "native_execution",
            "docker_plan_sha256",
            "decisions",
        ],
    ) || sidecar["schema_version"] != 1
        || sidecar["kind"] != "boxferry-docker-dependency-decisions"
        || sidecar["native_execution"] != false
        || sidecar["docker_plan_sha256"] != lowercase_hex(&Sha256::digest(plan_bytes))
    {
        return Err("Docker dependency sidecar envelope or exact plan binding differs".into());
    }
    let actual = sidecar["decisions"]
        .as_array()
        .ok_or("Docker dependency decisions missing")?;
    if actual.len() != route.docker_dependencies.len() {
        return Err("Docker dependency decision inventory differs".into());
    }
    let mut edges = BTreeSet::new();
    for decision in actual {
        let service = decision["service"]
            .as_str()
            .ok_or("Docker dependency service missing")?;
        let dependency = decision["dependency"]
            .as_str()
            .ok_or("Docker dependency target missing")?;
        if !edges.insert((service, dependency)) {
            return Err("Docker dependency decisions repeat an edge".into());
        }
        validate_docker_dependency_decision(manifest, route, decision, index, service, dependency)?;
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_dependency_decision(
    manifest: &ScenarioManifest,
    route: &RouteExpectation,
    decision: &serde_json::Value,
    index: &DockerRequestIndex<'_>,
    service: &str,
    dependency: &str,
) -> Result<(), Box<dyn Error>> {
    if !docker_closed_fields(
        decision,
        &[
            "service",
            "service_runtime_name",
            "dependency",
            "dependency_runtime_name",
            "condition",
            "condition_explicit",
            "required",
            "required_explicit",
            "restart",
            "restart_explicit",
            "fidelity",
            "native_engine_field",
            "provenance",
        ],
    ) {
        return Err("Docker dependency decision fields differ".into());
    }
    let expected = route
        .docker_dependencies
        .iter()
        .find(|edge| edge.service == service && edge.dependency == dependency)
        .ok_or("Docker dependency edge differs from authored source")?;
    for (key, value) in [
        ("condition", serde_json::json!(expected.condition)),
        (
            "condition_explicit",
            serde_json::json!(expected.condition_explicit.is_explicit()),
        ),
        ("required", serde_json::json!(expected.required)),
        (
            "required_explicit",
            serde_json::json!(expected.required_explicit.is_explicit()),
        ),
        ("restart", serde_json::json!(expected.restart)),
        (
            "restart_explicit",
            serde_json::json!(expected.restart_explicit.is_explicit()),
        ),
        ("fidelity", serde_json::json!("approximate")),
        ("native_engine_field", serde_json::json!(false)),
    ] {
        if decision[key] != value {
            return Err("Docker dependency choices differ from authored source".into());
        }
    }
    for (key, logical) in [
        ("service_runtime_name", service),
        ("dependency_runtime_name", dependency),
    ] {
        let name = authored_docker_runtime_name(manifest, logical)?;
        if decision[key] != name || !index.creates.contains_key(&name) {
            return Err("Docker dependency runtime identity differs from authored service".into());
        }
    }
    validate_docker_dependency_provenance(&decision["provenance"])?;
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_docker_dependency_provenance(provenance: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    if !docker_closed_fields(provenance, &["reference", "condition", "required", "restart"]) {
        return Err("Docker dependency provenance fields differ".into());
    }
    for key in ["reference", "condition", "required", "restart"] {
        let values = provenance[key]
            .as_array()
            .ok_or("Docker dependency provenance categories missing")?;
        if values.len() > 32
            || values.iter().any(|value| {
                !value.as_str().is_some_and(|value| {
                    matches!(
                        value,
                        "source_document"
                            | "runtime_observation"
                            | "user_override"
                            | "implementation_default"
                            | "conversion_decision"
                    )
                })
            })
        {
            return Err("Docker dependency provenance includes unreviewed categories".into());
        }
    }
    Ok(())
}

#[cfg(feature = "docker")]
fn validate_exact_docker_bind_inventory(
    plan: &serde_json::Value,
    expected: &[DockerBindExpectation],
) -> Result<(), Box<dyn Error>> {
    let parsed = parse_docker_prerequisites(plan)?;
    let requests = plan["requests"]
        .as_array()
        .ok_or("Docker exact inventory lacks requests")?;
    let api = plan["context"]["rendering_api_version"]
        .as_str()
        .ok_or("Docker exact bind inventory lacks API context")?;
    let inputs = DockerPlanInputs {
        requests,
        api_prefix: format!("/v{api}/"),
        prerequisite_identities: parsed.identities,
        bind_prerequisites: parsed.binds,
        network_internal: parsed.network_internal,
    };
    let index = index_docker_requests(requests, &inputs.api_prefix)?;
    validate_docker_bind_mounts(expected, &inputs, &index)
}

#[cfg(feature = "docker")]
fn expected_exact_docker_health(
    manifest: &ScenarioManifest,
    service: &str,
) -> Result<Option<serde_json::Value>, Box<dyn Error>> {
    let Some(health) = manifest
        .semantics
        .required_healthchecks
        .iter()
        .find(|health| health.service == service)
    else {
        return Ok(None);
    };
    // The six independently authored Nextcloud overlay tests all specify 10s, 3s and 12.
    // Other timing contracts need their own source evidence before exact inventory admission.
    if manifest.id != "nextcloud-application" || health.command_kind != "exec" {
        return Err("Docker exact health timing contract is not independently reviewed".into());
    }
    let test = std::iter::once("CMD")
        .chain(health.command.iter().map(String::as_str))
        .collect::<Vec<_>>();
    Ok(Some(
        serde_json::json!({"Test":test,"Interval":10_000_000_000_u64,"Timeout":3_000_000_000_u64,"Retries":12}),
    ))
}

#[cfg(feature = "docker")]
fn expected_exact_docker_restart(
    manifest: &ScenarioManifest,
    service: &str,
) -> Result<Option<serde_json::Value>, Box<dyn Error>> {
    let policy = manifest.semantics.required_restart.iter().find_map(|value| {
        value
            .split_once(':')
            .filter(|(name, _)| *name == service)
            .map(|(_, policy)| policy)
    });
    let name = match policy {
        None | Some("absent") => return Ok(None),
        Some("never") => "no",
        Some("always") => "always",
        Some("unless-stopped") => "unless-stopped",
        _ => return Err("Docker exact restart contract lacks independently reviewed retry choices".into()),
    };
    Ok(Some(serde_json::json!({"Name":name,"MaximumRetryCount":0})))
}

#[cfg(feature = "docker")]
#[allow(
    clippy::too_many_lines,
    reason = "the exact native inventory checks one authored route contract as a unit"
)]
fn validate_exact_docker_inventory_for_route(
    manifest: &ScenarioManifest,
    route: Option<&RouteExpectation>,
    plan: &serde_json::Value,
    bind_expectations: &[DockerBindExpectation],
) -> Result<(), Box<dyn Error>> {
    let parsed = parse_docker_prerequisites(plan)?;
    validate_docker_external_internal_for_route(manifest, route, &parsed.network_internal)?;
    if bind_expectations.len() != manifest.semantics.required_bind_mounts.len() {
        return Err("Docker exact inventory lacks source-corroborated bind expectations".into());
    }
    if !bind_expectations.is_empty() {
        validate_exact_docker_bind_inventory(plan, bind_expectations)?;
    }
    let requests = plan["requests"]
        .as_array()
        .ok_or("Docker exact inventory lacks requests")?;
    let mut actual_networks = BTreeSet::new();
    let mut actual_volumes = BTreeSet::new();
    let mut containers = BTreeMap::new();
    let mut connections = BTreeMap::new();
    for request in requests {
        let path = request["path"]
            .as_str()
            .ok_or("Docker exact inventory request lacks path")?;
        if path.ends_with("/networks/create") {
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("Docker network create lacks name")?;
            if !actual_networks.insert(name) {
                return Err("Docker exact inventory repeats a network create".into());
            }
            let expected = manifest
                .semantics
                .required_networks
                .iter()
                .find(|network| network.name == name && network.ownership == "application")
                .ok_or("Docker exact inventory has an undeclared network create")?;
            if request["body"]["Driver"] != "bridge" {
                return Err("Docker exact inventory changed the network driver".into());
            }
            if expected.internal == Some(true) && request["body"]["Internal"] != true {
                return Err("Docker exact inventory changed network isolation".into());
            }
            let mut expected_fields = BTreeSet::from(["Name", "Driver"]);
            if expected.internal == Some(true) {
                expected_fields.insert("Internal");
            }
            if request["body"]
                .as_object()
                .is_none_or(|body| body.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_fields)
            {
                return Err("Docker exact inventory invented network create fields".into());
            }
        } else if path.ends_with("/volumes/create") {
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("Docker volume create lacks name")?;
            if !actual_volumes.insert(name) {
                return Err("Docker exact inventory repeats a volume create".into());
            }
            if request["body"]
                .as_object()
                .is_none_or(|body| body.keys().map(String::as_str).collect::<BTreeSet<_>>() != BTreeSet::from(["Name"]))
            {
                return Err("Docker exact inventory invented volume create fields".into());
            }
        } else if let Some((_, name)) = path.split_once("/containers/create?name=") {
            if name.is_empty() || containers.insert(name, &request["body"]).is_some() {
                return Err("Docker exact inventory repeats a container create".into());
            }
        } else if let Some(network) = path
            .split_once("/networks/")
            .and_then(|(_, suffix)| suffix.strip_suffix("/connect"))
        {
            let container = request["body"]["Container"]
                .as_str()
                .ok_or("Docker network connect lacks container identity")?;
            if network.is_empty()
                || container.is_empty()
                || connections.insert((container, network), &request["body"]).is_some()
            {
                return Err("Docker exact inventory repeats a network connect".into());
            }
            let endpoint = &request["body"]["EndpointConfig"];
            if !endpoint.is_null() && !endpoint.as_object().is_some_and(serde_json::Map::is_empty) {
                return Err("Docker exact inventory invented network endpoint options".into());
            }
            if request["body"].as_object().is_none_or(|body| {
                body.keys().map(String::as_str).collect::<BTreeSet<_>>()
                    != BTreeSet::from(["Container", "EndpointConfig"])
            }) {
                return Err("Docker exact inventory invented network connect fields".into());
            }
        } else {
            return Err("Docker exact inventory contains an undeclared operation".into());
        }
    }
    let expected_networks = manifest
        .semantics
        .required_networks
        .iter()
        .filter(|network| network.ownership == "application")
        .map(|network| network.name.as_str())
        .collect::<BTreeSet<_>>();
    if actual_networks != expected_networks {
        return Err("Docker exact inventory changed the owned network create set".into());
    }
    let expected_volumes = manifest
        .semantics
        .ownership_boundaries
        .iter()
        .filter_map(|boundary| {
            boundary
                .strip_prefix("volume:")
                .and_then(|value| value.rsplit_once(':'))
        })
        .filter_map(|(name, ownership)| (ownership == "application").then_some(name))
        .collect::<BTreeSet<_>>();
    if actual_volumes != expected_volumes {
        return Err("Docker exact inventory changed the owned volume create set".into());
    }
    let expected_prerequisites = manifest
        .semantics
        .required_networks
        .iter()
        .filter(|network| network.ownership == "external")
        .map(|network| ("network", network.name.as_str()))
        .chain(
            manifest
                .semantics
                .ownership_boundaries
                .iter()
                .filter_map(|boundary| {
                    boundary
                        .strip_prefix("volume:")
                        .and_then(|value| value.rsplit_once(':'))
                })
                .filter_map(|(name, ownership)| (ownership == "external").then_some(("volume", name))),
        )
        .collect::<BTreeSet<_>>();
    let mut actual_prerequisites = BTreeSet::new();
    for prerequisite in plan["prerequisites"]
        .as_array()
        .ok_or("Docker exact inventory lacks prerequisites")?
    {
        let kind = prerequisite["kind"].as_str().ok_or("Docker prerequisite lacks kind")?;
        if kind == "bind_source" && !bind_expectations.is_empty() {
            // Complete obligations, association and exact bind inventory were checked above.
            continue;
        }
        let name = prerequisite["identity"]
            .as_str()
            .ok_or("Docker prerequisite lacks identity")?;
        if !actual_prerequisites.insert((kind, name)) {
            return Err("Docker exact inventory repeats a prerequisite".into());
        }
    }
    if actual_prerequisites != expected_prerequisites {
        return Err("Docker exact inventory changed the external prerequisite set".into());
    }
    for component in &manifest.components {
        let runtime_name = manifest
            .semantics
            .required_runtime_names
            .iter()
            .find_map(|name| {
                name.split_once(':')
                    .filter(|(service, _)| *service == component.name)
                    .map(|(_, native)| native)
            })
            .unwrap_or(&component.name);
        let body = containers
            .get(runtime_name)
            .ok_or("Docker exact inventory lacks an authored container")?;
        let expected_user = manifest.semantics.required_users.iter().find_map(|user| {
            user.split_once(':')
                .filter(|(service, _)| *service == component.name)
                .map(|(_, value)| value)
        });
        if body["User"].as_str() != expected_user {
            return Err("Docker exact inventory changed the container user".into());
        }
        let expected_command = manifest
            .semantics
            .required_commands
            .iter()
            .find(|command| command.service == component.name);
        let mut expected_container_fields = BTreeSet::from(["Image", "HostConfig", "NetworkingConfig"]);
        match expected_command {
            Some(command) if command.kind == "exec" => {
                if body["Cmd"] != serde_json::json!(command.values) {
                    return Err("Docker exact inventory changed the container command".into());
                }
                expected_container_fields.insert("Cmd");
            }
            Some(command) if command.kind == "absent" => {
                if !body["Cmd"].is_null() {
                    return Err("Docker exact inventory invented a container command".into());
                }
            }
            None => {
                if !body["Cmd"].is_null() {
                    return Err("Docker exact inventory invented a container command".into());
                }
            }
            Some(_) => return Err("Docker exact inventory has an unsupported authored command kind".into()),
        }
        if manifest.semantics.required_environment.iter().any(|assignment| {
            assignment
                .split_once(':')
                .is_some_and(|(service, _)| service == component.name)
        }) {
            expected_container_fields.insert("Env");
        }
        if expected_user.is_some() {
            expected_container_fields.insert("User");
        }
        let expected_health = expected_exact_docker_health(manifest, &component.name)?;
        if expected_health
            .as_ref()
            .is_some_and(|health| body["Healthcheck"] != *health)
            || expected_health.is_none() && !body["Healthcheck"].is_null()
        {
            return Err("Docker exact inventory changed authored healthcheck fields".into());
        }
        if expected_health.is_some() {
            expected_container_fields.insert("Healthcheck");
        }
        if manifest
            .semantics
            .required_port_bindings
            .iter()
            .any(|port| port.service == component.name)
        {
            expected_container_fields.insert("ExposedPorts");
        }
        if body.as_object().is_none_or(|fields| {
            fields.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_container_fields
        }) {
            return Err("Docker exact inventory invented container create fields".into());
        }
        let authored_networks = manifest
            .semantics
            .exact_network_memberships
            .iter()
            .find_map(|membership| {
                membership
                    .split_once(':')
                    .filter(|(service, _)| *service == component.name)
                    .map(|(_, names)| names.split(',').collect::<Vec<_>>())
            })
            .ok_or("Docker exact inventory lacks authored network memberships")?;
        let primary_network = *authored_networks
            .first()
            .ok_or("Docker exact inventory lacks an authored primary network")?;
        let expected_networks = authored_networks.into_iter().collect::<BTreeSet<_>>();
        let endpoints = &body["NetworkingConfig"]["EndpointsConfig"];
        let primary_networks = endpoints
            .as_object()
            .ok_or("Docker exact inventory lacks network attachments")?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if primary_networks != BTreeSet::from([primary_network]) {
            return Err("Docker exact inventory changed the primary network attachment".into());
        }
        if body["NetworkingConfig"].as_object().is_none_or(|fields| {
            fields.keys().map(String::as_str).collect::<BTreeSet<_>>() != BTreeSet::from(["EndpointsConfig"])
        }) {
            return Err("Docker exact inventory invented networking configuration fields".into());
        }
        if body["HostConfig"]["NetworkMode"] != primary_network {
            return Err("Docker exact inventory changed the network mode".into());
        }
        let mut expected_host_fields = BTreeSet::from(["NetworkMode"]);
        let expected_restart = expected_exact_docker_restart(manifest, &component.name)?;
        if expected_restart
            .as_ref()
            .is_some_and(|policy| body["HostConfig"]["RestartPolicy"] != *policy)
            || expected_restart.is_none() && !body["HostConfig"]["RestartPolicy"].is_null()
        {
            return Err("Docker exact inventory changed authored restart policy fields".into());
        }
        if expected_restart.is_some() {
            expected_host_fields.insert("RestartPolicy");
        }
        if bind_expectations.iter().any(|bind| bind.identity == runtime_name) {
            expected_host_fields.insert("Binds");
        }
        if manifest
            .semantics
            .required_volume_mounts
            .iter()
            .any(|mount| mount.service == component.name)
        {
            expected_host_fields.insert("Mounts");
        }
        if manifest
            .semantics
            .required_port_bindings
            .iter()
            .any(|port| port.service == component.name)
        {
            expected_host_fields.insert("PortBindings");
        }
        if body["HostConfig"]
            .as_object()
            .is_none_or(|fields| fields.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_host_fields)
        {
            return Err("Docker exact inventory invented host configuration fields".into());
        }
        if connections
            .keys()
            .any(|(container, network)| *container == runtime_name && primary_networks.contains(network))
        {
            return Err("Docker exact inventory repeats a network attachment".into());
        }
        let actual_networks = primary_networks
            .iter()
            .copied()
            .chain(
                connections
                    .keys()
                    .filter_map(|(container, network)| (*container == runtime_name).then_some(*network)),
            )
            .collect::<BTreeSet<_>>();
        if actual_networks != expected_networks {
            return Err("Docker exact inventory changed authored network attachment set".into());
        }
        if endpoints.as_object().is_some_and(|values| {
            values
                .values()
                .any(|value| !value.is_null() && !value.as_object().is_some_and(serde_json::Map::is_empty))
        }) {
            return Err("Docker exact inventory invented network endpoint options".into());
        }
        let expected_environment = manifest
            .semantics
            .required_environment
            .iter()
            .filter_map(|value| {
                value
                    .split_once(':')
                    .filter(|(service, _)| *service == component.name)
                    .map(|(_, assignment)| assignment)
            })
            .collect::<BTreeSet<_>>();
        let mut actual_environment = BTreeSet::new();
        if let Some(values) = body["Env"].as_array() {
            for value in values {
                let assignment = value
                    .as_str()
                    .ok_or("Docker exact inventory has malformed environment")?;
                if !actual_environment.insert(assignment) {
                    return Err("Docker exact inventory repeats an environment assignment".into());
                }
            }
        } else if !body["Env"].is_null() {
            return Err("Docker exact inventory has malformed environment".into());
        }
        if actual_environment != expected_environment {
            return Err("Docker exact inventory changed authored environment assignments".into());
        }
        let expected_mounts = manifest
            .semantics
            .required_volume_mounts
            .iter()
            .filter(|mount| mount.service == component.name)
            .map(|mount| ("volume", mount.source.as_str(), mount.target.as_str(), mount.read_only))
            .collect::<BTreeSet<_>>();
        let mut actual_mounts = BTreeSet::new();
        if let Some(mounts) = body["HostConfig"]["Mounts"].as_array() {
            for mount in mounts {
                let kind = mount["Type"]
                    .as_str()
                    .ok_or("Docker exact inventory has malformed mount type")?;
                let source = mount["Source"]
                    .as_str()
                    .ok_or("Docker exact inventory has malformed mount source")?;
                let target = mount["Target"]
                    .as_str()
                    .ok_or("Docker exact inventory has malformed mount target")?;
                let read_only = mount["ReadOnly"]
                    .as_bool()
                    .ok_or("Docker exact inventory has malformed mount mode")?;
                if mount.as_object().is_none_or(|fields| {
                    fields.keys().map(String::as_str).collect::<BTreeSet<_>>()
                        != BTreeSet::from(["Type", "Source", "Target", "ReadOnly"])
                }) {
                    return Err("Docker exact inventory invented mount fields".into());
                }
                if !actual_mounts.insert((kind, source, target, read_only)) {
                    return Err("Docker exact inventory repeats a mount".into());
                }
            }
        } else if !body["HostConfig"]["Mounts"].is_null() {
            return Err("Docker exact inventory has malformed mounts".into());
        }
        if actual_mounts != expected_mounts {
            return Err("Docker exact inventory changed authored mount set".into());
        }
        let expected_ports = manifest
            .semantics
            .required_port_bindings
            .iter()
            .filter(|port| port.service == component.name)
            .map(|port| format!("{}/{}", port.container, port.protocol))
            .collect::<BTreeSet<_>>();
        let exposed = &body["ExposedPorts"];
        let actual_ports = if exposed.is_null() {
            BTreeSet::new()
        } else {
            let by_port = exposed
                .as_object()
                .ok_or("Docker exact inventory has malformed exposed ports")?;
            if by_port
                .values()
                .any(|value| !value.as_object().is_some_and(serde_json::Map::is_empty))
            {
                return Err("Docker exact inventory invented an exposed port value".into());
            }
            by_port.keys().map(String::as_str).collect::<BTreeSet<_>>()
        };
        if actual_ports != expected_ports.iter().map(String::as_str).collect::<BTreeSet<_>>() {
            return Err("Docker exact inventory changed authored exposed ports".into());
        }
        let expected_bindings = manifest
            .semantics
            .required_port_bindings
            .iter()
            .filter(|port| port.service == component.name)
            .map(|port| {
                (
                    format!("{}/{}", port.container, port.protocol),
                    port.host_address.clone(),
                    port.published
                        .map_or_else(String::new, |published| published.to_string()),
                )
            })
            .collect::<BTreeSet<_>>();
        let mut actual_bindings = BTreeSet::new();
        let bindings = &body["HostConfig"]["PortBindings"];
        if let Some(by_port) = bindings.as_object() {
            if by_port.keys().map(String::as_str).collect::<BTreeSet<_>>()
                != expected_ports.iter().map(String::as_str).collect::<BTreeSet<_>>()
            {
                return Err("Docker exact inventory changed authored host binding set".into());
            }
            for (port, values) in by_port {
                for value in values
                    .as_array()
                    .ok_or("Docker exact inventory has malformed port bindings")?
                {
                    let host_port = value["HostPort"]
                        .as_str()
                        .ok_or("Docker exact inventory has malformed host port")?;
                    let host_ip = value
                        .as_object()
                        .ok_or("Docker exact inventory has malformed host binding")?
                        .get("HostIp")
                        .map(|host_ip| host_ip.as_str().ok_or("Docker exact inventory has malformed host IP"))
                        .transpose()?;
                    let expected_fields = if host_ip.is_some() {
                        BTreeSet::from(["HostIp", "HostPort"])
                    } else {
                        BTreeSet::from(["HostPort"])
                    };
                    if value.as_object().is_none_or(|fields| {
                        fields.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected_fields
                    }) {
                        return Err("Docker exact inventory invented host binding fields".into());
                    }
                    if !actual_bindings.insert((port.to_owned(), host_ip.map(str::to_owned), host_port.to_owned())) {
                        return Err("Docker exact inventory repeats a host binding".into());
                    }
                }
            }
        } else if !bindings.is_null() {
            return Err("Docker exact inventory has malformed port bindings".into());
        }
        if actual_bindings != expected_bindings {
            return Err("Docker exact inventory changed authored host binding set".into());
        }
    }
    if containers.len() != manifest.components.len() {
        return Err("Docker exact inventory added a container create".into());
    }
    if connections.keys().any(|(container, network)| {
        !containers.contains_key(container)
            || !actual_networks.contains(network) && !expected_prerequisites.contains(&("network", *network))
    }) {
        return Err("Docker exact inventory connected an undeclared resource".into());
    }
    Ok(())
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
        let external = value["external_preconditions"]
            .as_array()
            .ok_or("native external preconditions")?;
        let actual = creates
            .iter()
            .map(|operation| &operation["resource"])
            .chain(external.iter())
            .map(|resource| {
                let kind = match resource["kind"].as_str().ok_or("resource kind")? {
                    "container" => "service",
                    "volume" => "volume",
                    "network" => "network",
                    _ => return Err("unreviewed Podman resource kind".into()),
                };
                Ok(format!("{kind}:{}", resource["name"].as_str().ok_or("resource name")?))
            })
            .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
        let expected = manifest
            .semantics
            .selected_resources
            .iter()
            .map(|resource| {
                resource.strip_prefix("service:").map_or_else(
                    || resource.clone(),
                    |service| format!("service:{}", expected_podman_container_name(manifest, service)),
                )
            })
            .collect();
        assert_eq!(actual, expected);
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
        let expected_name = expected_podman_container_name(manifest, &component.name);
        let create = creates
            .iter()
            .find(|operation| {
                operation["resource"]["kind"] == "container" && operation["resource"]["name"] == expected_name
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
        validate_podman_container_identity(
            operations,
            create,
            options,
            expected_name,
            expected_podman_pod_name(manifest, &component.name),
        )?;
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

fn expected_podman_container_name<'a>(manifest: &'a ScenarioManifest, service: &'a str) -> &'a str {
    // Expectations come from authored semantic declarations, never observed output.
    manifest
        .semantics
        .required_runtime_names
        .iter()
        .filter_map(|requirement| requirement.split_once(':'))
        .find_map(|(logical, native)| (logical == service).then_some(native))
        .unwrap_or(service)
}

fn expected_podman_pod_name<'a>(manifest: &'a ScenarioManifest, service: &str) -> Option<&'a str> {
    manifest
        .semantics
        .required_groups
        .iter()
        .find(|group| group.ownership == "application" && group.members.iter().any(|member| member == service))
        .map(|group| group.name.as_str())
}

fn validate_podman_container_identity(
    operations: &[serde_json::Value],
    create: &serde_json::Value,
    options: &[serde_json::Value],
    expected_name: &str,
    expected_pod: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    if create["resource"]["name"] != expected_name {
        return Err("Podman create resource differs from authored container identity".into());
    }
    if option_values(options, &["--name"]).as_slice() != [expected_name] {
        return Err("Podman create CLI differs from authored container identity".into());
    }
    let encoded_name = expected_name.bytes().fold(String::new(), |mut encoded, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
        encoded
    });
    if !create["libpod"]["path_and_query"]
        .as_str()
        .is_some_and(|path| path.ends_with(&format!("/libpod/containers/create?name={encoded_name}")))
    {
        return Err("Podman create API differs from authored container identity".into());
    }
    let actual_pod = create["libpod"]["body"]["json"].get("pod");
    if actual_pod.and_then(serde_json::Value::as_str) != expected_pod
        || actual_pod.is_some_and(|pod| !pod.is_string())
        || option_values(options, &["--pod"]) != expected_pod.into_iter().collect::<Vec<_>>()
    {
        return Err("Podman pod association differs from authored group membership".into());
    }
    if expected_pod.is_some() {
        return Ok(());
    }
    let starts = operations
        .iter()
        .filter(|operation| {
            operation["action"] == "start_container"
                && operation["resource"]["kind"] == "container"
                && operation["resource"]["name"] == expected_name
        })
        .collect::<Vec<_>>();
    if starts.len() != 1 {
        return Err("Podman start resource differs from authored container identity".into());
    }
    if starts[0]["cli"]["argv"] != serde_json::json!(["container", "start", expected_name]) {
        return Err("Podman start CLI differs from authored container identity".into());
    }
    if !starts[0]["libpod"]["path_and_query"]
        .as_str()
        .is_some_and(|path| path.ends_with(&format!("/libpod/containers/{encoded_name}/start")))
    {
        return Err("Podman start API differs from authored container identity".into());
    }
    Ok(())
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
        let mount_options = podman_named_volume_options(expected.read_only, expected.selinux_relabel.as_deref())?;
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

fn podman_named_volume_options(read_only: bool, selinux_relabel: Option<&str>) -> Result<Vec<String>, Box<dyn Error>> {
    let access = if read_only { "ro" } else { "rw" };
    let mut options = vec![access.to_owned(), "copy".to_owned()];
    match selinux_relabel {
        None => {}
        Some("shared") => options.push("z".into()),
        Some("private") => options.push("Z".into()),
        Some(_) => return Err("unvalidated named-volume SELinux relabel".into()),
    }
    Ok(options)
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
        let typed_mount = manifest
            .semantics
            .required_volume_mounts
            .iter()
            .find(|mount| mount.service == owner && mount.source == volume && mount.target == destination);
        let mount_options = podman_named_volume_options(
            typed_mount.is_some_and(|mount| mount.read_only),
            typed_mount.and_then(|mount| mount.selinux_relabel.as_deref()),
        )?;
        let expected = format!("{volume}:{destination}:{}", mount_options.join(","));
        assert!(
            volumes.contains(&expected.as_str()),
            "required CLI volume option changed"
        );
        let native = body["volumes"].as_array().ok_or("native named volumes")?;
        assert!(
            native.iter().any(|entry| entry["Name"] == volume
                && entry["Dest"] == destination
                && entry["Options"] == serde_json::json!(mount_options)),
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
fn podman_container_identity_counterfactuals_detect_each_native_surface() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/conversion/document-route-matrix");
    let manifest = read_manifest_file(&fixture.join("document-route-normal.scenario.toml"))?;
    assert_eq!(expected_podman_container_name(&manifest, "web"), "web-runtime");
    assert_eq!(manifest.components[0].name, "web");
    let baseline = serde_json::json!([
        {
            "action": "create",
            "resource": { "kind": "container", "name": "web-runtime" },
            "cli": { "argv": ["container", "create", "--name", "web-runtime"] },
            "libpod": { "path_and_query": "/v6.1.0/libpod/containers/create?name=web-runtime" }
        },
        {
            "action": "start_container",
            "resource": { "kind": "container", "name": "web-runtime" },
            "cli": { "argv": ["container", "start", "web-runtime"] },
            "libpod": { "path_and_query": "/v6.1.0/libpod/containers/web-runtime/start" }
        }
    ]);
    let check = |value: &serde_json::Value| -> Result<(), Box<dyn Error>> {
        let operations = value.as_array().ok_or("identity operations")?;
        let create = operations.first().ok_or("identity create")?;
        let options = create["cli"]["argv"].as_array().ok_or("identity options")?;
        validate_podman_container_identity(
            operations,
            create,
            options,
            "web-runtime",
            expected_podman_pod_name(&manifest, "web"),
        )
    };
    check(&baseline)?;
    for (pointer, replacement, reason) in [
        ("/0/resource/name", serde_json::json!("web"), "create resource"),
        ("/0/cli/argv/3", serde_json::json!("web"), "create CLI"),
        (
            "/0/libpod/path_and_query",
            serde_json::json!("/v6.1.0/libpod/containers/create?name=web"),
            "create API",
        ),
        ("/1/resource/name", serde_json::json!("web"), "start resource"),
        ("/1/cli/argv/2", serde_json::json!("web"), "start CLI"),
        (
            "/1/libpod/path_and_query",
            serde_json::json!("/v6.1.0/libpod/containers/web/start"),
            "start API",
        ),
    ] {
        let mut changed = baseline.clone();
        *changed.pointer_mut(pointer).ok_or("identity mutation pointer")? = replacement;
        let error = check(&changed).err().ok_or("identity mutation must fail")?;
        assert!(
            error.to_string().contains(reason),
            "{pointer} failed for the wrong reason: {error}"
        );
    }
    let mut forged = baseline.clone();
    forged[0]["libpod"]["body"] = serde_json::json!({ "json": { "pod": "forged-pod" } });
    forged[1]["resource"]["name"] = serde_json::json!("web");
    let error = check(&forged)
        .err()
        .ok_or("forged pod must not hide invalid start identity")?;
    assert_eq!(
        error.to_string(),
        "Podman pod association differs from authored group membership"
    );
    Ok(())
}

#[test]
fn podman_container_identity_uses_authored_group_membership() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/conversion/document-route-matrix");
    let mut manifest = read_manifest_file(&fixture.join("document-route-normal.scenario.toml"))?;
    manifest.semantics.required_groups.push(toml::from_str(
        "name = 'authored-pod'\nownership = 'application'\nmembers = ['web']\n",
    )?);
    let expected_pod = expected_podman_pod_name(&manifest, "web");
    assert_eq!(expected_pod, Some("authored-pod"));
    assert_eq!(expected_podman_pod_name(&manifest, "other"), None);
    let create = serde_json::json!({
        "action": "create",
        "resource": { "kind": "container", "name": "web-runtime" },
        "cli": { "argv": ["container", "create", "--name", "web-runtime", "--pod", "authored-pod"] },
        "libpod": {
            "path_and_query": "/v6.1.0/libpod/containers/create?name=web-runtime",
            "body": { "json": { "pod": "authored-pod" } }
        }
    });
    let options = create["cli"]["argv"].as_array().ok_or("pod member options")?;
    // Authored pod members share the pod start operation and need no individual start.
    validate_podman_container_identity(&[], &create, options, "web-runtime", expected_pod)?;
    for (pointer, replacement) in [
        ("/libpod/body/json/pod", serde_json::json!("other-pod")),
        ("/cli/argv/5", serde_json::json!("other-pod")),
    ] {
        let mut changed = create.clone();
        *changed.pointer_mut(pointer).ok_or("pod mutation pointer")? = replacement;
        let options = changed["cli"]["argv"].as_array().ok_or("changed pod member options")?;
        let error = validate_podman_container_identity(&[], &changed, options, "web-runtime", expected_pod)
            .err()
            .ok_or("pod membership mutation must fail")?;
        assert_eq!(
            error.to_string(),
            "Podman pod association differs from authored group membership"
        );
    }
    manifest.semantics.required_groups[0].ownership = "uncertain".into();
    assert_eq!(expected_podman_pod_name(&manifest, "web"), None);
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
fn withheld_reimport_requires_absence_without_weakening_source_assertions() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/authored-core");
    let manifest = read_manifest(&fixture)?;
    let source = fs::read_to_string(fixture.join("input-compose.yaml"))?;
    let expected_gap = ["environment:app:APP_MODE".to_owned()];

    let original = import_compose(&source, &manifest.application.name)?;
    let original = original.application().ok_or("original Compose import")?;
    validate_neutral_application(&manifest, original, &[], &[])?;
    let error = validate_withheld_reimport(&manifest, original, &[], &expected_gap)
        .err()
        .ok_or("default-withheld validation accepted an original literal")?;
    assert!(error.contains("literal environment assignment app:APP_MODE"));

    let absent = source.replace("    environment:\n      APP_MODE: migration\n", "");
    assert_ne!(absent, source);
    let imported = import_compose(&absent, &manifest.application.name)?;
    let imported = imported.application().ok_or("Compose import without environment")?;
    validate_withheld_reimport(&manifest, imported, &[], &expected_gap)?;
    let error = validate_neutral_application(&manifest, imported, &[], &[])
        .err()
        .ok_or("source validation accepted a missing literal")?;
    assert!(error.contains("required environment app:APP_MODE changed"));

    let present_without_literal = source.replace("APP_MODE: migration", "APP_MODE:");
    let imported = import_compose(&present_without_literal, &manifest.application.name)?;
    let error = validate_withheld_reimport(
        &manifest,
        imported
            .application()
            .ok_or("Compose import with valueless assignment")?,
        &[],
        &expected_gap,
    )
    .err()
    .ok_or("default-withheld validation accepted a valueless assignment")?;
    assert!(error.contains("withheld environment app:APP_MODE remains assigned"));

    let empty_value = source.replace("APP_MODE: migration", "APP_MODE: ''");
    let imported = import_compose(&empty_value, &manifest.application.name)?;
    let error = validate_withheld_reimport(
        &manifest,
        imported.application().ok_or("Compose import with empty literal")?,
        &[],
        &expected_gap,
    )
    .err()
    .ok_or("default-withheld validation accepted an empty literal")?;
    assert!(error.contains("literal environment assignment app:APP_MODE"));

    let unrelated_literal = absent.replace("  database:\n", "  database:\n    environment:\n      LEAK: visible\n");
    let imported = import_compose(&unrelated_literal, &manifest.application.name)?;
    let error = validate_withheld_reimport(
        &manifest,
        imported.application().ok_or("Compose import with unrelated literal")?,
        &[],
        &expected_gap,
    )
    .err()
    .ok_or("default-withheld validation accepted an unrelated literal")?;
    assert!(error.contains("literal environment assignment database:LEAK"));
    Ok(())
}

fn assert_paperless_semantic_omissions(root: &Path) -> Result<(), Box<dyn Error>> {
    for (scenario, name, needle, expected) in [
        (
            "paperless-ngx-application",
            "storage",
            "      - data:/usr/src/paperless/data\n",
            "volume data shared consumer boundary changed",
        ),
        (
            "paperless-ngx-application",
            "database",
            "      PAPERLESS_DBHOST: db\n",
            "required environment webserver:PAPERLESS_DBHOST changed",
        ),
        (
            "paperless-ngx-application",
            "broker",
            "      PAPERLESS_REDIS: redis://broker:6379\n",
            "required environment webserver:PAPERLESS_REDIS changed",
        ),
        (
            "paperless-ngx-application",
            "worker",
            "      PAPERLESS_TASK_WORKERS: \"${PAPERLESS_TASK_WORKERS:?set PAPERLESS_TASK_WORKERS}\"\n",
            "required environment webserver:PAPERLESS_TASK_WORKERS changed",
        ),
        (
            "paperless-ngx-application",
            "storage-identity",
            "      USERMAP_UID: \"${USERMAP_UID:?set USERMAP_UID}\"\n",
            "required environment webserver:USERMAP_UID changed",
        ),
        (
            "paperless-ngx-application",
            "publication",
            "      - \"127.0.0.1:18000:8000\"\n",
            "publication:webserver:18000:8000/tcp",
        ),
        (
            "paperless-ngx-document-converters",
            "converter-enable",
            "      PAPERLESS_TIKA_ENABLED: \"${PAPERLESS_TIKA_ENABLED:?set PAPERLESS_TIKA_ENABLED}\"\n",
            "required environment webserver:PAPERLESS_TIKA_ENABLED changed",
        ),
        (
            "paperless-ngx-document-converters",
            "converter-endpoint",
            "      PAPERLESS_TIKA_GOTENBERG_ENDPOINT: \"${PAPERLESS_TIKA_GOTENBERG_ENDPOINT:?set PAPERLESS_TIKA_GOTENBERG_ENDPOINT}\"\n",
            "required environment webserver:PAPERLESS_TIKA_GOTENBERG_ENDPOINT changed",
        ),
    ] {
        let fixture = root.join("fixtures/scenarios").join(scenario);
        let manifest = read_manifest(&fixture)?;
        let source = fs::read_to_string(fixture.join("compose.yaml"))?;
        let mutation = source.replacen(needle, "", 1);
        assert_ne!(source, mutation, "{name} must actually mutate Paperless input");

        let directory = TemporaryDirectory::new(&format!("paperless-{name}-mutation"))?;
        fs::write(directory.path().join("compose.yaml"), mutation)?;
        let environment = parse_scenario_environment_file(&fixture.join("paperless.env.example"))?;
        let input = compose_mutation_input(Vec::new(), environment);
        let imported = import_compose_files(&input, directory.path(), &manifest.application.name)?;
        let error = validate_neutral_application(
            &manifest,
            imported.application().ok_or("mutated Paperless import")?,
            &manifest.semantics.required_environment_order,
            &[],
        )
        .err()
        .ok_or("Paperless semantic mutation must fail")?;
        assert!(error.contains(expected), "{name} failed for wrong reason: {error}");
    }
    Ok(())
}

#[test]
fn paperless_required_environment_overrides_and_omissions_fail_closed() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let base_fixture = root.join("fixtures/scenarios/paperless-ngx-application");
    let base_manifest = read_manifest(&base_fixture)?;
    let base_source = fs::read_to_string(base_fixture.join("compose.yaml"))?;

    let missing_environment = TemporaryDirectory::new("paperless-missing-environment")?;
    fs::write(missing_environment.path().join("compose.yaml"), &base_source)?;
    let input = compose_mutation_input(Vec::new(), Vec::new());
    match import_compose_files(&input, missing_environment.path(), &base_manifest.application.name) {
        Err(error) => assert!(
            error.to_string().contains("required") || error.to_string().contains("interpol"),
            "unexpected missing-environment importer failure: {error}"
        ),
        Ok(missing) => {
            let error = validate_neutral_application(
                &base_manifest,
                missing.application().ok_or("Paperless missing-environment import")?,
                &base_manifest.semantics.required_environment_order,
                &[],
            )
            .err()
            .ok_or("required Paperless interpolation values must fail closed")?;
            assert!(
                error.contains("required environment"),
                "unexpected missing-environment failure: {error}"
            );
        }
    }

    let override_fixture = TemporaryDirectory::new("paperless-environment-override")?;
    fs::write(override_fixture.path().join("compose.yaml"), &base_source)?;
    let mut defaults = parse_scenario_environment_file(&base_fixture.join("paperless.env.example"))?;
    let task_workers = defaults
        .iter_mut()
        .find(|assignment| assignment.starts_with("PAPERLESS_TASK_WORKERS="))
        .ok_or("Paperless task-worker default")?;
    *task_workers = "PAPERLESS_TASK_WORKERS=2".into();
    fs::write(
        override_fixture.path().join("defaults.env"),
        format!("{}\n", defaults.join("\n")),
    )?;
    let input = compose_mutation_input(vec!["defaults.env".into()], vec!["PAPERLESS_TASK_WORKERS=1".into()]);
    let imported = import_compose_files(&input, override_fixture.path(), &base_manifest.application.name)?;
    validate_neutral_application(
        &base_manifest,
        imported.application().ok_or("Paperless override import")?,
        &base_manifest.semantics.required_environment_order,
        &[],
    )?;

    assert_paperless_semantic_omissions(&root)?;

    Ok(())
}

#[test]
fn paperless_private_services_have_explicit_reachable_runtime_names() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    for (scenario, services) in [
        ("paperless-ngx-application", &["broker", "db"][..]),
        (
            "paperless-ngx-document-converters",
            &["broker", "db", "gotenberg", "tika"][..],
        ),
    ] {
        let fixture = root.join("fixtures/scenarios").join(scenario);
        let compose = fs::read_to_string(fixture.join("compose.yaml"))?;
        for service in services {
            let quadlet = fs::read_to_string(fixture.join(format!("{service}.container")))?;
            assert!(
                quadlet.lines().any(|line| line == format!("ContainerName={service}")),
                "{scenario} Quadlet service {service} must preserve its DNS runtime name"
            );
            assert!(
                compose
                    .lines()
                    .any(|line| line == format!("    container_name: {service}")),
                "{scenario} Compose service {service} must independently preserve its DNS runtime name"
            );
        }
    }
    Ok(())
}

#[test]
fn paperless_captured_native_evidence_replays_through_production_acquisition() -> Result<(), Box<dyn Error>> {
    let cassette = PodmanCassette::load(
        &repository_root()
            .join("fixtures/conformance/paperless-ngx-application/paperless-ngx-6.1.0-rootless.cassette.json"),
    )?;
    assert_eq!(
        cassette.scenario_id(),
        "paperless-ngx-application-podman-6.1.0-rootless-captured"
    );
    assert_eq!(cassette.engine_version(), "6.1.0");
    assert_eq!(cassette.execution_context(), "rootless");
    assert_eq!(cassette.interaction_count(), 27);

    // Production acquisition may inspect independently discovered resources
    // concurrently; match by method/path while still requiring full coverage.
    let server = PodmanCassetteServer::start_unordered(cassette)?;
    let transport = ReadOnlyUnixTransport::new(
        UnixConnection::new(server.socket())?,
        TransportLimits::default(),
        ReadOnlyUnixTransportTimeouts::default(),
    )?;
    let mut request = DiscoveryRequest::new();
    request.add_label_root(LabelSelector::exact(
        "io.boxferry.application",
        "paperless-captured-paperless",
    )?);
    let policy = PodmanPromotionPolicy::conservative()
        .with_effective_named_volume_mounts(true)
        .with_effective_named_networks(true)
        .with_portable_effective_settings(true);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let source = runtime.block_on(acquire_podman_source(
        Identifier::new("paperless-captured-paperless")?,
        &transport,
        AcquisitionOptions::redacted(),
        &request,
        policy,
    ))?;

    // This is the request-coverage assertion: finish fails unless production
    // acquisition consumed every recorded interaction.
    server.finish()?;
    let imported = PodmanImporter::new()?.import(&source);
    let application = imported.application().ok_or("captured Paperless application")?;
    assert_eq!(application.services().len(), 5);
    assert_eq!(application.volumes().len(), 6);
    assert_eq!(application.networks().len(), 2);
    Ok(())
}

#[test]
fn immich_captured_native_evidence_replays_through_production_acquisition() -> Result<(), Box<dyn Error>> {
    let cassette = PodmanCassette::load(
        &repository_root()
            .join("fixtures/conformance/immich-application/immich-application-6.1.0-rootless.cassette.json"),
    )?;
    assert_eq!(
        cassette.scenario_id(),
        "immich-application-podman-6.1.0-rootless-captured"
    );
    assert_eq!(cassette.engine_version(), "6.1.0");
    assert_eq!(cassette.execution_context(), "rootless");
    assert_eq!(cassette.interaction_count(), 23);

    let server = PodmanCassetteServer::start_unordered(cassette)?;
    let transport = ReadOnlyUnixTransport::new(
        UnixConnection::new(server.socket())?,
        TransportLimits::default(),
        ReadOnlyUnixTransportTimeouts::default(),
    )?;
    let mut request = DiscoveryRequest::new();
    request.add_label_root(LabelSelector::exact(
        "io.boxferry.application",
        "immich-captured-immich",
    )?);
    let policy = PodmanPromotionPolicy::conservative()
        .with_effective_named_volume_mounts(true)
        .with_effective_named_networks(true)
        .with_portable_effective_settings(true);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let source = runtime.block_on(acquire_podman_source(
        Identifier::new("immich-captured-immich")?,
        &transport,
        AcquisitionOptions::redacted(),
        &request,
        policy,
    ))?;

    server.finish()?;
    let imported = PodmanImporter::new()?.import(&source);
    let application = imported.application().ok_or("captured Immich application")?;
    assert_eq!(application.services().len(), 4);
    assert_eq!(application.volumes().len(), 4);
    assert_eq!(application.networks().len(), 2);
    Ok(())
}

fn compose_mutation_input(environment_files: Vec<String>, environment: Vec<String>) -> NativeInput {
    NativeInput {
        id: "compose-mutation".into(),
        importer: "compose".into(),
        files: vec!["compose.yaml".into()],
        format_version: "compose-specification".into(),
        outcome: Outcome::MigrationSuccess,
        reason: None,
        interpolate: true,
        environment,
        environment_files,
        all_profiles: false,
        import_diagnostics: None,
        import_diagnostics_file: None,
        podman: None,
    }
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
fn scenario_network_membership_counterfactual_detects_extra_attachment() -> Result<(), Box<dyn Error>> {
    let fixture = repository_root().join("fixtures/scenarios/supabase-application");
    let manifest = read_manifest(&fixture)?;
    let source = fs::read_to_string(fixture.join("compose.yaml"))?;
    let mutation = source.replacen(
        "      GOTRUE_DB_DATABASE_URL: postgres://supabase_auth_admin:boxferry-public-supabase-db-password@db:5432/postgres\n    networks: [backend]",
        "      GOTRUE_DB_DATABASE_URL: postgres://supabase_auth_admin:boxferry-public-supabase-db-password@db:5432/postgres\n    networks: [backend, edge]",
        1,
    );
    assert_ne!(source, mutation, "counterfactual must attach Auth to edge");
    let imported = import_compose(&mutation, &manifest.application.name)?;

    let error = validate_neutral_application(
        &manifest,
        imported.application().ok_or("Supabase application")?,
        &[],
        &[],
    )
    .err()
    .ok_or("counterfactual extra network attachment must fail")?;
    assert!(
        error.contains("network-memberships:auth:expected=backend:actual=backend,edge"),
        "extra network attachment failed for wrong reason: {error}"
    );
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

    let output = Command::new("bash")
        .args([
            "-c",
            concat!(
                "source \"$1\"\n",
                "podman_socket() { printf '<%s>\\n' \"$@\"; }\n",
                "scenario_podman_socket /run/podman/podman.sock inspect ",
                "--format '{{.State.Status}}' demo\n",
            ),
            "validator-test",
        ])
        .arg(root.join("scripts/lib/scenario-validators.sh"))
        .output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        concat!(
            "</run/podman/podman.sock>\n",
            "<validate runtime scenario via Podman inspect>\n",
            "<inspect>\n",
            "<--format>\n",
            "<{{.State.Status}}>\n",
            "<demo>\n",
        ),
        "scenario adapter must preserve the human action and complete Podman command",
    );
    assert!(output.stderr.is_empty());
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
    if interpolation.is_some() {
        source = source.with_interpolation_applied();
    }
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
    let include_values = manifest
        .native_inputs
        .iter()
        .find(|input| input.id == input_id)
        .and_then(|input| input.podman.as_ref())
        .ok_or("missing Podman input metadata")?
        .include_environment_values;
    import_podman_cassette_with_environment_values(manifest, input_id, cassette, include_values)
}

fn import_podman_cassette_with_environment_values(
    manifest: &ScenarioManifest,
    input_id: &str,
    cassette: PodmanCassette,
    include_values: bool,
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
    let acquisition = if include_values {
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
    let report_directory = TemporaryDirectory::new("scenario-full-report")?;
    let report_path = report_directory.path().join("report.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command.args(["convert", &input.importer, &route.exporter]);
    if route.includes_environment_values() {
        command.args(["--environment-values", "include"]);
    }
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
                .args(["--application-name", &podman.application])
                .args(["--podman-import-policy", "conservative"]);
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
    if route.exporter == "docker" {
        command.args([
            "--docker-target",
            route.docker_target.as_deref().ok_or("missing Docker preset")?,
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
#[test]
fn live_resource_validators_accept_renderer_declaration_forms() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let output = TemporaryDirectory::new("compose-resource-validator")?;
    let block = output.path().join("block");
    let compact = output.path().join("compact");
    let missing = output.path().join("missing");
    let quadlet = output.path().join("quadlet");
    let canonical_input = output.path().join("canonical-input.yaml");
    let canonical_output = output.path().join("canonical-output.yaml");
    fs::create_dir_all(&block)?;
    fs::create_dir_all(&compact)?;
    fs::create_dir_all(&missing)?;
    fs::create_dir_all(&quadlet)?;

    fs::write(
        block.join("compose.yaml"),
        "---\nnetworks:\n  backend:\n    name: external-backend\nvolumes:\n  data:\n    name: external-data\n",
    )?;
    fs::write(
        compact.join("compose.yaml"),
        "---\nnetworks:\n  backend: {}\nvolumes:\n  data: {}\n",
    )?;
    fs::write(
        missing.join("compose.yaml"),
        "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      backend: {}\n",
    )?;
    fs::write(
        quadlet.join("live-large-api.container"),
        "[Container]\nVolume=live-large-cache.volume:/cache:ro\nNetwork=live-large-private.network\nNetwork=live-large-edge.network\n",
    )?;
    fs::write(
        &canonical_input,
        "---\nservices:\n  app:\n    image: example.invalid/app:1\nnetworks:\n  z-edge: {}\n  a-backend: {}\nvolumes:\n  z-data: {}\n  a-cache: {}\n",
    )?;

    let result = Command::new("bash")
        .args([
            "-c",
            r#"
set -Eeuo pipefail
source "$1"
assert_resource_member compose "$2/block" network backend
assert_resource_member compose "$2/block" volume data
assert_resource_member compose "$2/compact" network backend
assert_resource_member compose "$2/compact" volume data
if assert_resource_member compose "$2/missing" network backend 2> "$2/missing.stderr"; then
  printf 'Service attachment unexpectedly satisfied top-level membership.\n' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Expected network resource backend is absent from compose output' "$2/missing.stderr"
if assert_resource_absent compose "$2/compact" volume data 2> "$2/present.stderr"; then
  printf 'Compact resource unexpectedly satisfied absence.\n' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Selection unexpectedly retained volume data' "$2/present.stderr"
current_prefix=live
current_podman_major=5
current_podman_rootless=false
assert_network_boundary_semantics quadlet "$2/quadlet"
"#,
            "scenario-validator",
        ])
        .arg(root.join("scripts/lib/scenario-validators.sh"))
        .arg(output.path())
        .output()?;
    assert!(
        result.status.success(),
        "scenario validator regression failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let canonical_result = Command::new("python3")
        .arg(root.join("fixtures/conformance/podman-live/canonicalize_compose.py"))
        .arg(&canonical_input)
        .arg(&canonical_output)
        .output()?;
    assert!(
        canonical_result.status.success(),
        "compact Compose canonicalization failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&canonical_result.stdout),
        String::from_utf8_lossy(&canonical_result.stderr)
    );
    let canonical = fs::read_to_string(canonical_output)?;
    let a_backend = canonical.find("  a-backend:").ok_or("missing a-backend")?;
    let z_edge = canonical.find("  z-edge:").ok_or("missing z-edge")?;
    let a_cache = canonical.find("  a-cache:").ok_or("missing a-cache")?;
    let z_data = canonical.find("  z-data:").ok_or("missing z-data")?;
    assert!(a_backend < z_edge);
    assert!(a_cache < z_data);
    Ok(())
}

#[test]
fn quadlet_network_membership_is_exact_and_confined() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let output = TemporaryDirectory::new("quadlet-network-membership")?;
    for name in [
        "standalone",
        "attachment",
        "dotted",
        "missing",
        "lookalike",
        "traversal",
    ] {
        fs::create_dir_all(output.path().join(name))?;
    }
    fs::write(output.path().join("standalone/managed.network"), "[Network]\n")?;
    fs::write(
        output.path().join("attachment/consumer.container"),
        "[Container]\nNetwork=external.network\n",
    )?;
    fs::write(
        output.path().join("dotted/consumer.container"),
        "[Container]\nNetwork=prod.net.network\n",
    )?;
    fs::write(
        output.path().join("lookalike/consumer.container"),
        "[Container]\nNetwork=prodXnet.network\n",
    )?;
    fs::write(output.path().join("outside.network"), "[Network]\n")?;

    let result = Command::new("bash")
        .args([
            "-c",
            r#"set -Eeuo pipefail
source "$1"
assert_resource_member quadlet "$2/standalone" network managed
assert_resource_member quadlet "$2/attachment" network external
assert_resource_member quadlet "$2/dotted" network prod.net
if assert_resource_member quadlet "$2/missing" network missing 2> "$2/missing.stderr"; then
  printf 'Missing Quadlet network unexpectedly satisfied membership.\n' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Expected network resource missing is absent from quadlet output' "$2/missing.stderr"
if assert_resource_member quadlet "$2/lookalike" network prod.net 2> "$2/lookalike.stderr"; then
  printf 'Regex-lookalike Quadlet network unexpectedly satisfied exact membership.\n' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Expected network resource prod.net is absent from quadlet output' "$2/lookalike.stderr"
if assert_resource_member quadlet "$2/traversal" network ../outside 2> "$2/traversal.stderr"; then
  printf 'Out-of-directory Quadlet network unexpectedly satisfied membership.\n' >&2
  exit 1
fi
grep --fixed-strings --quiet \
  'Expected network resource ../outside is absent from quadlet output' "$2/traversal.stderr"
"#,
            "scenario-validator",
        ])
        .arg(root.join("scripts/lib/scenario-validators.sh"))
        .arg(output.path())
        .output()?;
    assert!(
        result.status.success(),
        "Quadlet network membership regression failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn default_network_evidence_dispatches_all_inventory_paths_under_set_u() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let directory = TemporaryDirectory::new("legacy-default-network")?;
    for (name, report, version, rootless, present, succeeds, expected_gate) in default_network_evidence_cases() {
        let report_path = directory.path().join(format!("{name}.json"));
        let feature_gates = directory.path().join(format!("{name}.feature-gates.txt"));
        fs::write(&report_path, report)?;
        let output = Command::new("bash")
            .args([
                "-c",
                "set -u\nsource \"$1\"\nassert_default_podman_network_evidence \"$2\" \"$3\" \"$4\" \"$5\" \"$6\"",
                "legacy-default-network-test",
            ])
            .arg(root.join("scripts/lib/scenario-validators.sh"))
            .arg(&report_path)
            .arg(&feature_gates)
            .arg(version)
            .arg(rootless)
            .arg(present)
            .output()?;
        assert_eq!(
            output.status.success(),
            succeeds,
            "legacy default-network evidence {name}: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        if succeeds {
            assert!(
                fs::read_to_string(feature_gates)?.contains(expected_gate),
                "feature-gate evidence {name}"
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_rootless_344_default_network_evidence_names_rootless_mode() -> Result<(), Box<dyn Error>> {
    let root = repository_root();
    let directory = TemporaryDirectory::new("legacy-default-network-rootless-stderr")?;
    let report_path = directory.path().join("malformed-rootless-344.json");
    let feature_gates = directory.path().join("malformed-rootless-344.feature-gates.txt");
    fs::write(&report_path, r#"{"status":"success","diagnostics":[]}"#)?;

    let output = Command::new("bash")
        .args([
            "-c",
            "set -u\nsource \"$1\"\nassert_default_podman_network_evidence \"$2\" \"$3\" 3.4.4 true true",
            "legacy-default-network-rootless-stderr-test",
        ])
        .arg(root.join("scripts/lib/scenario-validators.sh"))
        .arg(&report_path)
        .arg(&feature_gates)
        .output()?;

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Podman 3.4.4 rootless default CNI network"),
        "malformed 3.4.4 rootless evidence must identify its mode: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

type DefaultNetworkEvidenceCase = (
    &'static str,
    String,
    &'static str,
    &'static str,
    &'static str,
    bool,
    &'static str,
);

fn default_network_evidence_cases() -> Vec<DefaultNetworkEvidenceCase> {
    let legacy = r#"{
  "status": "success",
  "diagnostics": [{
    "code": "BFP0002",
    "source_code": "PLN0023",
    "fields": [
      {"name": "subject", "value": "network:podman"},
      {"name": "reason", "value": "PodmanLens found native response fields without typed portable mappings; path descriptors were retained without values"},
      {"name": "decision", "value": "omitted"},
      {"name": "source_engine", "value": "3.0.1"},
      {"name": "source_api", "value": "3.0.0"},
      {"name": "native_path", "value": "$.CniConfig"},
      {"name": "native_value_policy", "value": "field paths retained; native values not retained"}
    ]
  }]
}"#;
    let additional_legacy_cases = additional_legacy_default_network_evidence_cases(legacy);
    let wrong_api = legacy.replacen(
        "\"source_api\", \"value\": \"3.0.0\"",
        "\"source_api\", \"value\": \"3.0.1\"",
        1,
    );
    let wrong_source_code = legacy.replacen("\"PLN0023\"", "\"PLN0049\"", 1);
    let quadlet_decision = legacy.replace(
        "  }]\n}",
        "  }, {\"code\": \"BFQ0007\", \"fields\": [{\"name\": \"subject\", \"value\": \"networks.podman\"}]}]\n}",
    );
    let typed = r#"{
  "status": "success",
  "diagnostics": [{"code":"BFQ0007","fields":[{"name":"subject","value":"networks.podman"},{"name":"reason","value":"network lifecycle ownership is uncertain; no managed .network unit was generated"}]}]
}"#;
    vec![
        (
            "legacy",
            legacy.to_owned(),
            "3.0.1",
            "false",
            "true",
            true,
            "Podman 3.0.1 rootful default CNI network remains omitted",
        ),
        ("wrong-api", wrong_api, "3.0.1", "false", "true", false, ""),
        (
            "wrong-source-code",
            wrong_source_code,
            "3.0.1",
            "false",
            "true",
            false,
            "",
        ),
        (
            "quadlet-decision",
            quadlet_decision,
            "3.0.1",
            "false",
            "true",
            false,
            "",
        ),
        (
            "typed",
            typed.to_owned(),
            "6.1.0",
            "false",
            "true",
            true,
            "all-resource Quadlet export diagnoses the uncertain default Podman network",
        ),
        (
            "absent",
            r#"{"status":"success","diagnostics":[]}"#.to_owned(),
            "3.0.1",
            "true",
            "false",
            true,
            "default Podman network absent from live inventory",
        ),
    ]
    .into_iter()
    .chain(additional_legacy_cases)
    .collect()
}

fn additional_legacy_default_network_evidence_cases(legacy_301: &str) -> Vec<DefaultNetworkEvidenceCase> {
    let legacy_344 = legacy_301
        .replace(
            "\"source_engine\", \"value\": \"3.0.1\"",
            "\"source_engine\", \"value\": \"3.4.4\"",
        )
        .replace(
            "\"source_api\", \"value\": \"3.0.0\"",
            "\"source_api\", \"value\": \"3.4.4\"",
        );
    vec![
        (
            "legacy-344",
            legacy_344.clone(),
            "3.4.4",
            "false",
            "true",
            true,
            "Podman 3.4.4 rootful default CNI network remains omitted",
        ),
        (
            "legacy-344-rootless",
            legacy_344.clone(),
            "3.4.4",
            "true",
            "true",
            true,
            "Podman 3.4.4 rootless default CNI network remains omitted",
        ),
        (
            "legacy-344-crossed-api",
            legacy_301.to_owned(),
            "3.4.4",
            "false",
            "true",
            false,
            "",
        ),
        (
            "legacy-301-crossed-api",
            legacy_344,
            "3.0.1",
            "false",
            "true",
            false,
            "",
        ),
        (
            "unreviewed-legacy-version",
            legacy_301.to_owned(),
            "3.2.0",
            "false",
            "true",
            false,
            "",
        ),
        (
            "legacy-301-rootless-not-reviewed",
            legacy_301.to_owned(),
            "3.0.1",
            "true",
            "true",
            false,
            "",
        ),
    ]
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        #[cfg(not(unix))]
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

#[test]
fn physical_quadlet_service_alias_reimport_refuses_sensitive_text_but_keeps_alias_free_positive()
-> Result<(), Box<dyn Error>> {
    let compose_target = boxferry::TargetProfile::new(
        boxferry::COMPOSE_SPECIFICATION_TARGET,
        boxferry::COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(boxferry::COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    let quadlet_target = boxferry::TargetProfile::new(
        "podman",
        boxferry::PlatformVersion::new(5, 4, 0),
        Some(boxferry::PlatformVersion::new(6, 1, 2)),
    )?;
    for aliased in [false, true] {
        let text = if aliased {
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n    networks:\n      backend:\n        aliases: [protected-alias-canary-never-print, second-alias]\nnetworks:\n  backend: {}\n"
        } else {
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n    networks: [backend]\nnetworks:\n  backend: {}\n"
        };
        let imported = import_compose(text, "physical-alias-chain")?;
        let direct = convert_imported(
            imported.clone(),
            &ComposeExporter::new()?,
            &compose_target,
            boxferry::LossPolicy::AllowPartial,
        )?;
        assert!(!direct.is_blocked(), "public aliases keep direct Compose generation");
        let generated = convert_imported(
            imported,
            &QuadletExporter::new()?,
            &quadlet_target,
            boxferry::LossPolicy::AllowPartial,
        )?;
        assert!(!generated.is_blocked());
        let directory = TemporaryDirectory::new("physical-alias-reimport")?;
        let files = generated.output().ok_or("Quadlet output missing")?;
        let mut inputs = Vec::new();
        for (index, file) in files.files().iter().enumerate() {
            let path = directory.path().join(file.name().as_str());
            fs::write(&path, file.text())?;
            let physical = fs::read_to_string(path)?;
            if file.name().as_str().ends_with(".container") {
                assert_eq!(
                    physical.contains("NetworkAlias=protected-alias-canary-never-print"),
                    aliased
                );
            }
            inputs.push(QuadletDocumentInput::new(
                file.name().as_str(),
                boxferry::quadlet::quadlet_lens::source::SourceId::new(u32::try_from(index + 1)?),
                physical,
            ));
        }
        let native = QuadletSource::parse(Identifier::new("physical-alias-chain")?, inputs)?;
        let reimport = QuadletImporter::new()?.import(native.source());
        let application = reimport.application().ok_or("Quadlet application missing")?;
        let aliases = application
            .services()
            .iter()
            .flat_map(|service| service.value().networks())
            .flat_map(|network| network.value().alias_sensitivities())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(aliases, if aliased { vec![true, true] } else { vec![] });
        for policy in [
            boxferry::LossPolicy::ExactOnly,
            boxferry::LossPolicy::AllowApproximate,
            boxferry::LossPolicy::AllowPartial,
        ] {
            let result = convert_imported(reimport.clone(), &ComposeExporter::new()?, &compose_target, policy)?;
            if aliased {
                assert!(result.is_blocked() && result.output().is_none());
                let refusals = result
                    .outcomes()
                    .iter()
                    .filter(|outcome| outcome.subject().contains(".aliases["))
                    .collect::<Vec<_>>();
                assert_eq!(refusals.len(), 2);
                for (index, outcome) in refusals.iter().enumerate() {
                    assert_eq!(
                        outcome.subject(),
                        format!("services.web.networks.backend.aliases[{index}]")
                    );
                    assert_eq!(outcome.kind(), ConversionKind::Unsupported);
                    assert!(!outcome.origins().is_empty());
                    assert_eq!(
                        outcome.diagnostic().map(boxferry::DiagnosticCode::as_str),
                        Some("BFC0007")
                    );
                }
                assert!(!format!("{:?}", result.diagnostics()).contains("protected-alias-canary-never-print"));
            } else if policy == boxferry::LossPolicy::AllowPartial {
                assert!(
                    !result.is_blocked() && result.output().is_some(),
                    "zero-alias physical reimport retains the positive route"
                );
            }
        }
    }
    Ok(())
}

// Independently transcribed from the pinned public source; these assertions establish why the
// obsolete root-extension losses disappear without weakening any remaining route/runtime gap.
#[test]
fn pinned_vendor_metadata_retains_superset_payloads_and_appwrite_compose_reimport() -> Result<(), Box<dyn Error>> {
    let expected = [
        (
            "apache-superset",
            vec![
                ("x-superset-user", serde_json::json!("root")),
                (
                    "x-superset-volumes",
                    serde_json::json!([
                        "./docker:/app/docker",
                        "./superset:/app/superset",
                        "./superset-core:/app/superset-core",
                        "./superset-frontend:/app/superset-frontend",
                        "superset_home:/app/superset_home",
                        "./tests:/app/tests",
                        "superset_data:/app/data",
                        "./local_extensions:/app/local_extensions"
                    ]),
                ),
                (
                    "x-common-build",
                    serde_json::json!({
                        "context": ".", "target": "${SUPERSET_BUILD_TARGET:-dev}",
                        "cache_from": ["apache/superset-cache:3.11-slim-trixie"],
                        "args": {"DEV_MODE": "true", "INCLUDE_CHROMIUM": "${INCLUDE_CHROMIUM:-false}",
                            "INCLUDE_FIREFOX": "${INCLUDE_FIREFOX:-false}", "BUILD_TRANSLATIONS": "${BUILD_TRANSLATIONS:-false}"}
                    }),
                ),
            ],
        ),
        (
            "appwrite",
            vec![(
                "x-logging",
                serde_json::json!({"logging": {
                    "driver": "json-file", "options": {"max-file": "5", "max-size": "10m"}
                }}),
            )],
        ),
    ];
    for (project, metadata) in expected {
        let fixture = repository_root().join(format!("fixtures/scenarios/real-world-compose-{project}"));
        let manifest = read_manifest(&fixture)?;
        let input = manifest
            .native_inputs
            .iter()
            .find(|input| input.importer == "compose")
            .ok_or("Compose input")?;
        let imported = import_native_input(&manifest, input, &fixture)?;
        let mut application = imported.application().ok_or("application")?.clone();
        assert_public_metadata(&application, &metadata)?;
        if project == "appwrite" {
            let quadlet = manifest
                .evidence
                .iter()
                .find(|route| route.exporter == "quadlet")
                .ok_or("Quadlet route")?;
            assert_eq!(quadlet.outcome, Outcome::ExpectedRejection);
            assert_appwrite_metadata_has_no_quadlet_carrier(&application)?;
            application.withhold_literal_environment_values();
            let version = boxferry::PlatformVersion::new(1, 0, 0);
            let target = boxferry::TargetProfile::new("compose-specification", version, Some(version))?;
            let plan = ExportAdapter::plan(&ComposeExporter::new()?, &application, &target)?;
            let authorized = plan.authorize(boxferry::LossPolicy::AllowPartial);
            let text = authorized.output().ok_or("reviewed partial Compose artifact")?.text();
            let loaded = LoadedProject::load([DocumentInput::new(
                ComposeSourceId::new(1),
                DocumentOrigin::new("generated-appwrite.yaml", &fixture),
                text,
            )])?;
            let merged = merge_project(&loaded, None);
            let source = ComposeSource::new(
                merged.project().ok_or("merged generated source")?.clone(),
                Identifier::new("appwrite")?,
            )?;
            let reimported = ComposeImporter::new()?.import(&source);
            assert_public_metadata(reimported.application().ok_or("reimported application")?, &metadata)?;
        }
    }
    Ok(())
}

fn assert_public_metadata(
    application: &boxferry::Application,
    expected: &[(&str, serde_json::Value)],
) -> Result<(), Box<dyn Error>> {
    assert_eq!(application.vendor_metadata().len(), expected.len());
    for (metadata, (namespace, value)) in application.vendor_metadata().iter().zip(expected) {
        assert!(matches!(metadata.scope(), boxferry::MetadataScope::Application));
        assert_eq!(metadata.namespace().value().expose(), *namespace);
        assert!(!metadata.namespace().origins().is_empty());
        assert!(!metadata.payload().origins().is_empty());
        assert_eq!(public_metadata_value(metadata.payload().value())?, *value);
    }
    Ok(())
}

fn public_metadata_value(node: &boxferry::MetadataNode) -> Result<serde_json::Value, Box<dyn Error>> {
    match node.value() {
        boxferry::MetadataValue::String(value) => Ok(serde_json::json!(value.expose())),
        boxferry::MetadataValue::Sequence(values) => Ok(serde_json::Value::Array(
            values
                .iter()
                .map(|value| public_metadata_value(value.value()))
                .collect::<Result<_, _>>()?,
        )),
        boxferry::MetadataValue::Mapping(entries) => Ok(serde_json::Value::Object(
            entries
                .iter()
                .map(|entry| {
                    Ok((
                        entry.key().value().expose().to_owned(),
                        public_metadata_value(entry.value().value())?,
                    ))
                })
                .collect::<Result<_, Box<dyn Error>>>()?,
        )),
        _ => Err("pinned extension payload changed its independently expected scalar type".into()),
    }
}

#[test]
fn pinned_authored_environment_expressions_use_the_specific_quadlet_source_rule() -> Result<(), Box<dyn Error>> {
    // Each expression is independently transcribed from its immutable plain/double-quoted field.
    // The other Host/Required/literal environment losses keep their generic target rule.
    for (project, service_name, key, expression) in [
        (
            "apache-superset",
            "superset-node",
            "SCARF_ANALYTICS",
            "${SCARF_ANALYTICS:-}",
        ),
        ("immich", "database", "POSTGRES_PASSWORD", "${DB_PASSWORD}"),
        (
            "librechat",
            "admin-panel",
            "SESSION_SECRET",
            "${ADMIN_PANEL_SESSION_SECRET}",
        ),
        ("plausible", "plausible", "SECRET_KEY_BASE", "${SECRET_KEY_BASE}"),
        ("supabase", "auth", "API_EXTERNAL_URL", "${API_EXTERNAL_URL}"),
    ] {
        let fixture = repository_root().join(format!("fixtures/scenarios/real-world-compose-{project}"));
        let manifest = read_manifest(&fixture)?;
        let input = manifest
            .native_inputs
            .iter()
            .find(|input| input.importer == "compose")
            .ok_or("Compose input")?;
        let imported = import_native_input(&manifest, input, &fixture)?;
        let application = imported.application().ok_or("application")?;
        let service = application
            .services()
            .iter()
            .find(|service| service.value().name().as_str() == service_name)
            .ok_or("required service")?
            .value();
        let entry = service
            .environment()
            .iter()
            .find(|entry| entry.value().name().as_str() == key)
            .ok_or("required environment field")?;
        assert!(
            matches!(entry.value().value(), boxferry::EnvironmentValue::UnresolvedSource { source_format, expression: value } if source_format.as_str() == "compose" && value.expose() == expression)
        );
        let target = boxferry::TargetProfile::new(
            "podman",
            boxferry::PlatformVersion::new(5, 4, 0),
            Some(boxferry::PlatformVersion::new(6, 1, 0)),
        )?;
        let plan = ExportAdapter::plan(&QuadletExporter::new()?, application, &target)?;
        let subject = format!("services.{service_name}.environment.{key}");
        let outcomes = plan
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect::<Vec<_>>();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].kind(), ConversionKind::Unsupported);
        assert_eq!(
            outcomes[0].diagnostic().map(boxferry::DiagnosticCode::as_str),
            Some("BFQ0014")
        );
        let mut withheld = application.clone();
        withheld.withhold_literal_environment_values();
        let withheld_plan = ExportAdapter::plan(&QuadletExporter::new()?, &withheld, &target)?;
        let withheld_outcomes = withheld_plan
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect::<Vec<_>>();
        assert_eq!(withheld_outcomes.len(), 1);
        assert_eq!(withheld_outcomes[0].kind(), ConversionKind::Unsupported);
        assert_eq!(
            withheld_outcomes[0].diagnostic().map(boxferry::DiagnosticCode::as_str),
            Some("BFQ0003")
        );
    }
    Ok(())
}

fn assert_appwrite_metadata_has_no_quadlet_carrier(application: &boxferry::Application) -> Result<(), Box<dyn Error>> {
    let target = boxferry::TargetProfile::new(
        "podman",
        boxferry::PlatformVersion::new(5, 4, 0),
        Some(boxferry::PlatformVersion::new(6, 1, 0)),
    )?;
    let plan = ExportAdapter::plan(&QuadletExporter::new()?, application, &target)?;
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid
                && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFQ0012"))
    );
    assert!(plan.diagnostics().iter().any(|diagnostic| diagnostic.code().as_str() == "BFQ0003"
        && diagnostic.fields().iter().any(|field| field.name() == "subject" && field.value().expose() == "application.vendor_metadata[0]")
        && diagnostic.fields().iter().any(|field| field.name() == "reason" && field.value().expose() == "metadata has no generated owning unit; retain the source metadata separately or generate its owning service unit")));
    assert!(plan.authorize(boxferry::LossPolicy::AllowPartial).output().is_none());
    Ok(())
}
