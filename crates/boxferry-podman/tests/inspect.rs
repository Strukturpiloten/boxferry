//! Fixture-driven Podman inspect boundary tests.

use std::path::{Path, PathBuf};

use boxferry_engine::{ConversionKind, DiagnosticCode, ImportAdapter, PlatformVersion, Severity};
use boxferry_model::{
    Command, EnvironmentValue, Identifier, MountSource, Protocol, Provenance, ResourceOwnership, SelinuxRelabel,
    SourceId, Sourced,
};
use boxferry_podman::{PodmanImporter, PodmanInspectDocuments, PodmanInspectSource};
use boxferry_runtime::{EffectiveCommand, OverrideReconstruction, RuntimeImplementation, RuntimeResolutions};

#[test]
fn podman_5_4_decodes_effective_state_and_relationships_without_raw_id_leaks() -> Result<(), String> {
    let source = fixture_source("podman-inspect-5-4", PlatformVersion::new(5, 4, 0))?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.decode(&source);
    let snapshot = result.snapshot().ok_or("snapshot expected")?;

    assert_eq!(snapshot.implementation(), &RuntimeImplementation::Podman);
    assert_eq!(snapshot.images().len(), 1);
    assert_eq!(snapshot.networks().len(), 1);
    assert_eq!(snapshot.volumes().len(), 1);
    assert_eq!(snapshot.pods().len(), 1);
    assert!(snapshot.pods()[0].creation_evidence().is_some());
    assert_eq!(snapshot.pods()[0].members().len(), 1);

    let container = snapshot.containers().first().ok_or("container expected")?;
    assert_eq!(container.name().as_str(), "web");
    assert_eq!(
        container.image().map(boxferry_model::ImageReference::as_str),
        Some("registry.example.invalid/team/web:1.2@sha256:abcd")
    );
    assert!(container.image_source_id().is_some());
    assert!(container.pod_source_id().is_some());
    assert!(container.creation_evidence().is_some());
    assert_eq!(container.networks()[0].aliases(), ["web", "public-api"]);
    assert_eq!(container.ports()[0].container(), 8080);
    assert_eq!(container.ports()[0].published(), Some(18080));
    assert_eq!(container.ports()[0].host_address(), Some("127.0.0.1"));
    assert_eq!(container.ports()[0].protocol(), &Protocol::Tcp);
    assert!(matches!(container.mounts()[0].source(), MountSource::Volume(name) if name.as_str() == "data"));
    assert_eq!(container.mounts()[1].selinux_relabel(), Some(SelinuxRelabel::Private));
    assert!(container.mounts()[1].read_only());
    assert!(matches!(container.command(), Some(EffectiveCommand::Exec(arguments)) if arguments.len() == 2));
    assert_eq!(
        container.user().map(boxferry_model::ProtectedString::expose),
        Some("1001:1002")
    );
    assert_eq!(
        container
            .working_directory()
            .map(boxferry_model::ProtectedString::expose),
        Some("/srv/app")
    );
    assert_eq!(container.read_only_root_filesystem(), Some(true));

    let rendered = format!("{source:?} {result:?}");
    for sensitive in [
        "MODE=production",
        "--production",
        "container-runtime-id-web",
        "image-runtime-id-web",
        "pod-runtime-id-app",
        "1001:1002",
        "/srv/app",
    ] {
        assert!(!rendered.contains(sensitive), "debug output leaked {sensitive}");
    }
    assert!(rendered.contains("[REDACTED]"));

    let unsupported = result
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .collect::<Vec<_>>();
    assert!(unsupported.iter().any(|outcome| {
        outcome.subject() == "runtime.podman.containers.web.Config"
            && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFP0002")
    }));
    for subject in [
        "runtime.podman.containers.web",
        "runtime.podman.containers.web.Networks.frontend",
        "runtime.podman.containers.web.Mounts[0]",
    ] {
        assert!(unsupported.iter().any(|outcome| outcome.subject() == subject));
    }
    assert!(
        unsupported
            .iter()
            .any(|outcome| outcome.subject() == "runtime.podman.networks.frontend")
    );
    assert!(
        unsupported
            .iter()
            .any(|outcome| outcome.subject() == "runtime.podman.volumes.data")
    );
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Error)
    );
    Ok(())
}

#[test]
fn podman_import_composes_native_losses_with_runtime_override_reconstruction() -> Result<(), String> {
    let source = fixture_source("podman-inspect-5-4", PlatformVersion::new(5, 4, 2))?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.import(&source);
    let application = result.application().ok_or("application expected")?;
    let service = application.services().first().ok_or("service expected")?.value();
    let group = application
        .service_groups()
        .first()
        .ok_or("service group expected")?
        .value();

    assert!(matches!(service.command().map(Sourced::value), Some(Command::Exec(arguments)) if arguments.len() == 2));
    assert_eq!(service.environment().len(), 1);
    assert_eq!(service.environment()[0].value().name().as_str(), "MODE");
    assert_eq!(group.name().as_str(), "app-pod");
    assert_eq!(group.members()[0].value().as_str(), "web");
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert!(service.working_directory().is_none());
    assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&true));
    assert!(matches!(
        service.environment()[0].value().value(),
        EnvironmentValue::Literal(value) if value.expose() == "production"
    ));
    for code in ["BFP0002", "BFR0001", "BFR0002", "BFR0004", "BFR0005"] {
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == code),
            "diagnostic {code} expected"
        );
    }
    assert!(!format!("{result:?}").contains("production"));
    assert!(!format!("{result:?}").contains("1001:1002"));
    Ok(())
}

#[test]
fn podman_importer_forwards_explicit_runtime_lifecycle_resolutions() -> Result<(), String> {
    let source = fixture_source("podman-inspect-5-4", PlatformVersion::new(5, 4, 2))?;
    let mut resolutions = RuntimeResolutions::new();
    resolutions
        .set_network_ownership(id("frontend")?, resolution(ResourceOwnership::External)?)
        .map_err(|error| error.to_string())?;
    resolutions
        .set_volume_ownership(id("data")?, resolution(ResourceOwnership::Application)?)
        .map_err(|error| error.to_string())?;
    resolutions
        .set_service_group_ownership(id("app-pod")?, resolution(ResourceOwnership::Application)?)
        .map_err(|error| error.to_string())?;
    let importer = importer(OverrideReconstruction::PreserveObservedState)?.with_resolutions(resolutions);
    assert_eq!(
        importer
            .resolutions()
            .network_ownership(&id("frontend")?)
            .map(Sourced::value),
        Some(&ResourceOwnership::External)
    );

    let result = importer.import(&source);
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
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0009"))
            .count(),
        3
    );
    Ok(())
}

#[test]
fn current_reviewed_podman_shape_is_accepted_and_additive_config_is_reported() -> Result<(), String> {
    let source = fixture_source("podman-inspect-6-0", PlatformVersion::new(6, 0, 2))?;
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    let snapshot = result.snapshot().ok_or("snapshot expected")?;
    let container = snapshot.containers().first().ok_or("container expected")?;
    assert_eq!(container.name().as_str(), "api");
    assert!(matches!(container.command(), Some(EffectiveCommand::Empty)));
    assert_eq!(container.environment(), Some([].as_slice()));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "runtime.podman.containers.api.Config"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFP0002")
    }));
    Ok(())
}

#[test]
fn malformed_document_fails_closed_without_echoing_the_payload() -> Result<(), String> {
    let source = PodmanInspectSource::new(
        id("example")?,
        PlatformVersion::new(5, 4, 0),
        PodmanInspectDocuments::new(
            "[{\"Config\":{\"Env\":[\"TOKEN=do-not-print\"]}",
            "[]",
            "[]",
            "[]",
            "[]",
        ),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    assert!(result.snapshot().is_none());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Invalid && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFP0001")
    }));
    let rendered = format!("{source:?} {result:?}");
    assert!(!rendered.contains("do-not-print"));
    assert!(rendered.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn non_boolean_read_only_root_state_fails_closed() -> Result<(), String> {
    let source = PodmanInspectSource::new(
        id("example")?,
        PlatformVersion::new(5, 4, 0),
        PodmanInspectDocuments::new(
            r#"[{"Id":"container-id","Image":"image-id","ImageName":"example.invalid/web:1","Name":"web","Config":{"Image":"example.invalid/web:1"},"HostConfig":{"ReadonlyRootfs":"yes"}}]"#,
            r#"[{"Id":"image-id"}]"#,
            "[]",
            "[]",
            "[]",
        ),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    assert!(result.snapshot().is_none());
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFP0001" && diagnostic.severity() == Severity::Error })
    );
    Ok(())
}

#[test]
fn decoder_rejects_versions_outside_the_finite_reviewed_range() -> Result<(), String> {
    for version in [PlatformVersion::new(5, 3, 9), PlatformVersion::new(6, 0, 3)] {
        let source = PodmanInspectSource::new(
            id("example")?,
            version,
            PodmanInspectDocuments::new("[]", "[]", "[]", "[]", "[]"),
        );
        let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
        assert!(result.snapshot().is_none());
        assert_eq!(result.diagnostics()[0].code().as_str(), "BFP0003");
        assert_eq!(result.diagnostics()[0].severity(), Severity::Error);
    }
    Ok(())
}

fn fixture_source(name: &str, version: PlatformVersion) -> Result<PodmanInspectSource, String> {
    let root = fixture_root(name);
    Ok(PodmanInspectSource::new(
        id("example")?,
        version,
        PodmanInspectDocuments::new(
            read(&root, "containers.json")?,
            read(&root, "images.json")?,
            read(&root, "networks.json")?,
            read(&root, "volumes.json")?,
            read(&root, "pods.json")?,
        ),
    ))
}

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/runtime")
        .join(name)
}

fn read(root: &Path, name: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(name)).map_err(|error| error.to_string())
}

fn importer(policy: OverrideReconstruction) -> Result<PodmanImporter, String> {
    PodmanImporter::new(policy).map_err(|error| error.to_string())
}

fn resolution(ownership: ResourceOwnership) -> Result<Sourced<ResourceOwnership>, String> {
    Ok(Sourced::from_source(
        ownership,
        Provenance::user_override(SourceId::new("decision:test-lifecycle").map_err(|error| error.to_string())?),
    ))
}

fn id(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}
