//! Fixture-driven Docker inspect boundary tests.

use std::path::{Path, PathBuf};

use boxferry_docker::{DockerApiVersion, DockerImporter, DockerInspectDocuments, DockerInspectSource};
use boxferry_engine::{ConversionKind, DiagnosticCode, ImportAdapter, Severity};
use boxferry_model::{
    Command, EnvironmentValue, HealthcheckCommand, Identifier, MountSource, Protocol, Provenance, ResourceOwnership,
    RestartPolicy, SelinuxRelabel, SourceId, Sourced,
};
use boxferry_runtime::{EffectiveCommand, OverrideReconstruction, RuntimeImplementation, RuntimeResolutions};

#[test]
fn regular_healthchecks_are_decoded_and_compared_with_image_defaults() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 55),
        DockerInspectDocuments::new(
            r#"[{"Id":"container-id","Image":"image-id","Name":"/web","Config":{"Image":"example.invalid/web:1","Healthcheck":{"Test":["CMD-SHELL","check --token docker-health-secret"],"Interval":10000000000,"Timeout":2000000000,"Retries":3,"StartPeriod":5000000000,"StartInterval":1000000000}}}]"#,
            r#"[{"Id":"image-id","Config":{"Healthcheck":{"Test":["CMD-SHELL","check --token docker-health-secret"],"Interval":30000000000,"Timeout":2000000000,"Retries":3,"StartPeriod":5000000000,"StartInterval":1000000000}}}]"#,
            "[]",
            "[]",
        ),
    );
    let importer = importer(OverrideReconstruction::InferImageOverrides)?;
    let decoded = importer.decode(&source);
    let snapshot = decoded.snapshot().ok_or("snapshot expected")?;
    let healthcheck = snapshot.containers()[0].healthcheck().ok_or("health check expected")?;
    assert!(matches!(healthcheck.command(), Some(HealthcheckCommand::Shell(_))));
    assert_eq!(
        healthcheck.interval().map(boxferry_model::HealthcheckDuration::as_str),
        Some("10s")
    );
    assert_eq!(
        healthcheck.timeout().map(boxferry_model::HealthcheckDuration::as_str),
        Some("2s")
    );
    assert_eq!(
        healthcheck.retries().map(boxferry_model::HealthcheckRetries::as_str),
        Some("3")
    );
    assert_eq!(
        healthcheck
            .start_period()
            .map(boxferry_model::HealthcheckDuration::as_str),
        Some("5s")
    );
    assert_eq!(
        healthcheck
            .start_interval()
            .map(boxferry_model::HealthcheckDuration::as_str),
        Some("1s")
    );
    assert!(!format!("{decoded:?}").contains("docker-health-secret"));

    let result = importer.import(&source);
    let healthcheck = result
        .application()
        .and_then(|application| application.services().first())
        .and_then(|service| service.value().healthcheck())
        .ok_or("retained health-check override expected")?;
    assert_eq!(
        healthcheck.value().interval().map(|value| value.value().as_str()),
        Some("10s")
    );
    assert!(healthcheck.value().command().is_none());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.healthcheck.interval"
            && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFR0002")
    }));
    Ok(())
}

#[test]
fn decodes_the_reviewed_docker_restart_policy_object() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new(
            r#"[{"Id":"container-id","Image":"image-id","Name":"/web","Config":{"Image":"example.invalid/web:1"},"HostConfig":{"RestartPolicy":{"Name":"on-failure","MaximumRetryCount":4}}}]"#,
            r#"[{"Id":"image-id"}]"#,
            "[]",
            "[]",
        ),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
    let policy = result
        .snapshot()
        .and_then(|snapshot| snapshot.containers().first())
        .and_then(boxferry_runtime::ContainerObservation::restart_policy)
        .ok_or("restart policy expected")?;

    assert!(matches!(
        policy,
        RestartPolicy::OnFailure {
            maximum_retries: Some(value)
        } if value.get() == 4
    ));
    assert!(
        !result
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "runtime.docker.containers.web.HostConfig" })
    );
    Ok(())
}

#[test]
fn invalid_docker_restart_policy_objects_fail_closed() -> Result<(), String> {
    for restart_policy in [
        r#"{"Name":"always","MaximumRetryCount":1}"#,
        r#"{"Name":"sometimes","MaximumRetryCount":0}"#,
        r#"{"Name":"on-failure","MaximumRetryCount":-1}"#,
        r#""always""#,
    ] {
        let containers = format!(
            r#"[{{"Id":"container-id","Image":"image-id","Name":"/web","Config":{{"Image":"example.invalid/web:1"}},"HostConfig":{{"RestartPolicy":{restart_policy}}}}}]"#
        );
        let source = DockerInspectSource::new(
            id("example")?,
            DockerApiVersion::new(1, 40),
            DockerInspectDocuments::new(containers, r#"[{"Id":"image-id"}]"#, "[]", "[]"),
        );
        let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
        assert!(result.snapshot().is_none());
        assert!(
            result.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFD0001" && diagnostic.severity() == Severity::Error
            })
        );
    }
    Ok(())
}

#[test]
fn invalid_docker_metadata_label_maps_fail_closed() -> Result<(), String> {
    for labels in [r#"{"":"value"}"#, r#"{"com.example.invalid":1}"#] {
        let containers = format!(
            r#"[{{"Id":"container-id","Image":"image-id","Name":"/web","Config":{{"Image":"example.invalid/web:1","Labels":{labels}}}}}]"#
        );
        let source = DockerInspectSource::new(
            id("example")?,
            DockerApiVersion::new(1, 40),
            DockerInspectDocuments::new(containers, r#"[{"Id":"image-id"}]"#, "[]", "[]"),
        );
        let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
        assert!(result.snapshot().is_none());
        assert!(
            result.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFD0001" && diagnostic.severity() == Severity::Error
            })
        );
    }
    Ok(())
}

#[test]
fn start_interval_is_rejected_before_the_docker_api_introduced_it() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new(
            r#"[{"Id":"container-id","Image":"image-id","Name":"/web","Config":{"Image":"example.invalid/web:1","Healthcheck":{"Test":["CMD","true"],"StartInterval":1000000000}}}]"#,
            r#"[{"Id":"image-id"}]"#,
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
            .any(|diagnostic| diagnostic.code().as_str() == "BFD0001")
    );
    Ok(())
}

#[test]
fn api_1_40_decodes_effective_state_and_relationships_without_raw_id_leaks() -> Result<(), String> {
    let source = fixture_source("docker-inspect-api-1-40", DockerApiVersion::new(1, 40))?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.decode(&source);
    let snapshot = result.snapshot().ok_or("snapshot expected")?;

    assert_eq!(snapshot.implementation(), &RuntimeImplementation::Docker);
    assert_eq!(snapshot.images().len(), 1);
    assert_eq!(snapshot.networks().len(), 1);
    assert_eq!(snapshot.volumes().len(), 1);
    assert!(snapshot.pods().is_empty());

    let container = snapshot.containers().first().ok_or("container expected")?;
    assert_eq!(container.name().as_str(), "web");
    assert_eq!(
        container.image().map(boxferry_model::ImageReference::as_str),
        Some("registry.example.invalid/team/web:1.2@sha256:abcd")
    );
    assert!(container.image_source_id().is_some());
    assert_eq!(container.networks()[0].aliases(), ["web", "public-api"]);
    assert_eq!(container.ports()[0].container(), 8080);
    assert_eq!(container.ports()[0].published(), Some(18080));
    assert_eq!(container.ports()[0].host_address(), Some("127.0.0.1"));
    assert_eq!(container.ports()[0].protocol(), &Protocol::Tcp);
    assert!(matches!(container.mounts()[0].source(), MountSource::Volume(name) if name.as_str() == "data"));
    assert!(container.mounts()[1].read_only());
    assert_eq!(container.mounts()[1].selinux_relabel(), Some(SelinuxRelabel::Private));
    assert!(matches!(container.command(), Some(EffectiveCommand::Exec(arguments)) if arguments.len() == 3));
    assert_eq!(container.labels().map(<[_]>::len), Some(2));
    assert_eq!(
        container
            .labels()
            .and_then(|labels| labels.first())
            .map(|label| label.name().as_str()),
        Some("com.example.purpose")
    );
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
    assert!(matches!(
        container.restart_policy(),
        Some(RestartPolicy::OnFailure {
            maximum_retries: Some(value)
        }) if value.get() == 4
    ));

    let rendered = format!("{source:?} {result:?}");
    for sensitive in [
        "MODE=production",
        "--production",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "0123456789ab",
        "image-runtime-id-web",
        "network-runtime-id-frontend",
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
    for subject in [
        "runtime.docker.containers.web.Config",
        "runtime.docker.containers.web.Mounts[0]",
        "runtime.docker.images.0.Config",
        "runtime.docker.networks.frontend",
        "runtime.docker.volumes.data",
    ] {
        assert!(unsupported.iter().any(|outcome| outcome.subject() == subject));
    }
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Error)
    );
    Ok(())
}

#[test]
fn docker_import_composes_native_losses_with_runtime_reconstruction() -> Result<(), String> {
    let source = fixture_source("docker-inspect-api-1-40", DockerApiVersion::new(1, 40))?;
    let result = importer(OverrideReconstruction::InferImageOverrides)?.import(&source);
    let application = result.application().ok_or("application expected")?;
    let service = application.services().first().ok_or("service expected")?.value();

    assert!(matches!(service.command().map(Sourced::value), Some(Command::Exec(arguments)) if arguments.len() == 3));
    assert_eq!(service.environment().len(), 1);
    assert_eq!(service.labels().len(), 1);
    assert_eq!(service.labels()[0].value().name().as_str(), "com.example.purpose");
    assert_eq!(service.labels()[0].value().value().expose(), "fixture");
    assert_eq!(service.environment()[0].value().name().as_str(), "MODE");
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert!(service.working_directory().is_none());
    assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&true));
    assert!(matches!(
        service.environment()[0].value().value(),
        EnvironmentValue::Literal(value) if value.expose() == "production"
    ));
    for code in ["BFD0002", "BFR0001", "BFR0002", "BFR0004"] {
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
fn docker_importer_forwards_explicit_runtime_lifecycle_resolutions() -> Result<(), String> {
    let source = fixture_source("docker-inspect-api-1-40", DockerApiVersion::new(1, 40))?;
    let mut resolutions = RuntimeResolutions::new();
    resolutions
        .set_network_ownership(id("frontend")?, resolution(ResourceOwnership::External)?)
        .map_err(|error| error.to_string())?;
    resolutions
        .set_volume_ownership(id("data")?, resolution(ResourceOwnership::Application)?)
        .map_err(|error| error.to_string())?;
    let importer = importer(OverrideReconstruction::PreserveObservedState)?.with_resolutions(resolutions);
    assert_eq!(
        importer
            .resolutions()
            .volume_ownership(&id("data")?)
            .map(Sourced::value),
        Some(&ResourceOwnership::Application)
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
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0009"))
            .count(),
        2
    );
    Ok(())
}

#[test]
fn current_reviewed_api_shape_is_accepted_and_additive_config_is_reported() -> Result<(), String> {
    let source = fixture_source("docker-inspect-api-1-55", DockerApiVersion::new(1, 55))?;
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    let snapshot = result.snapshot().ok_or("snapshot expected")?;
    let container = snapshot.containers().first().ok_or("container expected")?;
    assert_eq!(container.name().as_str(), "api");
    assert!(matches!(container.command(), Some(EffectiveCommand::Empty)));
    assert_eq!(container.environment(), Some([].as_slice()));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "runtime.docker.containers.api.Config"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFD0002")
    }));
    Ok(())
}

#[test]
fn api_1_45_and_newer_preserve_exact_submitted_aliases() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 45),
        DockerInspectDocuments::new(
            r#"[{
                "Id":"0123456789abcdef",
                "Image":"image-id",
                "Name":"/web",
                "Config":{"Image":"example.invalid/web:1"},
                "NetworkSettings":{"Networks":{"frontend":{"Aliases":["0123456789ab"]}}}
            }]"#,
            r#"[{"Id":"image-id"}]"#,
            r#"[{"Name":"frontend"}]"#,
            "[]",
        ),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
    let snapshot = result.snapshot().ok_or("snapshot expected")?;

    assert_eq!(snapshot.containers()[0].networks()[0].aliases(), ["0123456789ab"]);
    Ok(())
}

#[test]
fn missing_related_documents_are_reported_without_discarding_the_container() -> Result<(), String> {
    let root = fixture_root("docker-inspect-api-1-40");
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new(read(&root, "containers.json")?, "[]", "[]", "[]"),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    assert!(result.snapshot().is_some());
    assert_eq!(
        result
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFD0004")
            .count(),
        3
    );
    Ok(())
}

#[test]
fn malformed_document_fails_closed_without_echoing_the_payload() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new("[{\"Config\":{\"Env\":[\"TOKEN=do-not-print\"]}", "[]", "[]", "[]"),
    );
    let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

    assert!(result.snapshot().is_none());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Invalid && outcome.diagnostic().map(DiagnosticCode::as_str) == Some("BFD0001")
    }));
    let rendered = format!("{source:?} {result:?}");
    assert!(!rendered.contains("do-not-print"));
    assert!(rendered.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn container_without_a_reconstructable_image_reference_fails_closed() -> Result<(), String> {
    for containers in [
        r#"[{"Id":"container-id","Image":"image-id","Name":"/web"}]"#,
        r#"[{"Id":"container-id","Image":"image-id","Name":"/web","Config":{}}]"#,
    ] {
        let source = DockerInspectSource::new(
            id("example")?,
            DockerApiVersion::new(1, 40),
            DockerInspectDocuments::new(containers, "[]", "[]", "[]"),
        );
        let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);

        assert!(result.snapshot().is_none());
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "BFD0001")
        );
    }
    Ok(())
}

#[test]
fn non_boolean_read_only_root_state_fails_closed() -> Result<(), String> {
    let source = DockerInspectSource::new(
        id("example")?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new(
            r#"[{"Id":"container-id","Image":"image-id","Name":"/web","Config":{"Image":"example.invalid/web:1"},"HostConfig":{"ReadonlyRootfs":"yes"}}]"#,
            r#"[{"Id":"image-id"}]"#,
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
            .any(|diagnostic| { diagnostic.code().as_str() == "BFD0001" && diagnostic.severity() == Severity::Error })
    );
    Ok(())
}

#[test]
fn decoder_rejects_api_versions_outside_the_finite_reviewed_range() -> Result<(), String> {
    for version in [DockerApiVersion::new(1, 39), DockerApiVersion::new(1, 56)] {
        let source = DockerInspectSource::new(
            id("example")?,
            version,
            DockerInspectDocuments::new("[]", "[]", "[]", "[]"),
        );
        let result = importer(OverrideReconstruction::PreserveObservedState)?.decode(&source);
        assert!(result.snapshot().is_none());
        assert_eq!(result.diagnostics()[0].code().as_str(), "BFD0003");
        assert_eq!(result.diagnostics()[0].severity(), Severity::Error);
    }
    Ok(())
}

fn fixture_source(name: &str, version: DockerApiVersion) -> Result<DockerInspectSource, String> {
    let root = fixture_root(name);
    Ok(DockerInspectSource::new(
        id("example")?,
        version,
        DockerInspectDocuments::new(
            read(&root, "containers.json")?,
            read(&root, "images.json")?,
            read(&root, "networks.json")?,
            read(&root, "volumes.json")?,
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

fn importer(policy: OverrideReconstruction) -> Result<DockerImporter, String> {
    DockerImporter::new(policy).map_err(|error| error.to_string())
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
