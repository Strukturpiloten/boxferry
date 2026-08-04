//! Runtime reconstruction contracts exercised through the public component API.

use boxferry_engine::{ConversionKind, DiagnosticCode, ImportAdapter};
use boxferry_model::{
    Command, EnvironmentValue, Identifier, ImageReference, Mount, MountSource, NetworkAttachment, Provenance,
    ProvenanceKind, ResourceOwnership, SourceId, Sourced,
};
use boxferry_runtime::{
    ContainerObservation, CreationEvidence, EffectiveCommand, ImageObservation, NetworkObservation,
    OverrideReconstruction, PodObservation, RuntimeEnvironmentVariable, RuntimeImplementation, RuntimeImporter,
    RuntimeResolutions, RuntimeSnapshot, VolumeObservation,
};

#[test]
fn image_comparison_retains_only_inferred_overrides_with_decision_provenance() -> Result<(), String> {
    let image_source = source("runtime:podman:image:web")?;
    let container_source = source("runtime:podman:container:web")?;
    let mut image = ImageObservation::new(image_source.clone());
    image.set_command(EffectiveCommand::exec(["server"]));
    image.set_environment(vec![environment("BASE", "image")?]);
    image.set_user("1000:1000");
    image.set_working_directory("/srv/app");

    let mut container = complete_container(container_source, "web")?;
    container.set_image(image_reference()?, Some(image_source));
    container.set_command(EffectiveCommand::exec(["server", "--production"]));
    container.set_environment(vec![environment("BASE", "image")?, environment("MODE", "production")?]);
    container.set_user("1001:1002");
    container.set_working_directory("/srv/app");
    container.set_read_only_root_filesystem(true);

    let mut snapshot = snapshot()?;
    snapshot.add_image(image).map_err(|error| error.to_string())?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.import(&snapshot);
    let application = result.application().ok_or("application expected")?;
    let service = application.services().first().ok_or("service expected")?.value();

    let command = service.command().ok_or("inferred command override expected")?;
    assert_eq!(origin_kinds(command.origins()), expected_inferred_origins());
    match command.value() {
        Command::Exec(arguments) => {
            assert_eq!(arguments.len(), 2);
            assert_eq!(arguments[1].expose(), "--production");
        }
        _ => return Err("exec command expected".to_owned()),
    }

    assert_eq!(service.environment().len(), 1);
    let mode = &service.environment()[0];
    assert_eq!(mode.value().name().as_str(), "MODE");
    assert_eq!(origin_kinds(mode.origins()), expected_inferred_origins());
    match mode.value().value() {
        EnvironmentValue::Literal(value) => assert_eq!(value.expose(), "production"),
        _ => return Err("literal environment value expected".to_owned()),
    }
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert!(service.working_directory().is_none());
    assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&true));

    for subject in [
        "services.web.command",
        "services.web.environment.BASE",
        "services.web.environment.MODE",
        "services.web.user",
        "services.web.group",
        "services.web.working_directory",
    ] {
        let outcome = result
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == subject)
            .ok_or_else(|| format!("outcome expected for {subject}"))?;
        assert_eq!(outcome.kind(), ConversionKind::Approximate);
        assert_eq!(outcome.diagnostic().map(DiagnosticCode::as_str), Some("BFR0002"));
    }
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "application.reconstruction")
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.read_only_root_filesystem" && outcome.kind() == ConversionKind::Exact
    }));
    assert!(!format!("{result:?}").contains("production"));
    assert!(!format!("{result:?}").contains("1001:1002"));
    Ok(())
}

#[test]
fn preserve_policy_keeps_effective_values_without_requiring_image_inspection() -> Result<(), String> {
    let mut container = complete_container(source("runtime:docker:container:web")?, "web")?;
    container.set_image(image_reference()?, None);
    container.set_command(EffectiveCommand::exec(["server", "--observed"]));
    container.set_environment(vec![environment("MODE", "observed")?]);
    container.set_user("1001:1002");
    container.set_working_directory("/srv/observed");
    container.set_read_only_root_filesystem(false);

    let mut snapshot = RuntimeSnapshot::new(id("example")?, RuntimeImplementation::Docker);
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    let result = importer(OverrideReconstruction::PreserveObservedState)?.import(&snapshot);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .ok_or("service expected")?
        .value();

    assert_eq!(
        origin_kinds(service.command().ok_or("command expected")?.origins()),
        vec![ProvenanceKind::RuntimeObservation]
    );
    assert_eq!(
        origin_kinds(service.environment()[0].origins()),
        vec![ProvenanceKind::RuntimeObservation]
    );
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/srv/observed")
    );
    assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&false));
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.subject() != "services.web.overrides")
    );
    Ok(())
}

#[test]
fn inference_preserves_effective_values_when_image_defaults_are_unavailable() -> Result<(), String> {
    let mut container = complete_container(source("runtime:podman:container:web")?, "web")?;
    container.set_image(image_reference()?, Some(source("runtime:podman:image:missing")?));
    container.set_command(EffectiveCommand::exec(["server", "--review-me"]));
    container.set_environment(vec![environment("MODE", "review-me")?]);
    container.set_user("1001:1002");
    container.set_working_directory("/srv/review-me");
    container.add_network(NetworkAttachment::new(id("missing-network")?, vec!["web".to_owned()]));

    let mut snapshot = snapshot()?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.import(&snapshot);
    let application = result.application().ok_or("application expected")?;
    let service = application.services().first().ok_or("service expected")?.value();

    assert_eq!(
        origin_kinds(service.command().ok_or("command expected")?.origins()),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision,]
    );
    assert_eq!(
        origin_kinds(service.environment()[0].origins()),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision,]
    );
    assert_eq!(
        origin_kinds(service.user().ok_or("user expected")?.origins()),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
    );
    assert_eq!(
        origin_kinds(service.group().ok_or("group expected")?.origins()),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
    );
    assert_eq!(
        origin_kinds(
            service
                .working_directory()
                .ok_or("working directory expected")?
                .origins()
        ),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
    );
    assert_eq!(application.networks().len(), 1);
    assert_eq!(
        application.networks()[0].value().ownership(),
        ResourceOwnership::Uncertain
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.overrides"
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0003")
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "networks.missing-network"
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0004")
    }));
    assert!(!format!("{result:?}").contains("review-me"));
    assert!(!format!("{result:?}").contains("1001:1002"));
    Ok(())
}

#[test]
fn preserves_alias_order_and_storage_relationships_with_uncertain_lifecycle() -> Result<(), String> {
    let mut snapshot = snapshot()?;
    snapshot
        .add_network(NetworkObservation::new(
            source("runtime:podman:network:frontend")?,
            id("frontend")?,
        ))
        .map_err(|error| error.to_string())?;
    snapshot
        .add_volume(VolumeObservation::new(
            source("runtime:podman:volume:data")?,
            id("data")?,
        ))
        .map_err(|error| error.to_string())?;

    let mut container = complete_container(source("runtime:podman:container:web")?, "web")?;
    container.set_image(image_reference()?, None);
    container.add_network(NetworkAttachment::new(
        id("frontend")?,
        vec!["web".to_owned(), "public-api".to_owned()],
    ));
    container.add_mount(
        Mount::new(MountSource::Volume(id("data")?), "/var/lib/example", false).map_err(|error| error.to_string())?,
    );
    snapshot.add_container(container).map_err(|error| error.to_string())?;

    let result = importer(OverrideReconstruction::PreserveObservedState)?.import(&snapshot);
    let application = result.application().ok_or("application expected")?;
    let service = application.services().first().ok_or("service expected")?.value();

    assert_eq!(
        application.networks()[0].value().ownership(),
        ResourceOwnership::Uncertain
    );
    assert_eq!(
        application.volumes()[0].value().ownership(),
        ResourceOwnership::Uncertain
    );
    assert_eq!(service.networks()[0].value().aliases(), ["web", "public-api"]);
    assert!(matches!(
        service.mounts()[0].value().source(),
        MountSource::Volume(name) if name.as_str() == "data"
    ));
    for subject in ["networks.frontend", "volumes.data"] {
        let outcome = result
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == subject)
            .ok_or_else(|| format!("outcome expected for {subject}"))?;
        assert_eq!(outcome.kind(), ConversionKind::Approximate);
        assert_eq!(outcome.diagnostic().map(DiagnosticCode::as_str), Some("BFR0004"));
    }
    Ok(())
}

#[test]
fn creation_command_is_optional_evidence_and_never_changes_effective_values() -> Result<(), String> {
    let without_evidence = snapshot_with_optional_evidence(false)?;
    let with_evidence = snapshot_with_optional_evidence(true)?;
    let importer = importer(OverrideReconstruction::PreserveObservedState)?;
    let without = importer.import(&without_evidence);
    let with = importer.import(&with_evidence);

    assert_eq!(without.application(), with.application());
    let without_origins = reconstruction_outcome(&without)?.origins().len();
    let with_origins = reconstruction_outcome(&with)?.origins().len();
    assert_eq!(with_origins, without_origins + 1);
    assert_eq!(
        reconstruction_outcome(&with)?
            .origins()
            .last()
            .map(|origin| origin.source_id().as_str()),
        Some("runtime:podman:create:web")
    );
    Ok(())
}

#[test]
fn pod_membership_becomes_a_provenance_aware_neutral_service_group() -> Result<(), String> {
    let container_source = source("runtime:podman:container:web")?;
    let pod_source = source("runtime:podman:pod:example")?;
    let mut container = complete_container(container_source.clone(), "web")?;
    container.set_image(image_reference()?, None);
    container.set_pod_source_id(pod_source.clone());

    let mut pod = PodObservation::new(pod_source, id("example-pod")?);
    pod.add_member(container_source.clone());
    let mut snapshot = snapshot()?;
    snapshot.add_pod(pod).map_err(|error| error.to_string())?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;

    assert_eq!(snapshot.pods()[0].members(), [container_source]);
    let result = importer(OverrideReconstruction::PreserveObservedState)?.import(&snapshot);
    let application = result.application().ok_or("application expected")?;
    let group = application.service_groups().first().ok_or("service group expected")?;
    assert_eq!(group.value().name().as_str(), "example-pod");
    assert_eq!(group.value().ownership(), ResourceOwnership::Uncertain);
    assert_eq!(group.value().members()[0].value().as_str(), "web");
    assert_eq!(
        origin_kinds(group.value().members()[0].origins()),
        vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::RuntimeObservation]
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.example-pod.members[0]" && outcome.kind() == ConversionKind::Exact
    }));
    let outcome = result
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "service_groups.example-pod.lifecycle")
        .ok_or("service-group lifecycle outcome expected")?;
    assert_eq!(outcome.kind(), ConversionKind::Approximate);
    assert_eq!(outcome.diagnostic().map(DiagnosticCode::as_str), Some("BFR0005"));
    Ok(())
}

#[test]
fn explicit_lifecycle_resolutions_retain_observation_and_user_override_provenance() -> Result<(), String> {
    let container_source = source("runtime:podman:container:web")?;
    let pod_source = source("runtime:podman:pod:observed")?;
    let mut container = complete_container(container_source.clone(), "web")?;
    container.set_image(image_reference()?, None);
    container.set_pod_source_id(pod_source.clone());
    container.add_network(NetworkAttachment::new(id("frontend")?, Vec::new()));
    container.add_mount(
        Mount::new(MountSource::Volume(id("data")?), "/var/lib/example", false).map_err(|error| error.to_string())?,
    );

    let mut pod = PodObservation::new(pod_source, id("observed-pod")?);
    pod.add_member(container_source);
    let mut snapshot = snapshot()?;
    snapshot
        .add_network(NetworkObservation::new(
            source("runtime:podman:network:frontend")?,
            id("frontend")?,
        ))
        .map_err(|error| error.to_string())?;
    snapshot
        .add_volume(VolumeObservation::new(
            source("runtime:podman:volume:data")?,
            id("data")?,
        ))
        .map_err(|error| error.to_string())?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    snapshot.add_pod(pod).map_err(|error| error.to_string())?;

    let mut resolutions = RuntimeResolutions::new();
    resolutions
        .set_network_ownership(id("frontend")?, user_resolution(ResourceOwnership::External)?)
        .map_err(|error| error.to_string())?;
    resolutions
        .set_volume_ownership(id("data")?, user_resolution(ResourceOwnership::Application)?)
        .map_err(|error| error.to_string())?;
    resolutions
        .set_service_group_ownership(id("observed-pod")?, user_resolution(ResourceOwnership::Application)?)
        .map_err(|error| error.to_string())?;

    let importer = importer(OverrideReconstruction::PreserveObservedState)?.with_resolutions(resolutions);
    let result = importer.import(&snapshot);
    let application = result.application().ok_or("application expected")?;
    assert_eq!(
        application.networks()[0].value().ownership(),
        ResourceOwnership::External
    );
    assert_eq!(
        application.volumes()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert_eq!(
        application.service_groups()[0].value().ownership(),
        ResourceOwnership::Application
    );
    for origins in [
        application.networks()[0].origins(),
        application.volumes()[0].origins(),
        application.service_groups()[0].origins(),
    ] {
        assert_eq!(
            origin_kinds(origins),
            vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::UserOverride]
        );
    }
    for subject in [
        "networks.frontend",
        "volumes.data",
        "service_groups.observed-pod.lifecycle",
    ] {
        let outcome = result
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == subject)
            .ok_or_else(|| format!("outcome expected for {subject}"))?;
        assert_eq!(outcome.kind(), ConversionKind::Approximate);
        assert_eq!(outcome.diagnostic().map(DiagnosticCode::as_str), Some("BFR0009"));
        assert_eq!(
            origin_kinds(outcome.origins()),
            vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::UserOverride]
        );
    }
    Ok(())
}

#[test]
fn contradictory_pod_and_container_membership_is_invalid_instead_of_guessed() -> Result<(), String> {
    let container_source = source("runtime:podman:container:web")?;
    let pod_source = source("runtime:podman:pod:example")?;
    let other_pod_source = source("runtime:podman:pod:other")?;
    let mut container = complete_container(container_source.clone(), "web")?;
    container.set_image(image_reference()?, None);
    container.set_pod_source_id(other_pod_source);

    let mut pod = PodObservation::new(pod_source, id("example-pod")?);
    pod.add_member(container_source);
    let mut snapshot = snapshot()?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    snapshot.add_pod(pod).map_err(|error| error.to_string())?;

    let result = importer(OverrideReconstruction::PreserveObservedState)?.import(&snapshot);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.example-pod.members[0]"
            && outcome.kind() == ConversionKind::Invalid
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0008")
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.service_group"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0005")
    }));
    Ok(())
}

#[test]
fn missing_pod_member_is_reported_and_not_invented_in_the_neutral_group() -> Result<(), String> {
    let mut pod = PodObservation::new(source("runtime:podman:pod:example")?, id("example-pod")?);
    pod.add_member(source("runtime:podman:container:missing")?);
    let mut snapshot = snapshot()?;
    snapshot.add_pod(pod).map_err(|error| error.to_string())?;

    let result = importer(OverrideReconstruction::PreserveObservedState)?.import(&snapshot);
    let group = result
        .application()
        .and_then(|application| application.service_groups().first())
        .ok_or("empty service group expected")?;
    assert!(group.value().members().is_empty());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.example-pod.members[0]"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0005")
    }));
    Ok(())
}

fn snapshot_with_optional_evidence(include: bool) -> Result<RuntimeSnapshot, String> {
    let mut container = complete_container(source("runtime:podman:container:web")?, "web")?;
    container.set_image(image_reference()?, None);
    if include {
        container.set_creation_evidence(CreationEvidence::new(
            source("runtime:podman:create:web")?,
            ["podman", "run", "--env", "PASSWORD=never-print-this"],
        ));
    }
    let mut snapshot = snapshot()?;
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    Ok(snapshot)
}

fn complete_container(source_id: SourceId, name: &str) -> Result<ContainerObservation, String> {
    let mut container = ContainerObservation::new(source_id, id(name)?);
    container.set_command(EffectiveCommand::Empty);
    container.set_environment(Vec::new());
    Ok(container)
}

fn reconstruction_outcome(
    result: &boxferry_engine::ImportResult,
) -> Result<&boxferry_engine::ConversionOutcome, String> {
    result
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.reconstruction")
        .ok_or_else(|| "reconstruction outcome expected".to_owned())
}

fn origin_kinds(origins: &[Provenance]) -> Vec<ProvenanceKind> {
    origins.iter().map(Provenance::kind).collect()
}

fn expected_inferred_origins() -> Vec<ProvenanceKind> {
    vec![
        ProvenanceKind::RuntimeObservation,
        ProvenanceKind::RuntimeObservation,
        ProvenanceKind::ConversionDecision,
    ]
}

fn importer(policy: OverrideReconstruction) -> Result<RuntimeImporter, String> {
    RuntimeImporter::new(policy).map_err(|error| error.to_string())
}

fn snapshot() -> Result<RuntimeSnapshot, String> {
    Ok(RuntimeSnapshot::new(id("example")?, RuntimeImplementation::Podman))
}

fn image_reference() -> Result<ImageReference, String> {
    ImageReference::parse("example.invalid/web:1@sha256:abcd").map_err(|error| error.to_string())
}

fn environment(name: &str, value: &str) -> Result<RuntimeEnvironmentVariable, String> {
    Ok(RuntimeEnvironmentVariable::new(id(name)?, value))
}

fn id(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}

fn source(value: &str) -> Result<SourceId, String> {
    SourceId::new(value).map_err(|error| error.to_string())
}

fn user_resolution(ownership: ResourceOwnership) -> Result<Sourced<ResourceOwnership>, String> {
    Ok(Sourced::from_source(
        ownership,
        Provenance::user_override(source("decision:runtime-lifecycle")?),
    ))
}
