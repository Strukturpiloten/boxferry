use std::{error::Error, num::NonZeroU16, time::Duration};

use boxferry_engine::{
    ConversionKind, ExportAdapter, ImportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile,
    convert_imported,
};
use boxferry_model::{Identifier, MetadataLabel, ProtectedString, Provenance, ProvenanceKind, Sourced};
use docker_lens::{
    acquisition::{Budget, Limits, NativeId, ReadRequest},
    decoder::{DecodedInventory, LabelObservation, decode_capture},
    evidence::{HttpStatus, ProtectedValue},
    observation::{Availability, Observed, Origin, ResourceRef},
    version::{ApiVersion, DaemonMode, EngineBuild, TargetCapabilityCatalog},
};
use serde_json::{Value, json};

use super::{
    DockerPublicLabelDecision, DockerPublicLabelDecisionError, DockerPublicLabelDecisions,
    DockerPublicLabelResourceKind as Kind, PUBLIC_LABEL_TOTAL_LIMIT, PUBLIC_LABEL_VALUE_LIMIT,
};
use crate::{DockerExporter, DockerImporter, DockerPromotionPolicy, DockerResourceDecisions, DockerSource};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const KEY: &str = "org.example.ordinary";
const VALUE: &str = "ordinary-observed-setting";
const UNSELECTED: &str = "unselected-protected-canary";

fn native_id(value: String) -> TestResult<NativeId> {
    NativeId::new(value).ok_or_else(|| "invalid fixture ID".into())
}

fn inventory() -> TestResult<DecodedInventory> {
    let mut budget = Budget::new(Limits {
        max_requests: 8,
        max_selected_resources: 8,
        max_expansions: 8,
        max_response_bytes: 8192,
        max_total_bytes: 32768,
        max_elapsed: Duration::from_secs(2),
    })
    .map_err(|_| "fixture budget invalid")?;
    let api = ApiVersion::new(NonZeroU16::new(1).ok_or("invalid fixture major")?, 49);
    let documents = [
        (
            ReadRequest::DaemonVersion,
            None,
            json!({"Version":"29.8.1","ApiVersion":"1.49","MinAPIVersion":"1.40"}),
        ),
        (
            ReadRequest::InspectContainer(native_id("a".repeat(64))?),
            Some(ResourceRef::new(1)),
            json!({"Id":"a".repeat(64),"Config":{"Image":"registry.invalid/metadata:1","Labels":{KEY:VALUE,"private.unselected":UNSELECTED,"empty.setting":""}}}),
        ),
        (
            ReadRequest::InspectNetwork(native_id("b".repeat(64))?),
            Some(ResourceRef::new(2)),
            json!({"Id":"b".repeat(64),"Name":"source-network","Driver":"bridge","Internal":false,"Containers":{},"Labels":{KEY:VALUE}}),
        ),
        (
            ReadRequest::InspectVolume(native_id("source-data".to_owned())?),
            Some(ResourceRef::new(3)),
            json!({"Name":"source-data","Driver":"local","Labels":{KEY:VALUE}}),
        ),
        (
            ReadRequest::InspectNetwork(native_id("c".repeat(64))?),
            Some(ResourceRef::new(4)),
            json!({"Id":"c".repeat(64),"Name":"external-edge","Driver":"bridge","Internal":false,"Containers":{},"Labels":{KEY:UNSELECTED}}),
        ),
    ];
    budget.record_selection(1).map_err(|_| "fixture selection invalid")?;
    for (request, reference, document) in documents {
        let version = reference.map(|_| api);
        budget
            .record_request(request, reference, version)
            .map_err(|_| "fixture request invalid")?;
        let bytes = serde_json::to_vec(&document)?;
        budget
            .read_response(
                HttpStatus::new(200).map_err(|_| "fixture status invalid")?,
                bytes.as_slice(),
            )
            .map_err(|_| "fixture response invalid")?;
    }
    decode_capture(&budget.into_capture().map_err(|_| "fixture capture invalid")?)
        .map_err(|error| format!("fixture decode invalid: {error:?}").into())
}

fn source(
    inventory: DecodedInventory,
    decisions: Vec<DockerPublicLabelDecision>,
    portable: bool,
) -> TestResult<DockerSource> {
    let mut resources = DockerResourceDecisions::new();
    for spec in [
        format!("network:{}=create:destination-network", "b".repeat(64)),
        format!("network:{}=external:external-edge", "c".repeat(64)),
        "volume:source-data=create:destination-data".to_owned(),
    ] {
        resources.add(spec.parse()?)?;
    }
    Ok(DockerSource::new(Identifier::new("metadata-review")?, inventory)
        .with_promotion_policy(if portable {
            DockerPromotionPolicy::portable_application()
        } else {
            DockerPromotionPolicy::conservative()
        })
        .with_resource_decisions(resources)
        .with_public_label_decisions(DockerPublicLabelDecisions::new(decisions)?))
}

fn declaration(kind: Kind, key: &str) -> TestResult<DockerPublicLabelDecision> {
    let identity = match kind {
        Kind::Container => "a".repeat(64),
        Kind::Network => "b".repeat(64),
        Kind::Volume => "source-data".to_owned(),
    };
    Ok(DockerPublicLabelDecision::new(kind, identity, key.to_owned())?)
}

fn exporter() -> TestResult<(DockerExporter, TargetProfile)> {
    let profile = TargetCapabilityCatalog::reviewed()
        .profiles()
        .find(|profile| {
            matches!(profile.identity().build(), EngineBuild::Upstream)
                && profile.release().as_str() == "29.8.1"
                && profile.mode() == DaemonMode::Rootful
        })
        .ok_or("reviewed target absent")?
        .clone();
    let version = PlatformVersion::new(29, 8, 1);
    Ok((
        DockerExporter::new(profile)?,
        TargetProfile::new("docker", version, Some(version))?,
    ))
}

#[test]
fn constructors_are_exact_bounded_duplicate_free_and_redacted() -> TestResult {
    for (kind, source) in [
        (Kind::Container, "a".repeat(64)),
        (Kind::Network, "b".repeat(64)),
        (Kind::Volume, "valid-data.name".to_owned()),
    ] {
        let decision = DockerPublicLabelDecision::new(kind, source.clone(), "ordinary-key-canary".to_owned())?;
        assert_eq!(decision.kind(), kind);
        assert_eq!(decision.source(), source);
        assert_eq!(decision.key(), "ordinary-key-canary");
        let debug = format!("{decision:?}");
        assert!(!debug.contains(&source));
        assert!(!debug.contains("ordinary-key-canary"));
    }
    for (kind, source) in [
        (Kind::Container, "a".repeat(63)),
        (Kind::Network, "A".repeat(64)),
        (Kind::Volume, "/invalid".to_owned()),
    ] {
        assert_eq!(
            DockerPublicLabelDecision::new(kind, source, "key".to_owned()),
            Err(DockerPublicLabelDecisionError::InvalidSourceIdentity)
        );
    }
    for key in [String::new(), "x\0y".to_owned(), "é".repeat(129)] {
        assert_eq!(
            DockerPublicLabelDecision::new(Kind::Container, "a".repeat(64), key),
            Err(DockerPublicLabelDecisionError::InvalidKey)
        );
    }
    assert!(DockerPublicLabelDecision::new(Kind::Container, "a".repeat(64), "é".repeat(128)).is_ok());
    let entry = declaration(Kind::Container, KEY)?;
    assert_eq!(
        DockerPublicLabelDecisions::new(vec![entry.clone(), entry]),
        Err(DockerPublicLabelDecisionError::Duplicate)
    );
    let entries = (0..128)
        .map(|index| declaration(Kind::Container, &format!("ordinary.{index}")))
        .collect::<TestResult<Vec<_>>>()?;
    assert_eq!(DockerPublicLabelDecisions::new(entries.clone())?.entries().len(), 128);
    let mut excess = entries;
    excess.push(declaration(Kind::Volume, "another")?);
    assert_eq!(
        DockerPublicLabelDecisions::new(excess),
        Err(DockerPublicLabelDecisionError::TooMany)
    );
    assert!(DockerPublicLabelDecisions::default().entries().is_empty());
    Ok(())
}

#[test]
fn conservative_and_portable_presets_never_declare_labels_public() -> TestResult {
    for portable in [false, true] {
        let source = source(inventory()?, vec![], portable)?;
        assert!(source.public_label_decisions().entries().is_empty());
        let imported = DockerImporter::new()?.import(&source);
        let application = imported.application().ok_or("missing neutral application")?;
        assert!(application.services()[0].value().labels().is_empty());
        assert!(
            application
                .networks()
                .iter()
                .all(|network| network.value().labels().is_none())
        );
        assert!(application.volumes()[0].value().labels().is_none());
        for subject in [
            "services[0].labels",
            "networks[0].labels",
            "networks[1].labels",
            "volumes[0].labels",
        ] {
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported)
            );
        }
        assert!(!format!("{imported:?}").contains(UNSELECTED));
    }
    Ok(())
}

#[test]
fn decoded_labels_copy_actual_values_with_explicit_provenance_and_resource_isolation() -> TestResult {
    let decisions = [Kind::Container, Kind::Network, Kind::Volume]
        .into_iter()
        .map(|kind| declaration(kind, KEY))
        .collect::<TestResult<Vec<_>>>()?;
    let source = source(inventory()?, decisions, true)?;
    let imported = DockerImporter::new()?.import(&source);
    let app = imported.application().ok_or("missing neutral application")?;
    let labels = [
        app.services()[0].value().labels()[0].clone(),
        app.networks()[0].value().labels().ok_or("network labels absent")?[0].clone(),
        app.volumes()[0].value().labels().ok_or("volume labels absent")?[0].clone(),
    ];
    for label in labels {
        assert_eq!(label.value().name().as_str(), KEY);
        assert_eq!(label.value().value().expose(), VALUE);
        assert!(!label.value().value().is_sensitive());
        assert_eq!(
            label.origins().iter().map(Provenance::kind).collect::<Vec<_>>(),
            vec![
                ProvenanceKind::RuntimeObservation,
                ProvenanceKind::UserOverride,
                ProvenanceKind::ConversionDecision
            ]
        );
    }
    assert!(app.networks()[1].value().labels().is_none());
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "networks[1].labels" && outcome.kind() == ConversionKind::Unsupported)
    );
    let reconstructed: Vec<_> = imported
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFD0003")
        .collect();
    assert!(reconstructed.len() >= 3);
    assert!(
        reconstructed
            .iter()
            .all(|diagnostic| diagnostic.severity() == Severity::Note)
    );
    let text = format!("{:?}{:?}", imported.diagnostics(), imported.outcomes());
    for value in [KEY, VALUE, UNSELECTED] {
        assert!(!text.contains(value));
    }
    let (exporter, target) = exporter()?;
    let converted = convert_imported(imported, &exporter, &target, LossPolicy::AllowPartial)?;
    let output = converted
        .output()
        .ok_or("declared public metadata did not produce an inert candidate")?;
    let requests: Vec<Value> = std::str::from_utf8(output.bytes())?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    for suffix in [
        "/networks/create",
        "/volumes/create",
        "/containers/create?name=docker-service-1",
    ] {
        let request = requests
            .iter()
            .find(|request| request["path"].as_str().is_some_and(|path| path.ends_with(suffix)))
            .ok_or("expected native create request missing")?;
        assert_eq!(request["body"]["Labels"], json!({KEY:VALUE}));
    }
    assert!(!std::str::from_utf8(output.bytes())?.contains(UNSELECTED));
    Ok(())
}

#[test]
fn empty_observed_value_is_preserved_without_reconstructing_redaction() -> TestResult {
    let source = source(
        inventory()?,
        vec![declaration(Kind::Container, "empty.setting")?],
        false,
    )?;
    let imported = DockerImporter::new()?.import(&source);
    let label = &imported.application().ok_or("missing neutral application")?.services()[0]
        .value()
        .labels()[0];
    assert_eq!(label.value().value().expose(), "");
    assert!(!label.value().value().is_sensitive());
    Ok(())
}

#[test]
fn explicit_external_labels_and_existing_sensitive_labels_still_refuse_export() -> TestResult {
    let external = DockerPublicLabelDecision::new(Kind::Network, "c".repeat(64), KEY.to_owned())?;
    let imported = DockerImporter::new()?.import(&source(inventory()?, vec![external], true)?);
    let (exporter, target) = exporter()?;
    let plan = exporter.plan(imported.application().ok_or("missing neutral application")?, &target)?;
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.subject().contains("labels") && outcome.kind() == ConversionKind::Unsupported)
    );
    let partial = plan.authorize(LossPolicy::AllowPartial);
    if let Some(output) = partial.output() {
        assert!(!std::str::from_utf8(output.bytes())?.contains(UNSELECTED));
        assert!(output.network_prerequisites().is_empty());
    }
    let imported = DockerImporter::new()?.import(&source(inventory()?, vec![], true)?);
    let (mut app, _, _) = imported.into_parts();
    let app = app.as_mut().ok_or("missing neutral application")?;
    let mut service = app.services()[0].value().clone();
    service.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new(KEY)?,
        ProtectedString::sensitive("sensitive-label-canary"),
    )));
    let mut neutral = boxferry_model::Application::new(Identifier::new("protected-review")?);
    neutral.add_service(Sourced::generated(service))?;
    let plan = exporter.plan(&neutral, &target)?;
    assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());
    Ok(())
}

fn assert_invalid(inventory: DecodedInventory, decisions: Vec<DockerPublicLabelDecision>) -> TestResult {
    let imported = DockerImporter::new()?.import(&source(inventory, decisions, true)?);
    assert!(imported.application().is_none());
    assert!(
        imported
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity() == Severity::Error
                && diagnostic.fields().iter().any(|field| field.name() == "subject"
                    && field
                        .value()
                        .redacted()
                        .starts_with("application.public_label_decisions[")))
    );
    let text = format!("{:?}{:?}", imported.diagnostics(), imported.outcomes());
    let source_identity = "a".repeat(64);
    for value in [
        KEY,
        VALUE,
        UNSELECTED,
        "bad-native-value-canary",
        source_identity.as_str(),
    ] {
        assert!(!text.contains(value));
    }
    Ok(())
}

fn copied_labels(observed: &Observed<Vec<LabelObservation>>) -> TestResult<Vec<LabelObservation>> {
    Ok(observed
        .value()
        .ok_or("fixture labels absent")?
        .iter()
        .map(|label| LabelObservation {
            key: ProtectedValue::new(label.key.as_bytes().to_vec()),
            value: label.value.value().map_or_else(
                || Observed::unavailable(label.value.availability, label.value.origin),
                |value| {
                    Observed::present(
                        ProtectedValue::new(value.as_bytes().to_vec()),
                        label.value.availability,
                        label.value.origin,
                    )
                },
            ),
        })
        .collect())
}

fn change_label(inventory: &mut DecodedInventory, change: impl FnOnce(&mut LabelObservation)) -> TestResult {
    let collection = &mut inventory.containers[0].labels;
    let mut labels = copied_labels(collection)?;
    change(
        labels
            .iter_mut()
            .find(|label| label.key.as_bytes() == KEY.as_bytes())
            .ok_or("fixture label absent")?,
    );
    *collection = Observed::present(labels, collection.availability, collection.origin);
    Ok(())
}

#[test]
fn collection_and_value_origin_are_reviewed_independently() -> TestResult {
    for kind in [Kind::Container, Kind::Network, Kind::Volume] {
        for origin in [Origin::Configured, Origin::RuntimeAssigned, Origin::Unknown] {
            let mut observed = inventory()?;
            let labels = match kind {
                Kind::Container => &mut observed.containers[0].labels,
                Kind::Network => &mut observed.networks[0].labels,
                Kind::Volume => &mut observed.volumes[0].labels,
            };
            labels.origin = origin;
            assert_invalid(observed, vec![declaration(kind, KEY)?])?;
            let mut observed = inventory()?;
            let labels = match kind {
                Kind::Container => &mut observed.containers[0].labels,
                Kind::Network => &mut observed.networks[0].labels,
                Kind::Volume => &mut observed.volumes[0].labels,
            };
            let mut values = copied_labels(labels)?;
            let label = values
                .iter_mut()
                .find(|label| label.key.as_bytes() == KEY.as_bytes())
                .ok_or("fixture label absent")?;
            label.value.origin = origin;
            *labels = Observed::present(values, labels.availability, labels.origin);
            assert_invalid(observed, vec![declaration(kind, KEY)?])?;
        }
    }
    Ok(())
}

#[test]
fn contradictory_missing_and_redacted_collection_or_value_never_become_empty() -> TestResult {
    for availability in [
        Availability::Missing,
        Availability::Null,
        Availability::Empty,
        Availability::Redacted,
    ] {
        let mut observed = inventory()?;
        observed.containers[0].labels.availability = availability;
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
        let mut observed = inventory()?;
        observed.containers[0].labels = Observed::unavailable(availability, Origin::Effective);
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
        let mut observed = inventory()?;
        change_label(&mut observed, |label| label.value.availability = availability)?;
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
        let mut observed = inventory()?;
        change_label(&mut observed, |label| {
            label.value = Observed::unavailable(availability, Origin::Effective);
        })?;
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    }
    let mut observed = inventory()?;
    change_label(&mut observed, |label| {
        label.value = Observed::present(ProtectedValue::new(vec![]), Availability::Present, Origin::Effective);
    })?;
    assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    for bytes in [vec![0xff], b"bad-native-value-canary\0".to_vec()] {
        let mut observed = inventory()?;
        change_label(&mut observed, |label| {
            label.value = Observed::present(ProtectedValue::new(bytes), Availability::Present, Origin::Effective);
        })?;
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    }
    Ok(())
}

#[test]
fn stale_unselected_ambiguous_or_unreviewed_resource_identity_is_refused() -> TestResult {
    let stale = DockerPublicLabelDecision::new(Kind::Container, "d".repeat(64), KEY.to_owned())?;
    assert_invalid(inventory()?, vec![stale])?;
    assert_invalid(inventory()?, vec![declaration(Kind::Container, "missing-label")?])?;
    for kind in [Kind::Container, Kind::Network, Kind::Volume] {
        let mut observed = inventory()?;
        let mut duplicate = inventory()?;
        match kind {
            Kind::Container => observed.containers.push(duplicate.containers.remove(0)),
            Kind::Network => observed.networks.push(duplicate.networks.remove(0)),
            Kind::Volume => observed.volumes.push(duplicate.volumes.remove(0)),
        }
        assert_invalid(observed, vec![declaration(kind, KEY)?])?;
    }
    for origin in [Origin::Configured, Origin::Effective, Origin::Unknown] {
        let mut observed = inventory()?;
        observed.containers[0].id.origin = origin;
        assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    }
    let mut observed = inventory()?;
    let mut labels = copied_labels(&observed.containers[0].labels)?;
    labels.push(LabelObservation {
        key: ProtectedValue::new(KEY.as_bytes().to_vec()),
        value: Observed::present(
            ProtectedValue::new(VALUE.as_bytes().to_vec()),
            Availability::Present,
            Origin::Effective,
        ),
    });
    observed.containers[0].labels = Observed::present(labels, Availability::Present, Origin::Effective);
    assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    let mut observed = inventory()?;
    observed.containers.clear();
    observed
        .discovered_containers
        .push(docker_lens::decoder::ContainerSummary {
            id: ProtectedValue::new("a".repeat(64).into_bytes()),
            names: Observed::unavailable(Availability::Missing, Origin::Unknown),
            labels: Observed::unavailable(Availability::Missing, Origin::Unknown),
            image: Observed::unavailable(Availability::Missing, Origin::Unknown),
        });
    assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    Ok(())
}

#[test]
fn individual_value_budget_accepts_boundary_and_refuses_one_more_byte() -> TestResult {
    let mut observed = inventory()?;
    change_label(&mut observed, |label| {
        label.value = Observed::present(
            ProtectedValue::new(vec![b'x'; PUBLIC_LABEL_VALUE_LIMIT]),
            Availability::Present,
            Origin::Effective,
        );
    })?;
    let imported = DockerImporter::new()?.import(&source(observed, vec![declaration(Kind::Container, KEY)?], true)?);
    assert_eq!(
        imported.application().ok_or("boundary value refused")?.services()[0]
            .value()
            .labels()[0]
            .value()
            .value()
            .expose()
            .len(),
        PUBLIC_LABEL_VALUE_LIMIT
    );
    let mut observed = inventory()?;
    change_label(&mut observed, |label| {
        label.value = Observed::present(
            ProtectedValue::new(vec![b'x'; PUBLIC_LABEL_VALUE_LIMIT + 1]),
            Availability::Present,
            Origin::Effective,
        );
    })?;
    assert_invalid(observed, vec![declaration(Kind::Container, KEY)?])?;
    Ok(())
}

fn aggregate_inventory(extra: usize) -> TestResult<(DecodedInventory, Vec<DockerPublicLabelDecision>)> {
    let keys: Vec<_> = (0..128).map(|index| format!("ordinary.{index:03}")).collect();
    let key_bytes: usize = keys.iter().map(String::len).sum();
    let mut labels = Vec::new();
    let mut decisions = Vec::new();
    for (index, key) in keys.into_iter().enumerate() {
        decisions.push(declaration(Kind::Container, &key)?);
        let length = if index == 0 {
            PUBLIC_LABEL_VALUE_LIMIT - key_bytes + extra
        } else {
            PUBLIC_LABEL_VALUE_LIMIT
        };
        labels.push(LabelObservation {
            key: ProtectedValue::new(key.into_bytes()),
            value: Observed::present(
                ProtectedValue::new(vec![b'x'; length]),
                Availability::Present,
                Origin::Effective,
            ),
        });
    }
    let mut observed = inventory()?;
    observed.containers[0].labels = Observed::present(labels, Availability::Present, Origin::Effective);
    Ok((observed, decisions))
}

#[test]
fn aggregate_copy_budget_accepts_boundary_and_refuses_one_more_byte() -> TestResult {
    let (observed, decisions) = aggregate_inventory(0)?;
    let imported = DockerImporter::new()?.import(&source(observed, decisions, true)?);
    let labels = imported.application().ok_or("aggregate boundary refused")?.services()[0]
        .value()
        .labels();
    assert_eq!(labels.len(), 128);
    assert_eq!(
        labels
            .iter()
            .map(|label| label.value().name().as_str().len() + label.value().value().expose().len())
            .sum::<usize>(),
        PUBLIC_LABEL_TOTAL_LIMIT
    );
    let (observed, decisions) = aggregate_inventory(1)?;
    assert_invalid(observed, decisions)?;
    Ok(())
}
