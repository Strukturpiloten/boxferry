//! Supported facade API exercised as an external crate would use it.

use boxferry::{Application, Identifier, InMemoryAdapter, LossPolicy, PlatformVersion, TargetProfile, convert};

#[test]
fn facade_exposes_report_dto_without_cli_features() {
    use boxferry::report::{ConversionReport, ExitCategory, FixFirst, ReportStatus, VersionBounds, redact_text};

    let mut report = ConversionReport::new(
        "test",
        "compose",
        "quadlet",
        VersionBounds {
            minimum: "5.4".into(),
            maximum: "6.0".into(),
        },
    );
    report.status = ReportStatus::Blocked;
    report.exit_category = ExitCategory::PolicyBlocked;
    report.fix_first = Some(FixFirst {
        code: "BFC0001".into(),
        name: "compose-model-invalid".into(),
        description: "The Compose model is invalid.".into(),
        help: "Correct the Compose value.".into(),
        next_step: "Rerun BoxFerry.".into(),
    });
    assert_eq!(redact_text("databasePassword", "canary", false).0, "<redacted>");
    assert!(report.review_required);
    assert_eq!(
        report.fix_first.as_ref().map(|guidance| guidance.code.as_str()),
        Some("BFC0001")
    );
}

#[test]
fn facade_converts_through_public_adapter_contracts() -> Result<(), String> {
    let application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    let adapter = InMemoryAdapter::exact("rendered target".to_owned());
    let target =
        TargetProfile::new("podman", PlatformVersion::new(5, 4, 0), None).map_err(|error| error.to_string())?;

    let result =
        convert(&adapter, &application, &adapter, &target, LossPolicy::ExactOnly).map_err(|error| error.to_string())?;
    assert_eq!(result.output().map(String::as_str), Some("rendered target"));
    Ok(())
}

#[test]
fn facade_exposes_raw_preserving_neutral_host_mappings() -> Result<(), String> {
    use boxferry::{HostAddress, HostAddressKind, HostMapping, Service, Sourced};

    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    let mapping = HostMapping::new(
        Identifier::new("host.docker.internal").map_err(|error| error.to_string())?,
        HostAddress::new("host-gateway").map_err(|error| error.to_string())?,
    );
    service.add_host_mapping(Sourced::generated(mapping));

    let mapping = service.host_mappings().first().ok_or("host mapping expected")?.value();
    assert_eq!(mapping.hostname().as_str(), "host.docker.internal");
    assert_eq!(mapping.address().raw(), "host-gateway");
    assert_eq!(mapping.address().kind(), HostAddressKind::HostGateway);
    Ok(())
}

#[test]
fn facade_reexports_image_artifact_model_types() -> Result<(), String> {
    use boxferry::{
        BuildAttestation, BuildContext, BuildSettingValues, BuildSourceDeclaration, BuildSyntax, ImageAcquisition,
        ImageAcquisitionSetting, ImageArtifactAssignment, ImageBuild, ImageBuildSetting, ProtectedString,
        SourceBuildSecret, SourceBuildSetting, Sourced,
    };

    let identifier = Identifier::new("web-build").map_err(|error| error.to_string())?;
    let assignment = ImageArtifactAssignment::new(
        ProtectedString::plain("APP_MODE"),
        Some(ProtectedString::plain("production")),
    );
    let values = BuildSettingValues::new(BuildSyntax::Mapping, vec![Sourced::generated(assignment)]);
    let declaration =
        BuildSourceDeclaration::Structured(vec![Sourced::generated(SourceBuildSetting::Arguments(values))]);
    let mut build = ImageBuild::new(identifier.clone());
    build.set_source_declaration(Sourced::generated(declaration));
    build.set_settings(vec![Sourced::generated(ImageBuildSetting::ImageTags(
        BuildSettingValues::new(
            BuildSyntax::Repeated,
            vec![Sourced::generated(ProtectedString::plain("example.invalid/web:1"))],
        ),
    ))]);

    let mut acquisition = ImageAcquisition::new(identifier);
    acquisition.set_settings(vec![Sourced::generated(ImageAcquisitionSetting::Image(
        ProtectedString::plain("example.invalid/base:1"),
    ))]);

    let context = BuildContext::new(ProtectedString::plain("source"), ProtectedString::plain("./web"));
    let secret = SourceBuildSecret::new(ProtectedString::sensitive("build-token"));
    assert_eq!(context.name().expose(), "source");
    assert!(secret.source().is_sensitive());
    assert!(matches!(
        BuildAttestation::Boolean(true),
        BuildAttestation::Boolean(true)
    ));
    assert_eq!(
        build.source_declaration().map(|value| value.value().syntax()),
        Some(BuildSyntax::Structured)
    );
    assert_eq!(acquisition.settings().map(<[_]>::len), Some(1));
    assert!(!format!("{secret:?}").contains("build-token"));
    Ok(())
}

#[test]
fn facade_exposes_environment_file_declarations_without_filesystem_access() -> Result<(), String> {
    use boxferry::{EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax, ProtectedString, Service, Sourced};

    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    let mut environment_file = EnvironmentFile::new(
        ProtectedString::sensitive("./production.env"),
        EnvironmentFileSyntax::Long,
    )
    .map_err(|error| error.to_string())?;
    environment_file.set_required(Sourced::generated(true));
    environment_file.set_format(Sourced::generated(EnvironmentFileFormat::Raw));
    service.add_environment_file(Sourced::generated(environment_file));

    let declaration = service.environment_files().first().ok_or("environment file expected")?;
    assert!(declaration.value().is_required());
    assert!(matches!(
        declaration.value().format().map(Sourced::value),
        Some(EnvironmentFileFormat::Raw)
    ));
    assert!(!format!("{service:?}").contains("production.env"));
    Ok(())
}

#[test]
fn facade_exposes_neutral_execution_identity_and_context() -> Result<(), String> {
    use boxferry::{ProtectedString, Service, Sourced};

    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    service.set_user(Sourced::generated(ProtectedString::plain("1001")));
    service.set_group(Sourced::generated(ProtectedString::plain("1002")));
    service.set_user_namespace(Sourced::generated(ProtectedString::plain("keep-id")));
    service.add_supplementary_group(Sourced::generated(ProtectedString::plain("audio")));
    service.set_working_directory(Sourced::generated(ProtectedString::plain("/srv/app")));
    service.set_read_only_root_filesystem(Sourced::generated(true));

    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(service.supplementary_groups().len(), 1);
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/srv/app")
    );
    assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&true));
    Ok(())
}

#[test]
fn facade_exposes_neutral_config_and_secret_contract() -> Result<(), String> {
    use boxferry::{
        Config, ConfigMaterial, ProtectedString, ResourceGrant, ResourceGrantSyntax, ResourceOwnership, Secret,
        SecretMaterial, Service, Sourced,
    };

    let mut application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    let mut config = Config::new(
        Identifier::new("settings").map_err(|error| error.to_string())?,
        ResourceOwnership::Application,
    );
    config.set_material(Sourced::generated(ConfigMaterial::File(ProtectedString::plain(
        "./settings.toml",
    ))));
    application
        .add_config(Sourced::generated(config))
        .map_err(|error| error.to_string())?;

    let mut secret = Secret::new(
        Identifier::new("database-password").map_err(|error| error.to_string())?,
        ResourceOwnership::External,
    );
    secret.set_runtime_name(Sourced::generated(ProtectedString::plain("production-password")));
    secret.set_material(Sourced::generated(SecretMaterial::Environment(
        ProtectedString::sensitive("DATABASE_PASSWORD"),
    )));
    application
        .add_secret(Sourced::generated(secret))
        .map_err(|error| error.to_string())?;

    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    let mut grant = ResourceGrant::new(ProtectedString::plain("database-password"), ResourceGrantSyntax::Long)
        .map_err(|error| error.to_string())?;
    grant.set_target(Sourced::generated(ProtectedString::plain("/run/secrets/password")));
    service.add_secret_grant(Sourced::generated(grant));
    application
        .add_service(Sourced::generated(service))
        .map_err(|error| error.to_string())?;

    assert_eq!(application.configs().len(), 1);
    assert_eq!(application.secrets().len(), 1);
    assert_eq!(application.services()[0].value().secret_grants().len(), 1);
    assert!(!format!("{application:?}").contains("DATABASE_PASSWORD"));
    Ok(())
}

#[test]
fn facade_distinguishes_runtime_observation_provenance() -> Result<(), String> {
    use boxferry::{Provenance, ProvenanceKind, SourceId};

    let provenance =
        Provenance::runtime_observation(SourceId::new("runtime:container/web").map_err(|error| error.to_string())?);
    assert_eq!(provenance.kind(), ProvenanceKind::RuntimeObservation);
    assert_eq!(provenance.span(), None);
    Ok(())
}

#[test]
fn facade_exposes_neutral_structural_service_groups() -> Result<(), String> {
    use boxferry::{ResourceOwnership, Service, ServiceGroup, Sourced};

    let mut application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    application
        .add_service(Sourced::generated(Service::new(
            Identifier::new("web").map_err(|error| error.to_string())?,
        )))
        .map_err(|error| error.to_string())?;
    let mut group = ServiceGroup::new(
        Identifier::new("observed-group").map_err(|error| error.to_string())?,
        ResourceOwnership::Uncertain,
    );
    group
        .add_member(Sourced::generated(
            Identifier::new("web").map_err(|error| error.to_string())?,
        ))
        .map_err(|error| error.to_string())?;
    application
        .add_service_group(Sourced::generated(group))
        .map_err(|error| error.to_string())?;

    assert_eq!(
        application.service_groups()[0].value().members()[0].value().as_str(),
        "web"
    );
    Ok(())
}

#[cfg(feature = "runtime")]
#[test]
fn facade_exposes_runtime_reconstruction_additively() -> Result<(), String> {
    use boxferry::{
        ContainerObservation, EffectiveCommand, ImageReference, ImportAdapter, OverrideReconstruction,
        RuntimeImplementation, RuntimeImporter, RuntimeMetadataLabel, RuntimeSnapshot, SourceId,
    };

    let mut container = ContainerObservation::new(
        SourceId::new("runtime:podman:container:web").map_err(|error| error.to_string())?,
        Identifier::new("web").map_err(|error| error.to_string())?,
    );
    container.set_image(
        ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
        None,
    );
    container.set_command(EffectiveCommand::Empty);
    container.set_environment(Vec::new());
    container.set_labels(vec![RuntimeMetadataLabel::new(
        Identifier::new("com.example.role").map_err(|error| error.to_string())?,
        "web",
    )]);

    let mut snapshot = RuntimeSnapshot::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        RuntimeImplementation::Podman,
    );
    snapshot.add_container(container).map_err(|error| error.to_string())?;
    let importer =
        RuntimeImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let result = importer.import(&snapshot);

    assert_eq!(
        result.application().map(|application| application.services().len()),
        Some(1)
    );
    assert_eq!(
        result
            .application()
            .and_then(|application| application.services().first())
            .map(|service| service.value().labels().len()),
        Some(1)
    );
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "application.reconstruction")
    );
    Ok(())
}

#[cfg(feature = "compose")]
#[test]
fn facade_exposes_compose_adapters_and_specification_target_additively() -> Result<(), String> {
    use boxferry::compose::compose_lens::{
        loader::{DocumentInput, DocumentOrigin, LoadedProject},
        merge::merge_project,
        source::SourceId as ComposeSourceId,
    };
    use boxferry::{
        COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeImporter, ComposeSource,
        ImportAdapter, SourceId,
    };

    assert_eq!(COMPOSE_SPECIFICATION_TARGET, "compose-specification");
    assert_eq!(COMPOSE_SPECIFICATION_PROFILE_REVISION.to_string(), "1.0.0");

    let compose_source_id = ComposeSourceId::new(1);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("compose.yaml", "."),
        concat!(
            "services:\n",
            "  web:\n",
            "    image: example.invalid/web:1\n",
            "    extra_hosts:\n",
            "      - host.docker.internal=host-gateway\n",
        ),
    )])
    .map_err(|error| error.to_string())?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged Compose project expected")?.clone();
    let source = ComposeSource::new(project, Identifier::new("example").map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?
        .with_source_id(
            compose_source_id,
            SourceId::new("compose.yaml").map_err(|error| error.to_string())?,
        );
    let importer = ComposeImporter::new().map_err(|error| error.to_string())?;
    let result = importer.import(&source);

    assert_eq!(
        result.application().map(|application| application.services().len()),
        Some(1)
    );
    assert_eq!(
        result
            .application()
            .and_then(|application| application.services().first())
            .map(|service| service.value().host_mappings().len()),
        Some(1)
    );
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    Ok(())
}

#[cfg(feature = "quadlet")]
#[test]
fn facade_exposes_quadlet_parse_diagnostics() -> Result<(), String> {
    use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
    use boxferry::{QuadletDocumentInput, QuadletParseDiagnosticOrigin, QuadletParseDiagnosticSeverity, QuadletSource};

    let error = QuadletSource::parse(
        Identifier::new("example").map_err(|error| error.to_string())?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(41),
            "[Container]\nImage=\n",
        )],
    )
    .err()
    .ok_or("invalid Quadlet source should fail")?;
    let diagnostic = error
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.origin() == QuadletParseDiagnosticOrigin::Model)
        .ok_or("native model diagnostic expected")?;
    assert_eq!(diagnostic.code(), "QLM0005");
    assert_eq!(diagnostic.severity(), QuadletParseDiagnosticSeverity::Error);
    assert_eq!(diagnostic.labels()[0].source_id(), 41);
    Ok(())
}

#[cfg(feature = "quadlet")]
#[test]
fn facade_exposes_the_quadlet_export_adapter_additively() -> Result<(), String> {
    use boxferry::{
        ExportAdapter, Healthcheck, HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress,
        HostMapping, ImageReference, ProtectedString, QuadletExporter, QuadletGroupingPolicy, Service, Sourced,
    };

    let mut application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    service.set_image(Sourced::generated(
        ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
    ));
    service.add_host_mapping(Sourced::generated(HostMapping::new(
        Identifier::new("host.docker.internal").map_err(|error| error.to_string())?,
        HostAddress::new("host-gateway").map_err(|error| error.to_string())?,
    )));
    let mut healthcheck = Healthcheck::new();
    healthcheck.set_command(Sourced::generated(HealthcheckCommand::Shell(ProtectedString::plain(
        "curl --fail http://127.0.0.1/health",
    ))));
    healthcheck.set_interval(Sourced::generated(
        HealthcheckDuration::new("30s").map_err(|error| error.to_string())?,
    ));
    healthcheck.set_retries(Sourced::generated(
        HealthcheckRetries::new("3").map_err(|error| error.to_string())?,
    ));
    service.set_healthcheck(Sourced::generated(healthcheck));
    application
        .add_service(Sourced::generated(service))
        .map_err(|error| error.to_string())?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )
    .map_err(|error| error.to_string())?;
    let exporter = QuadletExporter::new()
        .map_err(|error| error.to_string())?
        .with_bind_source_mapping("~/data", "%h/data")
        .map_err(|error| error.to_string())?;
    assert_eq!(exporter.grouping_policy(), QuadletGroupingPolicy::SeparateContainers);
    assert_eq!(exporter.bind_source_mapping("~/data"), Some("%h/data"));
    let output = exporter
        .plan(&application, &target)
        .map_err(|error| error.to_string())?
        .authorize(LossPolicy::ExactOnly);

    assert_eq!(
        output
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "HealthCmd=[\"CMD-SHELL\",\"curl --fail http://127.0.0.1/health\"]\n",
            "HealthInterval=30s\n",
            "HealthRetries=3\n",
            "AddHost=host.docker.internal:host-gateway\n",
        ))
    );
    Ok(())
}
