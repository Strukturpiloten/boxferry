//! Supported facade API exercised as an external crate would use it.

use boxferry::{Application, Identifier, InMemoryAdapter, LossPolicy, PlatformVersion, TargetProfile, convert};

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

#[cfg(feature = "compose")]
#[test]
fn facade_exposes_the_compose_import_adapter_additively() -> Result<(), String> {
    use boxferry::compose::compose_lens::{
        loader::{DocumentInput, DocumentOrigin, LoadedProject},
        merge::merge_project,
        source::SourceId as ComposeSourceId,
    };
    use boxferry::{ComposeImporter, ComposeSource, ImportAdapter, SourceId};

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
