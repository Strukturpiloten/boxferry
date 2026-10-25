//! Offline consumer checks for ADR 0010's six authored fixture volume mappings.
//!
//! Only an explicitly named VOLUME-ONLY neutral projection reaches the exporter. These tests
//! neither validate the fixtures' other fields nor execute requests, move data, or establish
//! application/runtime acceptance. Positive rendering requires the actual reviewed exact
//! `DockerLens` candidate; a denied capability or profile fallback cannot satisfy these checks.

#![cfg(all(feature = "compose", feature = "docker"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::PathBuf,
};

#[path = "support/docker_volume_fixtures.rs"]
mod fixtures;

use boxferry::compose::compose_lens::{interpolation::MapEnvironment, loader::LoadedProject};
use boxferry::docker::docker_lens::version::{
    CapabilityEvidenceKey, DaemonMode, EngineBuild, TargetCapabilityCatalog, TargetProfile as NativeTargetProfile,
};
use boxferry::{
    Application, ConversionKind, ConversionPlan, DockerExporter, DockerOutput, ExportAdapter, Identifier, LossPolicy,
    MetadataLabel, PlatformVersion, ProtectedString, Provenance, ProvenanceKind, ResourceOwnership, Sourced,
    TargetProfile, Volume,
};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const RUN: &str = "volume-label-consumer-offline";
const PREFIX: &str = "offline";
const PROTECTED_LABEL: &str = "volume-label-protected-value-must-not-leak";

struct FixtureExpectation {
    application: &'static str,
    owner: &'static str,
    // Independently authored expectations, never derived from a fixture or rendered artifact.
    volumes: &'static [(&'static str, &'static str)],
}

const FIXTURES: &[FixtureExpectation] = &[
    FixtureExpectation {
        application: "forgejo",
        owner: "offline-forgejo",
        volumes: &[("db", "offline-forge-db"), ("data", "offline-forge-data")],
    },
    FixtureExpectation {
        application: "nextcloud",
        owner: "offline-cloud",
        volumes: &[
            ("database", "offline-cloud-database"),
            ("redis", "offline-cloud-redis"),
            ("nextcloud", "offline-cloud-nextcloud"),
        ],
    },
    FixtureExpectation {
        application: "paperless-ngx",
        owner: "offline-paperless",
        volumes: &[
            ("data", "offline-paper-data"),
            ("media", "offline-paper-media"),
            ("consume", "offline-paper-consume"),
            ("export", "offline-paper-export"),
            ("pgdata", "offline-paper-pgdata"),
            ("redisdata", "offline-paper-redisdata"),
        ],
    },
    FixtureExpectation {
        application: "immich",
        owner: "offline-immich",
        volumes: &[
            ("library", "offline-immich-library"),
            ("model-cache", "offline-immich-model-cache"),
            ("pgdata", "offline-immich-pgdata"),
            ("redisdata", "offline-immich-redisdata"),
        ],
    },
    FixtureExpectation {
        application: "observability",
        owner: "offline-observability",
        volumes: &[
            ("alloy-data", "offline-observability-alloy-data"),
            ("grafana-data", "offline-observability-grafana-data"),
            ("loki-data", "offline-observability-loki-data"),
            ("prometheus-data", "offline-observability-prometheus-data"),
            ("telemetry-logs", "offline-observability-telemetry-logs"),
        ],
    },
    FixtureExpectation {
        application: "supabase",
        owner: "offline-supabase",
        volumes: &[
            ("deno-cache", "offline-supabase-deno-cache"),
            ("pgdata", "offline-supabase-pgdata"),
            ("storage", "offline-supabase-storage"),
        ],
    },
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_path(fixture: &FixtureExpectation) -> String {
    format!("fixtures/conformance/{}-application/compose.yaml", fixture.application)
}

fn synthetic_environment(include_run: bool) -> MapEnvironment {
    fixtures::synthetic_environment(PREFIX, include_run.then_some(RUN))
}

fn loaded_fixture(fixture: &FixtureExpectation) -> TestResult<(LoadedProject, usize)> {
    let embedded = fixtures::fixture(fixture.application)?;
    let current = fs::read(repository_root().join(fixture_path(fixture)))?;
    assert_eq!(
        fixtures::sha256(&current),
        fixtures::sha256(embedded.template.as_bytes())
    );
    assert_eq!(embedded.volume_count, fixture.volumes.len());
    fixtures::loaded_fixture(embedded)
}

fn import_fixture(fixture: &FixtureExpectation) -> TestResult<(Application, usize)> {
    let _ = loaded_fixture(fixture)?;
    fixtures::import_fixture(fixtures::fixture(fixture.application)?, PREFIX, RUN)
}

fn assert_source_origins(origins: &[Provenance], fixture: &FixtureExpectation, source_length: usize) -> TestResult {
    assert!(!origins.is_empty(), "authored volume evidence lost its source");
    for origin in origins {
        assert_eq!(origin.kind(), ProvenanceKind::SourceDocument);
        assert_eq!(origin.source_id().as_str(), fixture_path(fixture));
        let span = origin.span().ok_or("authored volume evidence needs a source span")?;
        assert!(span.start() < span.end() && span.end() <= source_length);
    }
    Ok(())
}

fn assert_neutral_volumes(application: &Application, fixture: &FixtureExpectation, source_length: usize) -> TestResult {
    assert_eq!(application.volumes().len(), fixture.volumes.len());
    let expected: BTreeMap<_, _> = fixture.volumes.iter().copied().collect();
    let actual: BTreeMap<_, _> = application
        .volumes()
        .iter()
        .map(|volume| (volume.value().name().as_str(), volume))
        .collect();
    assert_eq!(actual.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>());
    for (key, runtime_name) in &expected {
        let sourced = actual[key];
        assert_source_origins(sourced.origins(), fixture, source_length)?;
        let volume = sourced.value();
        assert_eq!(volume.ownership(), ResourceOwnership::Application);
        let runtime = volume.runtime_name().ok_or("authored runtime name missing")?;
        assert!(!runtime.value().is_sensitive());
        assert_eq!(runtime.value().expose(), *runtime_name);
        assert_source_origins(runtime.origins(), fixture, source_length)?;
        let labels = volume.labels().ok_or("authored volume labels missing")?;
        assert_eq!(labels.len(), 2);
        let actual_labels: BTreeMap<_, _> = labels
            .iter()
            .map(|label| (label.value().name().as_str(), label.value().value().expose()))
            .collect();
        assert_eq!(
            actual_labels,
            BTreeMap::from([
                ("io.boxferry.application", fixture.owner),
                ("io.boxferry.live-run", RUN),
            ])
        );
        assert_source_origins(volume.labels_origins(), fixture, source_length)?;
        for label in labels {
            assert!(!label.value().value().is_sensitive());
            assert_source_origins(label.origins(), fixture, source_length)?;
        }
    }
    Ok(())
}

fn volume_only_projection(application: &Application, fixture: &FixtureExpectation) -> TestResult<Application> {
    let volume_only = fixtures::volume_only_projection(application, fixture.application)?;
    assert!(volume_only.services().is_empty());
    assert!(volume_only.networks().is_empty());
    assert_eq!(volume_only.volumes(), application.volumes());
    Ok(volume_only)
}

fn reviewed_profiles() -> Vec<NativeTargetProfile> {
    TargetCapabilityCatalog::reviewed().profiles().cloned().collect()
}

fn target(profile: &NativeTargetProfile) -> TestResult<TargetProfile> {
    let version = if matches!(profile.identity().build(), EngineBuild::Upstream) {
        PlatformVersion::new(29, 8, 1)
    } else {
        PlatformVersion::new(20, 10, 5)
    };
    Ok(TargetProfile::new("docker", version, Some(version))?)
}

fn assert_field_set(value: &Value, expected: &[&str]) -> TestResult {
    let object = value.as_object().ok_or("closed artifact object missing")?;
    let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
    assert_eq!(actual, expected.iter().copied().collect::<BTreeSet<_>>());
    Ok(())
}

fn expected_selected_context(profile: &NativeTargetProfile) -> TestResult<Value> {
    // Schema keys are independent literals. Every value comes from the caller-selected profile,
    // not output.native_profile(), the serialized artifact, or a native context formatter.
    let identity = profile.identity();
    let build = match identity.build() {
        EngineBuild::Upstream => json!({"kind": "upstream"}),
        EngineBuild::DebianPackage(revision) => json!({"kind": "debian_package", "revision": revision.as_str()}),
    };
    let mode = match identity.mode() {
        DaemonMode::Rootful => "rootful",
        DaemonMode::Rootless => "rootless",
        DaemonMode::Unknown => return Err("selected reviewed profile must have an exact daemon mode".into()),
    };
    let advertised = identity.advertised_api_version();
    let acquisition = identity.acquisition_api_version();
    let rendering = identity.rendering_api_version();
    let hex = b"0123456789abcdef";
    let evidence: String = profile
        .evidence_key()
        .as_sha256_bytes()
        .iter()
        .flat_map(|byte| {
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect();
    Ok(json!({
        "kind": "target",
        "build": build,
        "engine_release": identity.release().as_str(),
        "advertised_api_version": format!("{}.{}", advertised.major, advertised.minor),
        "acquisition_api_version": format!("{}.{}", acquisition.major, acquisition.minor),
        "rendering_api_version": format!("{}.{}", rendering.major, rendering.minor),
        "daemon_mode": mode,
        "evidence_sha256": evidence,
    }))
}

fn assert_rendered_volumes(fixture: &FixtureExpectation) -> TestResult {
    let (application, source_length) = import_fixture(fixture)?;
    assert_neutral_volumes(&application, fixture, source_length)?;
    let volume_only = volume_only_projection(&application, fixture)?;
    let profiles = reviewed_profiles();
    assert_eq!(profiles.len(), 4, "the four exact native lane profiles are required");
    for profile in profiles {
        let planned = DockerExporter::new(profile.clone())?.plan(&volume_only, &target(&profile)?)?;
        assert!(
            planned
                .outcomes()
                .iter()
                .all(|outcome| outcome.kind() == ConversionKind::Exact)
        );
        let output = planned
            .candidate()
            .ok_or("strict authored labelled-volume candidate missing")?;
        assert_eq!(output.native_profile(), &profile);
        assert!(output.network_prerequisites().is_empty());
        assert!(output.volume_prerequisites().is_empty());
        assert!(output.dependency_decisions().is_empty());
        let raw = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
        let producer_raw = fixtures::render_fixture(fixtures::fixture(fixture.application)?, PREFIX, RUN, &profile)?;
        assert_eq!(raw, producer_raw);
        let complete: Value = serde_json::from_slice(&raw)?;
        assert_field_set(&complete, &["schema_version", "context", "requests", "prerequisites"])?;
        assert_eq!(complete["schema_version"], 1);
        assert_eq!(complete["context"], expected_selected_context(&profile)?);
        assert_eq!(complete["prerequisites"], json!([]));
        let requests = complete["requests"].as_array().ok_or("complete requests missing")?;
        assert_eq!(requests.len(), fixture.volumes.len());
        let api = profile.rendering_api_version();
        let path = format!("/v{}.{}/volumes/create", api.major, api.minor);
        let mut actual = BTreeMap::new();
        for request in requests {
            assert_field_set(request, &["method", "path", "body"])?;
            assert_eq!(request["method"], "POST");
            assert_eq!(request["path"], path);
            let name = request["body"]["Name"].as_str().ok_or("created volume name missing")?;
            assert!(
                actual.insert(name, &request["body"]).is_none(),
                "duplicate volume create"
            );
        }
        for (_, runtime_name) in fixture.volumes {
            let expected_body = json!({
                "Name": runtime_name,
                "Labels": {"io.boxferry.live-run": RUN, "io.boxferry.application": fixture.owner},
            });
            assert_eq!(actual.get(runtime_name).copied(), Some(&expected_body));
        }
        assert!(!planned.authorize(LossPolicy::ExactOnly).is_blocked());
    }
    Ok(())
}

#[test]
fn all_six_authored_fixtures_preserve_exact_23_neutral_volume_labels_and_source_evidence() -> TestResult {
    assert_eq!(FIXTURES.len(), 6);
    let profiles = reviewed_profiles();
    for lane in [
        "debian11-rootful",
        "debian11-rootless",
        "upstream-rootful",
        "upstream-rootless",
    ] {
        assert!(profiles.contains(&fixtures::selected_profile(lane)?));
    }
    assert_eq!(FIXTURES.iter().map(|fixture| fixture.volumes.len()).sum::<usize>(), 23);
    for fixture in FIXTURES {
        let (application, source_length) = import_fixture(fixture)?;
        assert_neutral_volumes(&application, fixture, source_length)?;
        let _ = volume_only_projection(&application, fixture)?;
    }
    Ok(())
}

macro_rules! rendered_fixture_test {
    ($name:ident, $index:expr) => {
        #[test]
        fn $name() -> TestResult {
            assert_rendered_volumes(&FIXTURES[$index])
        }
    };
}

rendered_fixture_test!(forgejo_volume_only_labels_render_exactly_in_four_reviewed_profiles, 0);
rendered_fixture_test!(nextcloud_volume_only_labels_render_exactly_in_four_reviewed_profiles, 1);
rendered_fixture_test!(paperless_volume_only_labels_render_exactly_in_four_reviewed_profiles, 2);
rendered_fixture_test!(immich_volume_only_labels_render_exactly_in_four_reviewed_profiles, 3);
rendered_fixture_test!(
    observability_volume_only_labels_render_exactly_in_four_reviewed_profiles,
    4
);
rendered_fixture_test!(supabase_volume_only_labels_render_exactly_in_four_reviewed_profiles, 5);

#[test]
fn missing_explicit_run_input_cannot_be_replaced_by_ambient_environment() -> TestResult {
    for fixture in FIXTURES {
        let (loaded, _) = loaded_fixture(fixture)?;
        assert!(!loaded.interpolate(&synthetic_environment(false)).is_valid());
    }
    Ok(())
}

fn first_fixture_volume() -> TestResult<Sourced<Volume>> {
    let (application, _) = import_fixture(&FIXTURES[0])?;
    Ok(application
        .volumes()
        .first()
        .ok_or("first fixture volume missing")?
        .clone())
}

fn single_volume_projection(volume: Volume, origins: Vec<Provenance>) -> TestResult<Application> {
    let mut application = Application::new(Identifier::new("volume-only-negative")?);
    let mut sourced = Sourced::generated(volume);
    for origin in origins {
        sourced.add_origin(origin);
    }
    application.add_volume(sourced)?;
    Ok(application)
}

fn assert_rejected_label(planned: ConversionPlan<DockerOutput>, subject: &str, kind: ConversionKind) {
    assert!(
        planned.candidate().is_none(),
        "unsafe labels must not emit create or relabel requests"
    );
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == subject && outcome.kind() == kind)
    );
    assert!(!format!("{planned:?}").contains(PROTECTED_LABEL));
    assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());
}

#[test]
fn external_fixture_volume_labels_never_become_create_or_relabel_operations() -> TestResult {
    let (original, origins) = first_fixture_volume()?.into_parts();
    let mut external = Volume::new(original.name().clone(), ResourceOwnership::External);
    external.set_runtime_name(original.runtime_name().ok_or("fixture runtime name missing")?.clone());
    external.set_labels_with_origins(
        original.labels().ok_or("fixture labels missing")?.to_vec(),
        original.labels_origins().to_vec(),
    );
    let application = single_volume_projection(external, origins)?;
    for profile in reviewed_profiles() {
        let planned = DockerExporter::new(profile.clone())?.plan(&application, &target(&profile)?)?;
        assert_rejected_label(
            planned,
            &format!("volumes.{}.labels", original.name().as_str()),
            ConversionKind::Unsupported,
        );
    }
    Ok(())
}

#[test]
fn protected_duplicate_and_oversize_fixture_volume_labels_fail_before_native_admission() -> TestResult {
    let (original, origins) = first_fixture_volume()?.into_parts();
    for invalid in ["protected", "duplicate", "oversize"] {
        let mut volume = original.clone();
        let mut labels = volume.labels().ok_or("fixture labels missing")?.to_vec();
        let kind = if invalid == "protected" {
            ConversionKind::Unsupported
        } else {
            ConversionKind::Invalid
        };
        let index = if invalid == "duplicate" {
            labels.push(labels[0].clone());
            2
        } else {
            let value = if invalid == "protected" {
                ProtectedString::sensitive(PROTECTED_LABEL)
            } else {
                ProtectedString::plain("x".repeat(4097))
            };
            let mut replacement = Sourced::generated(MetadataLabel::new(labels[0].value().name().clone(), value));
            for origin in labels[0].origins() {
                replacement.add_origin(origin.clone());
            }
            labels[0] = replacement;
            0
        };
        volume.set_labels_with_origins(labels, original.labels_origins().to_vec());
        let application = single_volume_projection(volume, origins.clone())?;
        for profile in reviewed_profiles() {
            let planned = DockerExporter::new(profile.clone())?.plan(&application, &target(&profile)?)?;
            assert_rejected_label(
                planned,
                &format!("volumes.{}.labels.{index}", original.name().as_str()),
                kind,
            );
        }
    }
    Ok(())
}

#[test]
fn wrong_implementation_version_or_evidence_key_cannot_authorize_fixture_labels() -> TestResult {
    let (application, _) = import_fixture(&FIXTURES[0])?;
    let volume_only = volume_only_projection(&application, &FIXTURES[0])?;
    for profile in reviewed_profiles() {
        let version = PlatformVersion::new(1, 0, 0);
        let wrong_version = TargetProfile::new("docker", version, Some(version))?;
        let wrong_implementation = TargetProfile::new("compose", version, Some(version))?;
        let key = CapabilityEvidenceKey::sha256([7; 32]).map_err(|error| format!("{error:?}"))?;
        let wrong_evidence = NativeTargetProfile::new(profile.identity().clone(), key);
        for (native, box_target) in [
            (profile.clone(), wrong_version),
            (profile.clone(), wrong_implementation),
            (wrong_evidence, target(&profile)?),
        ] {
            let planned = DockerExporter::new(native)?.plan(&volume_only, &box_target)?;
            assert_rejected_label(planned, "target.docker", ConversionKind::Invalid);
        }
    }
    Ok(())
}
