//! Public Quadlet exporter behavior and target boundaries.

use std::error::Error;

use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile};
use boxferry_model::{
    Application, Command, Config, ConfigMaterial, EnvironmentValue, EnvironmentVariable, Healthcheck,
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress, HostMapping, Identifier, ImageReference,
    Mount, MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, ResourceGrant,
    ResourceGrantSyntax, ResourceOwnership, Secret, SecretMaterial, SelinuxRelabel, Service, ServiceDependency,
    ServiceDependencyCondition, ServiceGroup, SourceId, Sourced, Volume,
};
use boxferry_quadlet::{QuadletExporter, QuadletGroupingPolicy};

#[test]
fn exports_the_exact_first_conversion_subset_and_resolves_native_references() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("example")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;
    application.add_volume(sourced(Volume::new(id("data")?, ResourceOwnership::Application))?)?;

    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse(
        "registry.example:5000/team/web:1.3@sha256:fedcba",
    )?)?);
    service.set_command(sourced(Command::Exec(vec![
        ProtectedString::plain("php"),
        ProtectedString::plain("-v"),
    ]))?);
    service.set_user(sourced(ProtectedString::sensitive("1001"))?);
    service.set_group(sourced(ProtectedString::plain("1002"))?);
    service.set_user_namespace(sourced(ProtectedString::plain("keep-id"))?);
    service.add_supplementary_group(sourced(ProtectedString::plain("audio"))?);
    service.add_supplementary_group(sourced(ProtectedString::plain("44"))?);
    service.set_working_directory(sourced(ProtectedString::plain("/srv/app"))?);
    service.set_read_only_root_filesystem(sourced(true)?);
    service.set_healthcheck(sourced(exact_healthcheck()?)?);
    service.add_environment(sourced(EnvironmentVariable::new(
        id("APP_ENV")?,
        EnvironmentValue::Literal(ProtectedString::sensitive("production")),
    ))?);
    service.add_host_mapping(sourced(HostMapping::new(
        id("host.docker.internal")?,
        HostAddress::new("host-gateway")?,
    ))?);
    service.add_host_mapping(sourced(HostMapping::new(id("ipv6")?, HostAddress::new("::1")?))?);
    service.add_port(sourced(Port::new(
        80,
        Some(8080),
        Some("127.0.0.1".to_owned()),
        Protocol::Tcp,
    )?)?);
    let mut data = Mount::new(MountSource::Volume(id("data")?), "/var/lib/data", true)?;
    data.set_selinux_relabel(SelinuxRelabel::Private);
    service.add_mount(sourced(data)?);
    let mut config = Mount::new(MountSource::HostPath("/srv/config".to_owned()), "/etc/config", false)?;
    config.set_selinux_relabel(SelinuxRelabel::Shared);
    service.add_mount(sourced(config)?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath("%h/.config/example".to_owned()),
        "/home/config",
        true,
    )?)?);
    service.add_network(sourced(NetworkAttachment::new(id("frontend")?, Vec::new()))?);
    application.add_service(sourced(service)?)?;

    let exporter = QuadletExporter::new()?;
    let plan = exporter.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        plan.outcomes()
    );
    let output = plan.authorize(LossPolicy::ExactOnly);
    let output = output.output().ok_or("exact output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["frontend.network", "data.volume", "web.container"]
    );
    assert_eq!(
        output.file("frontend.network").map(boxferry_quadlet::QuadletFile::text),
        Some("[Network]\nNetworkName=frontend\n")
    );
    assert_eq!(
        output.file("data.volume").map(boxferry_quadlet::QuadletFile::text),
        Some("[Volume]\nVolumeName=data\n")
    );
    assert_eq!(
        output.file("web.container").map(boxferry_quadlet::QuadletFile::text),
        Some(exact_first_conversion_container())
    );
    assert!(output.document_set().is_valid());
    assert!(output.document_set().graph().is_complete());
    assert_eq!(output.document_set().graph().edges().len(), 2);
    assert!(!format!("{output:?}").contains("production"));
    assert!(!format!("{output:?}").contains("1001"));
    Ok(())
}

#[test]
fn emits_explicit_false_read_only_and_reports_unsafe_working_directory() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("execution-context")?);
    let mut service = image_service("web")?;
    service.set_working_directory(sourced(ProtectedString::plain("/srv/space path"))?);
    service.set_read_only_root_filesystem(sourced(false)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.working_directory" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.read_only_root_filesystem" && outcome.kind() == ConversionKind::Exact
    }));
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    assert_eq!(
        plan.authorize(LossPolicy::AllowPartial)
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "ReadOnly=false\n",
        ))
    );
    Ok(())
}

#[test]
fn reports_a_named_primary_group_instead_of_claiming_an_exact_quadlet_mapping() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("named-primary-group")?);
    let mut service = image_service("web")?;
    service.set_group(sourced(ProtectedString::plain("operators"))?);
    service.add_supplementary_group(sourced(ProtectedString::plain("audio"))?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(
        plan.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.group" && outcome.kind() == ConversionKind::Unsupported
        })
    );
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    assert_eq!(
        plan.authorize(LossPolicy::AllowPartial)
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "GroupAdd=audio\n",
        ))
    );
    Ok(())
}

#[test]
fn reports_start_interval_as_explicit_partial_output() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("health")?);
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    let mut healthcheck = Healthcheck::new();
    healthcheck.set_command(sourced(HealthcheckCommand::Shell(ProtectedString::plain(
        "curl --fail http://127.0.0.1/health || exit 1",
    )))?);
    healthcheck.set_interval(sourced(HealthcheckDuration::new("provider-specific")?)?);
    healthcheck.set_start_interval(sourced(HealthcheckDuration::new("2s")?)?);
    service.set_healthcheck(sourced(healthcheck)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.healthcheck.start_interval"
            && outcome.kind() == ConversionKind::Unsupported
            && !outcome.origins().is_empty()
    }));
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.healthcheck.interval"
            && outcome.kind() == ConversionKind::Unsupported
            && !outcome.origins().is_empty()
    }));
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    let partial = plan.authorize(LossPolicy::AllowPartial);
    assert_eq!(
        partial
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "HealthCmd=[\"CMD-SHELL\",\"curl --fail http://127.0.0.1/health || exit 1\"]\n",
        ))
    );
    Ok(())
}

#[test]
fn maps_required_and_optional_started_dependencies_to_systemd_units() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("dependencies")?);
    application.add_service(sourced(image_service("database")?)?)?;
    application.add_service(sourced(image_service("cache")?)?)?;

    let mut web = image_service("web")?;
    web.add_dependency(sourced(ServiceDependency::new(id("database")?))?);
    let mut cache = ServiceDependency::new(id("cache")?);
    cache.set_required(sourced(false)?);
    web.add_dependency(sourced(cache)?);
    application.add_service(sourced(web)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    let result = plan.authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("exact dependency output expected")?;
    assert_eq!(
        output.file("web.container").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Unit]\n",
            "Requires=database.service\n",
            "After=database.service\n",
            "Wants=cache.service\n",
            "After=cache.service\n",
            "\n",
            "[Container]\n",
            "Image=example.invalid/web:1\n",
        ))
    );
    Ok(())
}

#[test]
fn maps_healthy_dependencies_only_with_explicit_target_readiness() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("readiness")?);
    let mut database = image_service("database")?;
    database.set_healthcheck(sourced(exact_healthcheck()?)?);
    application.add_service(sourced(database)?)?;

    let mut web = image_service("web")?;
    let mut dependency = ServiceDependency::new(id("database")?);
    dependency.set_condition(sourced(ServiceDependencyCondition::Healthy)?);
    web.add_dependency(sourced(dependency)?);
    application.add_service(sourced(web)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    let result = plan.authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("health-gated output expected")?;
    let database = output
        .file("database.container")
        .ok_or("database container expected")?
        .text();
    assert!(database.contains("HealthCmd=[\"CMD\",\"curl\",\"--fail\",\"http://127.0.0.1/health\"]\n"));
    assert!(database.contains("Notify=healthy\n"));
    assert_eq!(
        output.file("web.container").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Unit]\n",
            "Requires=database.service\n",
            "After=database.service\n",
            "\n",
            "[Container]\n",
            "Image=example.invalid/web:1\n",
        ))
    );
    Ok(())
}

#[test]
fn keeps_restart_and_successful_completion_as_explicit_partial_losses() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("partial-dependencies")?);
    application.add_service(sourced(image_service("database")?)?)?;
    application.add_service(sourced(image_service("migration")?)?)?;

    let mut web = image_service("web")?;
    let mut database = ServiceDependency::new(id("database")?);
    database.set_restart(sourced(true)?);
    web.add_dependency(sourced(database)?);
    let mut migration = ServiceDependency::new(id("migration")?);
    migration.set_condition(sourced(ServiceDependencyCondition::CompletedSuccessfully)?);
    web.add_dependency(sourced(migration)?);
    application.add_service(sourced(web)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    for subject in ["services.web.dependencies[0].restart", "services.web.dependencies[1]"] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("partial dependency output expected")?;
    let web = output.file("web.container").ok_or("web container expected")?.text();
    assert!(web.contains("Requires=database.service\n"));
    assert!(!web.contains("migration.service"));
    Ok(())
}

#[test]
fn rejects_missing_required_dependencies_and_ordering_cycles() -> Result<(), Box<dyn Error>> {
    let mut missing = Application::new(id("missing")?);
    let mut web = image_service("web")?;
    web.add_dependency(sourced(ServiceDependency::new(id("database")?))?);
    missing.add_service(sourced(web)?)?;
    let missing_plan = QuadletExporter::new()?.plan(&missing, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(missing_plan.candidate().is_none());
    assert!(missing_plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.dependencies[0]" && outcome.kind() == ConversionKind::Invalid
    }));

    let mut cyclic = Application::new(id("cyclic")?);
    let mut web = image_service("web")?;
    web.add_dependency(sourced(ServiceDependency::new(id("database")?))?);
    cyclic.add_service(sourced(web)?)?;
    let mut database = image_service("database")?;
    database.add_dependency(sourced(ServiceDependency::new(id("web")?))?);
    cyclic.add_service(sourced(database)?)?;
    let cycle_plan = QuadletExporter::new()?.plan(&cyclic, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(cycle_plan.candidate().is_none());
    assert!(cycle_plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "application.dependencies" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(
        cycle_plan
            .diagnostics()
            .iter()
            .all(|diagnostic| { diagnostic.code().as_str() == "BFQ0008" && diagnostic.severity() == Severity::Error })
    );
    Ok(())
}

#[test]
fn omits_missing_optional_dependencies_only_after_partial_authorization() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("optional")?);
    let mut web = image_service("web")?;
    let mut dependency = ServiceDependency::new(id("cache")?);
    dependency.set_required(sourced(false)?);
    web.add_dependency(sourced(dependency)?);
    application.add_service(sourced(web)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.candidate().is_some());
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.dependencies[0]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    assert_eq!(
        plan.authorize(LossPolicy::AllowPartial)
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some("[Container]\nImage=example.invalid/web:1\n")
    );
    Ok(())
}

#[test]
fn disables_an_image_health_check_with_native_quadlet_syntax() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("disabled")?);
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    let mut healthcheck = Healthcheck::new();
    healthcheck.set_disabled(sourced(true)?);
    service.set_healthcheck(sourced(healthcheck)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert_eq!(
        plan.authorize(LossPolicy::ExactOnly)
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "HealthCmd=none\n",
        ))
    );
    Ok(())
}

#[test]
fn references_external_resources_without_generating_lifecycle_files() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("external")?);
    application.add_network(sourced(Network::new(id("shared")?, ResourceOwnership::External))?)?;
    application.add_volume(sourced(Volume::new(id("database")?, ResourceOwnership::External))?)?;
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    service.add_mount(sourced(Mount::new(
        MountSource::Volume(id("database")?),
        "/var/lib/database",
        false,
    )?)?);
    service.add_network(sourced(NetworkAttachment::new(id("shared")?, Vec::new()))?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    let result = plan.authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("exact output expected")?;
    assert_eq!(output.files().len(), 1);
    assert_eq!(
        output.file("web.container").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Volume=database:/var/lib/database\n",
            "Network=shared\n",
        ))
    );
    assert!(output.document_set().graph().references().is_empty());
    Ok(())
}

#[test]
fn maps_external_secrets_with_default_preservation_and_mount_options() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("external-secrets")?);
    application.add_secret(sourced(Secret::new(
        id("database-password")?,
        ResourceOwnership::External,
    ))?)?;
    let mut api_token = Secret::new(id("api-token")?, ResourceOwnership::External);
    api_token.set_runtime_name(sourced(ProtectedString::plain("company-api-token"))?);
    application.add_secret(sourced(api_token)?)?;
    application.add_secret(sourced(Secret::new(id("certificate")?, ResourceOwnership::External))?)?;

    let mut service = image_service("web")?;
    service.add_secret_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("database-password"),
        ResourceGrantSyntax::Short,
    )?)?);
    service.add_secret_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("api-token"),
        ResourceGrantSyntax::Short,
    )?)?);
    let mut certificate = ResourceGrant::new(ProtectedString::plain("certificate"), ResourceGrantSyntax::Long)?;
    certificate.set_target(sourced(ProtectedString::plain("/etc/example/certificate.pem"))?);
    certificate.set_uid(sourced(ProtectedString::plain("101"))?);
    certificate.set_gid(sourced(ProtectedString::plain("102"))?);
    certificate.set_mode(sourced(ProtectedString::plain("0o440"))?);
    service.add_secret_grant(sourced(certificate)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        plan.outcomes()
    );
    assert_eq!(
        plan.authorize(LossPolicy::ExactOnly)
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Secret=database-password\n",
            "Secret=company-api-token,target=api-token\n",
            "Secret=certificate,target=/etc/example/certificate.pem,uid=101,gid=102,mode=0440\n",
        ))
    );
    Ok(())
}

#[test]
fn reports_config_and_application_owned_secret_materialization_as_manual_actions() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("managed-material")?);
    let mut config = Config::new(id("settings")?, ResourceOwnership::Application);
    config.set_material(sourced(ConfigMaterial::Content(ProtectedString::plain(
        "debug=true\n",
    )))?);
    application.add_config(sourced(config)?)?;
    let mut secret = Secret::new(id("password")?, ResourceOwnership::Application);
    secret.set_material(sourced(SecretMaterial::File(ProtectedString::sensitive(
        "./password.txt",
    )))?);
    application.add_secret(sourced(secret)?)?;

    let mut service = image_service("web")?;
    service.add_config_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("settings"),
        ResourceGrantSyntax::Short,
    )?)?);
    service.add_secret_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("password"),
        ResourceGrantSyntax::Short,
    )?)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    for subject in [
        "configs.settings",
        "secrets.password",
        "services.web.configs[0]",
        "services.web.secrets[0]",
    ] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported }),
            "missing unsupported outcome for {subject}: {:#?}",
            plan.outcomes()
        );
    }
    assert!(plan.clone().authorize(LossPolicy::AllowApproximate).is_blocked());
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("authorized partial output expected")?;
    let container = output.file("web.container").ok_or("container expected")?.text();
    assert!(!container.contains("Secret="));
    assert!(!format!("{output:?}").contains("password.txt"));
    Ok(())
}

#[test]
fn rejects_a_secret_grant_without_a_declared_resource() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("missing-secret")?);
    let mut service = image_service("web")?;
    service.add_secret_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("missing"),
        ResourceGrantSyntax::Short,
    )?)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.secrets[0]" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(plan.authorize(LossPolicy::AllowPartial).is_blocked());
    Ok(())
}

#[test]
fn reports_a_sensitive_external_secret_name_without_disclosing_it() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("sensitive-secret-name")?);
    let mut secret = Secret::new(id("api-token")?, ResourceOwnership::External);
    secret.set_runtime_name(sourced(ProtectedString::sensitive("production-api-token"))?);
    application.add_secret(sourced(secret)?)?;
    let mut service = image_service("web")?;
    service.add_secret_grant(sourced(ResourceGrant::new(
        ProtectedString::plain("api-token"),
        ResourceGrantSyntax::Short,
    )?)?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.secrets[0]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(!format!("{plan:?}").contains("production-api-token"));
    let result = plan.authorize(LossPolicy::AllowPartial);
    let container = result
        .output()
        .and_then(|output| output.file("web.container"))
        .map(boxferry_quadlet::QuadletFile::text)
        .ok_or("partial container expected")?;
    assert!(!container.contains("Secret="));
    Ok(())
}

#[test]
fn groups_compatible_services_only_after_explicit_approximation_authorization() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("grouped")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;

    let mut web = Service::new(id("web")?);
    web.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    web.add_host_mapping(sourced(HostMapping::new(
        id("host.docker.internal")?,
        HostAddress::new("host-gateway")?,
    ))?);
    web.add_port(sourced(Port::new(80, Some(8080), None, Protocol::Tcp)?)?);
    web.add_network(sourced(NetworkAttachment::new(id("frontend")?, Vec::new()))?);
    application.add_service(sourced(web)?)?;

    let mut worker = Service::new(id("worker")?);
    worker.set_image(sourced(ImageReference::parse("example.invalid/worker:1")?)?);
    worker.add_host_mapping(sourced(HostMapping::new(
        id("host.docker.internal")?,
        HostAddress::new("host-gateway")?,
    ))?);
    worker.add_port(sourced(Port::new(90, Some(9090), None, Protocol::Tcp)?)?);
    worker.add_network(sourced(NetworkAttachment::new(id("frontend")?, Vec::new()))?);
    application.add_service(sourced(worker)?)?;

    let exporter = QuadletExporter::new()?.with_grouping_policy(QuadletGroupingPolicy::SinglePod);
    assert_eq!(exporter.grouping_policy(), QuadletGroupingPolicy::SinglePod);
    let plan = exporter.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    let grouping = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.grouping")
        .ok_or("grouping outcome expected")?;
    assert_eq!(grouping.kind(), ConversionKind::Approximate);
    assert_eq!(grouping.origins().len(), 2);
    assert!(
        plan.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFQ0007" && diagnostic.severity() == Severity::Warning
        })
    );
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());

    let result = plan.authorize(LossPolicy::AllowApproximate);
    let output = result.output().ok_or("authorized approximate output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["frontend.network", "grouped.pod", "web.container", "worker.container"]
    );
    assert_eq!(
        output.file("grouped.pod").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Pod]\n",
            "PodName=grouped\n",
            "AddHost=host.docker.internal:host-gateway\n",
            "PublishPort=8080:80/tcp\n",
            "PublishPort=9090:90/tcp\n",
            "Network=frontend.network\n",
        ))
    );
    assert_eq!(
        output.file("web.container").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Pod=grouped.pod\n",
        ))
    );
    assert_eq!(
        output.file("worker.container").map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/worker:1\n",
            "Pod=grouped.pod\n",
        ))
    );
    assert!(output.document_set().graph().is_complete());
    assert_eq!(output.document_set().graph().edges().len(), 3);
    Ok(())
}

#[test]
fn moves_an_identical_user_namespace_to_the_grouped_pod() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("grouped-userns")?);
    for name in ["web", "worker"] {
        let mut service = image_service(name)?;
        service.set_user_namespace(sourced(ProtectedString::plain("keep-id"))?);
        application.add_service(sourced(service)?)?;
    }

    let plan = QuadletExporter::new()?
        .with_grouping_policy(QuadletGroupingPolicy::SinglePod)
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    let namespace = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.pod.user_namespace")
        .ok_or("pod user-namespace outcome expected")?;
    assert_eq!(namespace.kind(), ConversionKind::Exact);
    assert_eq!(namespace.origins().len(), 2);
    assert!(!plan.outcomes().iter().any(|outcome| {
        outcome.subject().ends_with(".user_namespace") && outcome.kind() == ConversionKind::Unsupported
    }));

    let result = plan.authorize(LossPolicy::AllowApproximate);
    let output = result.output().ok_or("grouped output expected")?;
    let pod = output
        .file("grouped-userns.pod")
        .map(boxferry_quadlet::QuadletFile::text)
        .ok_or("grouped pod expected")?;
    assert!(pod.contains("UserNS=keep-id\n"));
    let container = output
        .file("web.container")
        .map(boxferry_quadlet::QuadletFile::text)
        .ok_or("grouped container expected")?;
    assert!(!container.contains("UserNS="));
    assert!(container.contains("Pod=grouped-userns.pod\n"));
    Ok(())
}

#[test]
fn rejects_mixed_or_conflicting_grouped_user_namespaces() -> Result<(), Box<dyn Error>> {
    for worker_namespace in [None, Some("private")] {
        let mut application = Application::new(id("grouped-userns-conflict")?);
        let mut web = image_service("web")?;
        web.set_user_namespace(sourced(ProtectedString::plain("keep-id"))?);
        application.add_service(sourced(web)?)?;
        let mut worker = image_service("worker")?;
        if let Some(namespace) = worker_namespace {
            worker.set_user_namespace(sourced(ProtectedString::plain(namespace))?);
        }
        application.add_service(sourced(worker)?)?;

        let plan = QuadletExporter::new()?
            .with_grouping_policy(QuadletGroupingPolicy::SinglePod)
            .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
        assert!(plan.candidate().is_none());
        assert!(plan.outcomes().iter().any(|outcome| {
            outcome.subject() == "application.grouping" && outcome.kind() == ConversionKind::Invalid
        }));
        assert!(plan.authorize(LossPolicy::AllowPartial).is_blocked());
    }
    Ok(())
}

#[test]
fn rejects_incompatible_single_pod_grouping_without_falling_back() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("conflict")?);
    for name in ["web", "worker"] {
        let mut service = Service::new(id(name)?);
        service.set_image(sourced(ImageReference::parse(format!("example.invalid/{name}:1"))?)?);
        service.add_port(sourced(Port::new(80, Some(8080), None, Protocol::Tcp)?)?);
        application.add_service(sourced(service)?)?;
    }

    let plan = QuadletExporter::new()?
        .with_grouping_policy(QuadletGroupingPolicy::SinglePod)
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.candidate().is_none());
    assert!(
        plan.outcomes().iter().any(|outcome| {
            outcome.subject() == "application.grouping" && outcome.kind() == ConversionKind::Invalid
        })
    );
    assert!(
        plan.diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFQ0007" && diagnostic.severity() == Severity::Error })
    );
    assert!(plan.authorize(LossPolicy::AllowPartial).is_blocked());
    Ok(())
}

#[test]
fn rejects_single_pod_grouping_that_would_erase_network_isolation() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("isolated")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;

    let mut web = Service::new(id("web")?);
    web.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    web.add_network(sourced(NetworkAttachment::new(id("frontend")?, Vec::new()))?);
    application.add_service(sourced(web)?)?;

    let mut worker = Service::new(id("worker")?);
    worker.set_image(sourced(ImageReference::parse("example.invalid/worker:1")?)?);
    application.add_service(sourced(worker)?)?;

    let plan = QuadletExporter::new()?
        .with_grouping_policy(QuadletGroupingPolicy::SinglePod)
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.candidate().is_none());
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFQ0007")
        .ok_or("grouping diagnostic expected")?;
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(diagnostic.fields().iter().any(|field| {
        field.name() == "reason" && field.value().expose().contains("same ordered network attachments")
    }));
    Ok(())
}

#[test]
fn rejects_single_pod_grouping_with_different_service_host_mappings() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("host-conflict")?);
    for (name, address) in [("web", "192.0.2.10"), ("worker", "192.0.2.11")] {
        let mut service = Service::new(id(name)?);
        service.set_image(sourced(ImageReference::parse(format!("example.invalid/{name}:1"))?)?);
        service.add_host_mapping(sourced(HostMapping::new(id("database")?, HostAddress::new(address)?))?);
        application.add_service(sourced(service)?)?;
    }

    let plan = QuadletExporter::new()?
        .with_grouping_policy(QuadletGroupingPolicy::SinglePod)
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.candidate().is_none());
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFQ0007")
        .ok_or("host grouping diagnostic expected")?;
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert!(
        diagnostic
            .fields()
            .iter()
            .any(|field| { field.name() == "reason" && field.value().expose().contains("same ordered host mappings") })
    );
    Ok(())
}

#[test]
fn resolves_relative_binds_against_explicit_caller_context() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("paths")?);
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath("./config".to_owned()),
        "/etc/config",
        true,
    )?)?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath("../shared".to_owned()),
        "/srv/shared",
        false,
    )?)?);
    application.add_service(sourced(service)?)?;

    let exporter = QuadletExporter::new()?.with_relative_bind_root("/srv/project/./")?;
    assert_eq!(exporter.relative_bind_root(), Some("/srv/project"));
    let output = exporter
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?
        .authorize(LossPolicy::ExactOnly);
    let container = output
        .output()
        .and_then(|output| output.file("web.container"))
        .map(boxferry_quadlet::QuadletFile::text)
        .ok_or("exact output expected")?;
    assert_eq!(
        container,
        concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Volume=/srv/project/config:/etc/config:ro\n",
            "Volume=/srv/shared:/srv/shared\n",
        )
    );
    assert!(
        QuadletExporter::new()?
            .with_relative_bind_root("relative/root")
            .is_err()
    );
    assert!(
        QuadletExporter::new()?
            .with_relative_bind_root("/../../escape")
            .is_err()
    );
    Ok(())
}

#[test]
fn maps_host_specific_bind_sources_only_through_explicit_caller_policy() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("host-paths")?);
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath("~/.config/example".to_owned()),
        "/etc/example",
        true,
    )?)?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath(r"C:\data\example".to_owned()),
        "/var/lib/example",
        false,
    )?)?);
    application.add_service(sourced(service)?)?;

    let default_plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert_eq!(
        default_plan
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert!(default_plan.authorize(LossPolicy::ExactOnly).is_blocked());

    let exporter = QuadletExporter::new()?
        .with_bind_source_mapping("~/.config/example", "%h/.config/example")?
        .with_bind_source_mapping(r"C:\data\example", "/mnt/c/data/example")?;
    assert_eq!(
        exporter.bind_source_mapping("~/.config/example"),
        Some("%h/.config/example")
    );
    let output = exporter
        .plan(&application, &podman_target(Some(version(6, 0, 2)))?)?
        .authorize(LossPolicy::ExactOnly);
    assert_eq!(
        output
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry_quadlet::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Volume=%h/.config/example:/etc/example:ro\n",
            "Volume=/mnt/c/data/example:/var/lib/example\n",
        ))
    );

    assert!(
        QuadletExporter::new()?
            .with_bind_source_mapping("~/data", "relative/data")
            .is_err()
    );
    assert!(
        QuadletExporter::new()?
            .with_bind_source_mapping("", "/srv/data")
            .is_err()
    );
    assert!(
        QuadletExporter::new()?
            .with_bind_source_mapping("~/data", "%h/data")?
            .with_bind_source_mapping("~/data", "/srv/data")
            .is_err()
    );
    Ok(())
}

#[test]
fn retains_a_partial_candidate_but_requires_authorization_for_unresolved_intent() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("partial")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    service.add_environment(sourced(EnvironmentVariable::new(
        id("FROM_HOST")?,
        EnvironmentValue::Host,
    ))?);
    service.add_host_mapping(sourced(HostMapping::new(
        id("host.docker.internal")?,
        HostAddress::new("host-gateway")?,
    ))?);
    service.add_host_mapping(sourced(HostMapping::new(
        id("deferred")?,
        HostAddress::new("${HOST_ADDRESS}")?,
    ))?);
    service.add_mount(sourced(Mount::new(
        MountSource::HostPath("./config".to_owned()),
        "/etc/config",
        true,
    )?)?);
    service.add_network(sourced(NetworkAttachment::new(
        id("frontend")?,
        vec!["web.local".to_owned()],
    ))?);
    application.add_service(sourced(service)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.candidate().is_some());
    for subject in [
        "services.web.environment.FROM_HOST",
        "services.web.host_mappings[1]",
        "services.web.mounts[0]",
        "services.web.networks.frontend.aliases",
    ] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }
    assert_eq!(
        plan.diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0003")
            .count(),
        4
    );
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    let partial = plan.authorize(LossPolicy::AllowPartial);
    let output = partial.output().ok_or("partial output expected")?;
    let container = output.file("web.container").ok_or("container expected")?.text();
    assert!(!container.contains("FROM_HOST"));
    assert!(container.contains("AddHost=host.docker.internal:host-gateway"));
    assert!(!container.contains("HOST_ADDRESS"));
    assert!(!container.contains("./config"));
    assert!(container.contains("Network=frontend.network"));
    Ok(())
}

#[test]
fn reports_neutral_service_groups_until_the_caller_resolves_target_lifecycle() -> Result<(), Box<dyn Error>> {
    let mut application = minimal_application()?;
    let mut group = ServiceGroup::new(id("observed-pod")?, ResourceOwnership::Uncertain);
    group.add_member(sourced(id("web")?)?)?;
    application.add_service_group(sourced(group)?)?;

    let plan = QuadletExporter::new()?.plan(&application, &podman_target(Some(version(6, 0, 2)))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.observed-pod"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFQ0003")
    }));
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    assert!(plan.authorize(LossPolicy::AllowPartial).output().is_some());
    Ok(())
}

#[test]
fn fails_closed_outside_verified_podman_coverage() -> Result<(), Box<dyn Error>> {
    let application = minimal_application()?;
    let exporter = QuadletExporter::new()?;
    for target in [
        TargetProfile::new("podman", version(5, 3, 0), Some(version(6, 0, 2)))?,
        TargetProfile::new("podman", version(5, 4, 0), Some(version(6, 1, 0)))?,
        TargetProfile::new("kubernetes", version(1, 34, 0), None)?,
    ] {
        let plan = exporter.plan(&application, &target)?;
        assert!(plan.candidate().is_none());
        assert!(
            plan.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFQ0001" && diagnostic.severity() == Severity::Error
            })
        );
        assert!(plan.authorize(LossPolicy::AllowPartial).is_blocked());
    }
    Ok(())
}

#[test]
fn reports_the_finite_evidence_ceiling_when_maximum_is_omitted() -> Result<(), Box<dyn Error>> {
    let plan = QuadletExporter::new()?.plan(&minimal_application()?, &podman_target(None)?)?;
    assert!(plan.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFQ0002"
            && diagnostic.severity() == Severity::Note
            && diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "verified-through" && field.value().expose() == "6.0.2")
    }));
    assert!(!plan.authorize(LossPolicy::ExactOnly).is_blocked());
    Ok(())
}

fn minimal_application() -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(id("minimal")?);
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    application.add_service(sourced(service)?)?;
    Ok(application)
}

fn image_service(name: &str) -> Result<Service, Box<dyn Error>> {
    let mut service = Service::new(id(name)?);
    service.set_image(sourced(ImageReference::parse(format!("example.invalid/{name}:1"))?)?);
    Ok(service)
}

fn exact_healthcheck() -> Result<Healthcheck, Box<dyn Error>> {
    let mut healthcheck = Healthcheck::new();
    healthcheck.set_command(sourced(HealthcheckCommand::Exec(vec![
        ProtectedString::plain("curl"),
        ProtectedString::plain("--fail"),
        ProtectedString::plain("http://127.0.0.1/health"),
    ]))?);
    healthcheck.set_interval(sourced(HealthcheckDuration::new("30s")?)?);
    healthcheck.set_timeout(sourced(HealthcheckDuration::new("5s")?)?);
    healthcheck.set_retries(sourced(HealthcheckRetries::new("3")?)?);
    healthcheck.set_start_period(sourced(HealthcheckDuration::new("10s")?)?);
    Ok(healthcheck)
}

const fn exact_first_conversion_container() -> &'static str {
    concat!(
        "[Container]\n",
        "Image=registry.example:5000/team/web:1.3@sha256:fedcba\n",
        "Exec=php -v\n",
        "User=1001\n",
        "Group=1002\n",
        "UserNS=keep-id\n",
        "GroupAdd=audio\n",
        "GroupAdd=44\n",
        "WorkingDir=/srv/app\n",
        "ReadOnly=true\n",
        "HealthCmd=[\"CMD\",\"curl\",\"--fail\",\"http://127.0.0.1/health\"]\n",
        "HealthInterval=30s\n",
        "HealthTimeout=5s\n",
        "HealthRetries=3\n",
        "HealthStartPeriod=10s\n",
        "Environment=APP_ENV=production\n",
        "AddHost=host.docker.internal:host-gateway\n",
        "AddHost=ipv6:[::1]\n",
        "PublishPort=127.0.0.1:8080:80/tcp\n",
        "Volume=data.volume:/var/lib/data:ro,Z\n",
        "Volume=/srv/config:/etc/config:z\n",
        "Volume=%h/.config/example:/home/config:ro\n",
        "Network=frontend.network\n",
    )
}

fn sourced<T>(value: T) -> Result<Sourced<T>, Box<dyn Error>> {
    Ok(Sourced::from_source(
        value,
        Provenance::source(SourceId::new("fixture.compose.yaml")?),
    ))
}

fn id(value: &str) -> Result<Identifier, Box<dyn Error>> {
    Ok(Identifier::new(value)?)
}

fn podman_target(maximum: Option<PlatformVersion>) -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new("podman", version(5, 4, 0), maximum)?)
}

const fn version(major: u64, minor: u64, patch: u64) -> PlatformVersion {
    PlatformVersion::new(major, minor, patch)
}
