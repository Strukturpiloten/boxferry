//! Public neutral-model-to-Compose adapter behavior.

use std::error::Error;

use boxferry_compose::{ComposeExporter, ComposeRuntime, DOCKER_COMPOSE_TARGET, PODMAN_COMPOSE_TARGET};
use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, TargetProfile};
use boxferry_model::{
    Application, Command, Config, EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue,
    EnvironmentVariable, Healthcheck, HostAddress, HostMapping, Identifier, ImageReference, MetadataLabel, Mount,
    MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, ResourceOwnership,
    RestartPolicy, Secret, SecurityOption, SelinuxRelabel, Service, ServiceGroup, SourceId, Sourced, Volume,
};

#[test]
fn exports_the_supported_subset_deterministically_and_redacts_sensitive_values() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("application.yaml")?);
    let mut application = Application::new(Identifier::new("demo")?);
    application.add_network(Sourced::from_source(
        Network::new(Identifier::new("frontend")?, ResourceOwnership::Application),
        origin.clone(),
    ))?;
    application.add_volume(Sourced::from_source(
        Volume::new(Identifier::new("data")?, ResourceOwnership::Application),
        origin.clone(),
    ))?;

    let mut service = Service::new(Identifier::new("web")?);
    service.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("ferry-web"),
        origin.clone(),
    ));
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_command(Sourced::from_source(
        Command::Exec(vec![
            ProtectedString::plain("server"),
            ProtectedString::plain("--foreground"),
        ]),
        origin.clone(),
    ));
    service.set_user(Sourced::from_source(ProtectedString::sensitive("1001"), origin.clone()));
    service.set_group(Sourced::from_source(ProtectedString::sensitive("1002"), origin.clone()));
    service.set_user_namespace(Sourced::from_source(ProtectedString::plain("private"), origin.clone()));
    service.add_supplementary_group(Sourced::from_source(ProtectedString::plain("44"), origin.clone()));
    service.set_working_directory(Sourced::from_source(ProtectedString::plain("/srv/app"), origin.clone()));
    service.set_read_only_root_filesystem(Sourced::from_source(true, origin.clone()));
    service.set_restart_policy(Sourced::from_source(
        RestartPolicy::on_failure(std::num::NonZeroU64::new(3)),
        origin.clone(),
    ));
    service.add_environment(Sourced::from_source(
        EnvironmentVariable::new(
            Identifier::new("TOKEN")?,
            EnvironmentValue::Literal(ProtectedString::sensitive("production-secret")),
        ),
        origin.clone(),
    ));
    service.add_environment(Sourced::from_source(
        EnvironmentVariable::new(Identifier::new("HOST_VALUE")?, EnvironmentValue::Host),
        origin.clone(),
    ));
    add_environment_files(&mut service, &origin)?;
    service.add_label(Sourced::from_source(
        MetadataLabel::new(Identifier::new("com.example.empty")?, ProtectedString::plain("")),
        origin.clone(),
    ));
    service.add_label(Sourced::from_source(
        MetadataLabel::new(
            Identifier::new("com.example.metadata")?,
            ProtectedString::sensitive(r#"{"channel": "production-secret"}"#),
        ),
        origin.clone(),
    ));
    service.add_host_mapping(Sourced::from_source(
        HostMapping::new(Identifier::new("database")?, HostAddress::new("192.0.2.10")?),
        origin.clone(),
    ));
    service.add_port(Sourced::from_source(
        Port::new(8080, Some(18080), Some("127.0.0.1".to_owned()), Protocol::Tcp)?,
        origin.clone(),
    ));
    service.add_mount(Sourced::from_source(
        Mount::new(MountSource::Volume(Identifier::new("data")?), "/var/lib/example", true)?,
        origin.clone(),
    ));
    service.add_mount(Sourced::from_source(
        Mount::new(MountSource::HostPath("./config".to_owned()), "/etc/example", false)?,
        origin.clone(),
    ));
    service.add_mount(Sourced::from_source(
        Mount::new(MountSource::Anonymous, "/tmp", false)?,
        origin.clone(),
    ));
    service.add_network(Sourced::from_source(
        NetworkAttachment::new(Identifier::new("frontend")?, vec!["web.local".to_owned()]),
        origin.clone(),
    ));
    application.add_service(Sourced::from_source(service, origin))?;

    let exporter = ComposeExporter::new()?.with_runtime(ComposeRuntime::DockerEngine(version(29, 7, 1)));
    let plan = exporter.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let output = plan.candidate().ok_or("generated document expected")?;
    assert!(output.is_sensitive());
    assert_eq!(output.text(), expected_supported_document());
    assert_sensitive_debug(&format!("{output:?}"));
    Ok(())
}

#[test]
fn exports_dns_collections_in_order_and_preserves_explicit_empty_lists() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("dns.compose.yaml")?);
    let mut application = Application::new(Identifier::new("dns")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_dns_servers_with_origins(
        vec![
            Sourced::from_source(ProtectedString::plain("1.1.1.1"), origin.clone()),
            Sourced::from_source(ProtectedString::plain("8.8.8.8"), origin.clone()),
        ],
        vec![origin.clone()],
    );
    service.set_dns_options_with_origins(Vec::new(), vec![origin.clone()]);
    service.set_dns_search_domains_with_origins(
        vec![Sourced::from_source(
            ProtectedString::plain("example.test"),
            origin.clone(),
        )],
        vec![origin],
    );
    application.add_service(Sourced::generated(service))?;
    let plan = ComposeExporter::new()?
        .with_runtime(ComposeRuntime::DockerEngine(version(29, 7, 1)))
        .plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("Compose output expected")?;
    let text = output.text();
    assert!(text.contains("dns:\n      - \"1.1.1.1\"\n      - \"8.8.8.8\""));
    assert!(text.contains("dns_opt: []"));
    assert!(text.contains("dns_search:\n      - \"example.test\""));
    Ok(())
}

#[test]
fn exports_all_security_options_canonically_in_order_and_redacts_sensitive_payloads() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("security.yaml")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_security_options_with_origins(
        vec![
            Sourced::from_source(
                SecurityOption::AppArmor(ProtectedString::plain("profile-a")),
                origin.clone(),
            ),
            Sourced::from_source(SecurityOption::NoNewPrivileges(false), origin.clone()),
            Sourced::from_source(
                SecurityOption::SeccompProfile(ProtectedString::sensitive("secret-seccomp.json")),
                origin.clone(),
            ),
            Sourced::from_source(SecurityOption::SecurityLabelDisable(true), origin.clone()),
            Sourced::from_source(
                SecurityOption::SecurityLabelFileType(ProtectedString::plain("container_file_t")),
                origin.clone(),
            ),
            Sourced::from_source(
                SecurityOption::SecurityLabelLevel(ProtectedString::plain("s0:c1,c2")),
                origin.clone(),
            ),
            Sourced::from_source(SecurityOption::SecurityLabelNested(true), origin.clone()),
            Sourced::from_source(
                SecurityOption::SecurityLabelType(ProtectedString::plain("container_t")),
                origin.clone(),
            ),
            Sourced::from_source(
                SecurityOption::Mask(ProtectedString::plain("/proc/acpi")),
                origin.clone(),
            ),
            Sourced::from_source(
                SecurityOption::Unmask(ProtectedString::plain("/proc/acpi")),
                origin.clone(),
            ),
            Sourced::from_source(
                SecurityOption::Mask(ProtectedString::plain("/proc/acpi")),
                origin.clone(),
            ),
        ],
        vec![origin.clone()],
    );
    let mut application = Application::new(Identifier::new("security")?);
    application.add_service(Sourced::from_source(service, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let output = plan.candidate().ok_or("generated Compose expected")?;
    assert!(output.is_sensitive());
    assert!(!format!("{output:?}").contains("secret-seccomp.json"));
    assert!(output.text().contains(concat!(
        "security_opt:\n",
        "      - \"apparmor=profile-a\"\n",
        "      - \"no-new-privileges:false\"\n",
        "      - \"seccomp=secret-seccomp.json\"\n",
        "      - \"label:disable\"\n",
        "      - \"label:filetype:container_file_t\"\n",
        "      - \"label:level:s0:c1,c2\"\n",
        "      - \"label:nested\"\n",
        "      - \"label:type:container_t\"\n",
        "      - \"mask=/proc/acpi\"\n",
        "      - \"unmask=/proc/acpi\"\n",
        "      - \"mask=/proc/acpi\"\n",
    )));
    assert_eq!(
        plan.outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == "services.web.security_opt")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn reports_false_selinux_security_options_as_unsupported_without_generating_them() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("security-false.yaml")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_security_options_with_origins(
        vec![
            Sourced::from_source(SecurityOption::SecurityLabelDisable(false), origin.clone()),
            Sourced::from_source(SecurityOption::SecurityLabelNested(false), origin.clone()),
            Sourced::from_source(
                SecurityOption::Mask(ProtectedString::plain("/proc/acpi")),
                origin.clone(),
            ),
        ],
        vec![origin.clone()],
    );
    let mut application = Application::new(Identifier::new("security-false")?);
    application.add_service(Sourced::from_source(service, origin.clone()))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    for index in [0, 1] {
        assert!(plan.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.security_opt[{index}]")
                && outcome.kind() == ConversionKind::Unsupported
                && outcome.origins() == [origin.clone()]
        }));
    }
    let output = plan.candidate().ok_or("generated Compose expected")?;
    assert!(output.text().contains("- \"mask=/proc/acpi\""));
    assert!(!output.text().contains("label:disable:false"));
    assert!(!output.text().contains("label:nested:false"));
    assert!(plan.authorize(LossPolicy::ExactOnly).output().is_none());
    Ok(())
}

#[test]
fn exports_explicit_empty_security_options_without_emitting_an_omitted_field() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("security-empty.yaml")?);
    let mut empty = Service::new(Identifier::new("empty")?);
    empty.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/empty:1")?,
        origin.clone(),
    ));
    empty.set_security_options_with_origins(Vec::new(), vec![origin.clone()]);
    let mut omitted = Service::new(Identifier::new("omitted")?);
    omitted.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/omitted:1")?,
        origin.clone(),
    ));
    let mut application = Application::new(Identifier::new("security-empty")?);
    application.add_service(Sourced::from_source(empty, origin.clone()))?;
    application.add_service(Sourced::from_source(omitted, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let output = plan.candidate().ok_or("generated Compose expected")?;
    assert!(
        output
            .text()
            .contains("  \"empty\":\n    image: \"example.invalid/empty:1\"\n    security_opt: []")
    );
    assert!(
        output
            .text()
            .contains("  \"omitted\":\n    image: \"example.invalid/omitted:1\"\n")
    );
    assert!(
        !output
            .text()
            .contains("  \"omitted\":\n    image: \"example.invalid/omitted:1\"\n    security_opt")
    );
    Ok(())
}

#[test]
fn reports_security_option_generation_failures_with_item_and_collection_provenance() -> Result<(), Box<dyn Error>> {
    let item_origin = Provenance::source(SourceId::new("invalid-security-item.yaml")?);
    let collection_origin = Provenance::source(SourceId::new("invalid-security-field.yaml")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        item_origin.clone(),
    ));
    service.set_security_options_with_origins(
        vec![Sourced::from_source(
            SecurityOption::AppArmor(ProtectedString::plain("invalid\nprofile")),
            item_origin.clone(),
        )],
        vec![collection_origin.clone()],
    );
    let mut application = Application::new(Identifier::new("invalid-security")?);
    application.add_service(Sourced::from_source(service, item_origin.clone()))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.candidate().is_none());
    let outcome = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.web.security_opt")
        .ok_or("security-option outcome expected")?;
    assert_eq!(outcome.kind(), ConversionKind::Invalid);
    assert_eq!(
        outcome.diagnostic().map(boxferry_engine::DiagnosticCode::as_str),
        Some("BFC0008")
    );
    assert_eq!(outcome.origins(), &[collection_origin, item_origin]);
    Ok(())
}

#[test]
fn reports_an_explicit_container_name_outside_the_compose_grammar() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("invalid-name.yaml")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("invalid name"),
        origin.clone(),
    ));
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    let mut application = Application::new(Identifier::new("invalid-name")?);
    application.add_service(Sourced::from_source(service, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.candidate().is_none());
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.container_name"
            && outcome.kind() == ConversionKind::Invalid
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0008")
    }));
    Ok(())
}

#[test]
fn exports_every_neutral_restart_policy_to_compose() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("restart.yaml")?);
    let policies = [
        ("disabled", RestartPolicy::Never, "restart: \"no\""),
        ("always", RestartPolicy::Always, "restart: \"always\""),
        ("failure", RestartPolicy::on_failure(None), "restart: \"on-failure\""),
        (
            "limited",
            RestartPolicy::on_failure(std::num::NonZeroU64::new(7)),
            "restart: \"on-failure:7\"",
        ),
        ("stopped", RestartPolicy::UnlessStopped, "restart: \"unless-stopped\""),
    ];
    let mut application = Application::new(Identifier::new("restart")?);
    for (name, policy, _) in policies {
        let mut service = Service::new(Identifier::new(name)?);
        service.set_image(Sourced::from_source(
            ImageReference::parse(format!("example.invalid/{name}:1"))?,
            origin.clone(),
        ));
        service.set_restart_policy(Sourced::from_source(policy, origin.clone()));
        application.add_service(Sourced::from_source(service, origin.clone()))?;
    }

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.diagnostics().is_empty(), "{:#?}", plan.diagnostics());
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let output = plan.candidate().ok_or("generated Compose expected")?;
    assert_eq!(
        output.text(),
        concat!(
            "name: \"restart\"\n",
            "services:\n",
            "  \"disabled\":\n",
            "    image: \"example.invalid/disabled:1\"\n",
            "    restart: \"no\"\n",
            "  \"always\":\n",
            "    image: \"example.invalid/always:1\"\n",
            "    restart: \"always\"\n",
            "  \"failure\":\n",
            "    image: \"example.invalid/failure:1\"\n",
            "    restart: \"on-failure\"\n",
            "  \"limited\":\n",
            "    image: \"example.invalid/limited:1\"\n",
            "    restart: \"on-failure:7\"\n",
            "  \"stopped\":\n",
            "    image: \"example.invalid/stopped:1\"\n",
            "    restart: \"unless-stopped\"\n",
        )
    );
    Ok(())
}

#[test]
fn reports_provider_and_runtime_sensitive_constructs_before_authorization() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("compatibility.yaml")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1@sha256:abcd")?,
        origin.clone(),
    ));
    service.set_user_namespace(Sourced::from_source(ProtectedString::plain("keep-id"), origin.clone()));
    service.add_host_mapping(Sourced::from_source(
        HostMapping::new(
            Identifier::new("host.docker.internal")?,
            HostAddress::new("host-gateway")?,
        ),
        origin.clone(),
    ));
    service.add_port(Sourced::from_source(
        Port::new(5000, None, None, Protocol::Sctp)?,
        origin.clone(),
    ));
    let mut bind = Mount::new(MountSource::HostPath("/srv/data".to_owned()), "/data", false)?;
    bind.set_selinux_relabel(SelinuxRelabel::Private);
    service.add_mount(Sourced::from_source(bind, origin.clone()));
    let mut application = Application::new(Identifier::new("compatibility")?);
    application.add_service(Sourced::from_source(service, origin))?;

    let exporter = ComposeExporter::new()?.with_runtime(ComposeRuntime::Podman(version(5, 6, 2)));
    let target = exact_target(DOCKER_COMPOSE_TARGET, version(2, 40, 3))?;
    let strict = exporter.plan(&application, &target)?.authorize(LossPolicy::ExactOnly);
    assert!(strict.is_blocked());
    assert!(strict.output().is_none());
    assert!(strict.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.extra_hosts[0]"
            && outcome.kind() == ConversionKind::Approximate
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0009")
    }));
    assert!(strict.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.user_namespace" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(strict.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.ports[0]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(
        strict.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.mounts[0]" && outcome.kind() == ConversionKind::Exact
        })
    );

    let partial = exporter
        .plan(&application, &target)?
        .authorize(LossPolicy::AllowPartial);
    let output = partial.output().ok_or("partial output expected")?;
    assert!(output.text().contains("host.docker.internal=host-gateway"));
    assert!(output.text().contains("5000/sctp"));
    assert!(output.text().contains("/srv/data:/data:Z"));
    Ok(())
}

#[test]
fn requires_an_exact_recognized_compose_provider_target() -> Result<(), Box<dyn Error>> {
    let application = minimal_application()?;
    let exporter = ComposeExporter::new()?;
    let podman_compose = exporter.plan(&application, &exact_target(PODMAN_COMPOSE_TARGET, version(1, 5, 0))?)?;
    assert!(podman_compose.candidate().is_some());
    assert!(podman_compose.diagnostics().is_empty());
    for target in [
        TargetProfile::new("podman compose", version(1, 5, 0), Some(version(1, 5, 0)))?,
        TargetProfile::new(DOCKER_COMPOSE_TARGET, version(5, 3, 1), None)?,
        TargetProfile::new(DOCKER_COMPOSE_TARGET, version(2, 40, 3), Some(version(5, 3, 1)))?,
    ] {
        let plan = exporter.plan(&application, &target)?;
        assert!(plan.candidate().is_none());
        assert!(plan.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFC0006" && diagnostic.severity() == boxferry_engine::Severity::Error
        }));
    }
    Ok(())
}

#[test]
fn unresolved_runtime_resource_ownership_is_visible_and_partial() -> Result<(), Box<dyn Error>> {
    let runtime = Provenance::runtime_observation(SourceId::new("runtime:docker:volume:data")?);
    let mut application = minimal_application()?;
    application.add_volume(Sourced::from_source(
        Volume::new(Identifier::new("data")?, ResourceOwnership::Uncertain),
        runtime,
    ))?;
    let exporter = ComposeExporter::new()?;
    let target = exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?;
    let plan = exporter.plan(&application, &target)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.data"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0007")
    }));
    let result = plan.authorize(LossPolicy::AllowPartial);
    assert!(result.output().is_some_and(|document| document.text().contains(concat!(
        "volumes:\n",
        "  \"data\":\n",
        "    name: \"data\"\n",
        "    external: true\n",
    ))));
    Ok(())
}

#[test]
fn unimplemented_native_fields_and_structural_groups_remain_visible() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.set_restart_policy(Sourced::generated(RestartPolicy::UnlessStopped));
    service.set_healthcheck(Sourced::generated(Healthcheck::new()));
    service.add_environment(Sourced::generated(EnvironmentVariable::new(
        Identifier::new("ABSENT")?,
        EnvironmentValue::Unset,
    )));
    let mut application = Application::new(Identifier::new("partial")?);
    application.add_service(Sourced::generated(service))?;
    application.add_config(Sourced::generated(Config::new(
        Identifier::new("settings")?,
        ResourceOwnership::Application,
    )))?;
    application.add_secret(Sourced::generated(Secret::new(
        Identifier::new("token")?,
        ResourceOwnership::External,
    )))?;
    let mut group = ServiceGroup::new(Identifier::new("observed-pod")?, ResourceOwnership::Application);
    group.add_member(Sourced::generated(Identifier::new("web")?))?;
    application.add_service_group(Sourced::generated(group))?;

    let exporter = ComposeExporter::new()?;
    let target = exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?;
    let plan = exporter.plan(&application, &target)?;
    for subject in [
        "configs.settings",
        "secrets.token",
        "service_groups.observed-pod",
        "services.web.environment.ABSENT",
        "services.web.healthcheck",
    ] {
        assert!(plan.outcomes().iter().any(|outcome| {
            outcome.subject() == subject
                && outcome.kind() == ConversionKind::Unsupported
                && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0007")
        }));
    }
    let partial = plan.authorize(LossPolicy::AllowPartial);
    assert!(partial.output().is_some_and(|output| {
        output.text()
            == concat!(
                "name: \"partial\"\n",
                "services:\n",
                "  \"web\":\n",
                "    image: \"example.invalid/web:1\"\n",
                "    restart: \"unless-stopped\"\n",
            )
    }));
    Ok(())
}

#[test]
fn refuses_to_reauthor_compose_managed_labels() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new("com.docker.compose.project")?,
        ProtectedString::sensitive("never-print-this"),
    )));
    let mut application = Application::new(Identifier::new("managed-label")?);
    application.add_service(Sourced::generated(service))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(5, 3, 1))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.labels.com.docker.compose.project"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0007")
    }));
    assert!(!format!("{plan:?}").contains("never-print-this"));
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("partial output expected")?;
    assert!(!output.text().contains("com.docker.compose.project"));
    Ok(())
}

fn minimal_application() -> Result<Application, Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    let mut application = Application::new(Identifier::new("minimal")?);
    application.add_service(Sourced::generated(service))?;
    Ok(application)
}

fn add_environment_files(service: &mut Service, origin: &Provenance) -> Result<(), Box<dyn Error>> {
    service.add_environment_file(Sourced::from_source(
        EnvironmentFile::new(ProtectedString::plain("./default.env"), EnvironmentFileSyntax::Short)?,
        origin.clone(),
    ));
    let mut private = EnvironmentFile::new(
        ProtectedString::sensitive("/run/credentials/private.env"),
        EnvironmentFileSyntax::Long,
    )?;
    private.set_required(Sourced::from_source(false, origin.clone()));
    private.set_format(Sourced::from_source(EnvironmentFileFormat::Raw, origin.clone()));
    service.add_environment_file(Sourced::from_source(private, origin.clone()));
    Ok(())
}

fn assert_sensitive_debug(debug: &str) {
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("production-secret"));
    assert!(!debug.contains("1001:1002"));
    assert!(!debug.contains("/run/credentials/private.env"));
}

fn exact_target(implementation: &str, version: PlatformVersion) -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(implementation, version, Some(version))?)
}

const fn version(major: u64, minor: u64, patch: u64) -> PlatformVersion {
    PlatformVersion::new(major, minor, patch)
}

const fn expected_supported_document() -> &'static str {
    concat!(
        "name: \"demo\"\n",
        "services:\n",
        "  \"web\":\n",
        "    container_name: \"ferry-web\"\n",
        "    image: \"example.invalid/web:1\"\n",
        "    command:\n",
        "      - \"server\"\n",
        "      - \"--foreground\"\n",
        "    env_file:\n",
        "      - \"./default.env\"\n",
        "      - path: \"/run/credentials/private.env\"\n",
        "        required: false\n",
        "        format: \"raw\"\n",
        "    environment:\n",
        "      - \"TOKEN=production-secret\"\n",
        "      - \"HOST_VALUE\"\n",
        "    labels:\n",
        "      \"com.example.empty\": \"\"\n",
        "      \"com.example.metadata\": \"{\\\"channel\\\": \\\"production-secret\\\"}\"\n",
        "    user: \"1001:1002\"\n",
        "    userns_mode: \"private\"\n",
        "    group_add:\n",
        "      - \"44\"\n",
        "    working_dir: \"/srv/app\"\n",
        "    read_only: true\n",
        "    restart: \"on-failure:3\"\n",
        "    extra_hosts:\n",
        "      - \"database=192.0.2.10\"\n",
        "    ports:\n",
        "      - target: 8080\n",
        "        published: \"18080\"\n",
        "        host_ip: \"127.0.0.1\"\n",
        "        protocol: \"tcp\"\n",
        "    volumes:\n",
        "      - type: \"volume\"\n",
        "        source: \"data\"\n",
        "        target: \"/var/lib/example\"\n",
        "        read_only: true\n",
        "      - type: \"bind\"\n",
        "        source: \"./config\"\n",
        "        target: \"/etc/example\"\n",
        "      - type: \"volume\"\n",
        "        target: \"/tmp\"\n",
        "    networks:\n",
        "      \"frontend\":\n",
        "        aliases:\n",
        "          - \"web.local\"\n",
        "networks:\n",
        "  \"frontend\": {}\n",
        "volumes:\n",
        "  \"data\": {}\n",
    )
}
