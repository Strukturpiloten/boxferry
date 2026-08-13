//! Public neutral-model-to-Compose adapter behavior.

use std::error::Error;

use boxferry_compose::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeRuntime,
    DOCKER_COMPOSE_TARGET, PODMAN_COMPOSE_TARGET,
};
use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, TargetProfile};
use boxferry_model::{
    Annotation, Application, Command, Config, Device, Entrypoint, EnvironmentFile, EnvironmentFileFormat,
    EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable, GroupExitPolicy, Healthcheck, HostAddress,
    HostMapping, Identifier, ImageAcquisition, ImageBuild, ImageReference, KernelParameter, Logging, LoggingOption,
    MetadataLabel, Mount, MountSource, Network, NetworkAttachment, NetworkDriverOption, NetworkIpamConfig, Port,
    ProtectedString, Protocol, Provenance, ResourceLimit, ResourceOwnership, RestartPolicy, Secret, SecurityOption,
    SelinuxRelabel, Service, ServiceGroup, ServiceGroupRuntime, SourceId, Sourced, StartupNotification, StopTimeout,
    Volume,
};

#[test]
#[allow(clippy::too_many_lines)]
fn generates_iteration_one_compose_fields_and_reports_reload_loss() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("iteration-one")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_entrypoint(Sourced::from_source(
        Entrypoint::Exec(vec![ProtectedString::plain("/bin/web")]),
        origin.clone(),
    ));
    service.set_run_init(Sourced::from_source(true, origin.clone()));
    service.set_stop_timeout(Sourced::from_source(StopTimeout::new("1m30s")?, origin.clone()));
    service.set_pull_policy(Sourced::from_source(boxferry_model::PullPolicy::Daily, origin.clone()));
    service.set_memory_limit(Sourced::from_source(ProtectedString::plain("256m"), origin.clone()));
    service.set_hostname(Sourced::from_source(
        ProtectedString::plain("web.local"),
        origin.clone(),
    ));
    service.set_pids_limit(Sourced::from_source(ProtectedString::plain("42"), origin.clone()));
    service.set_shm_size(Sourced::from_source(ProtectedString::plain("64m"), origin.clone()));
    service.set_cap_add_with_origins(
        vec![Sourced::from_source(
            ProtectedString::plain("NET_ADMIN"),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    service.set_cap_drop_with_origins(
        vec![Sourced::from_source(ProtectedString::plain("MKNOD"), origin.clone())],
        vec![origin.clone()],
    );
    service.set_tmpfs_with_origins(
        vec![Sourced::from_source(ProtectedString::plain("/run"), origin.clone())],
        vec![origin.clone()],
    );
    service.set_sysctls_with_origins(
        vec![Sourced::from_source(
            KernelParameter::new(
                ProtectedString::plain("net.core.somaxconn"),
                ProtectedString::plain("1024"),
            ),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    service.set_ulimits_with_origins(
        vec![Sourced::from_source(
            ResourceLimit::new(
                ProtectedString::plain("nofile"),
                Some(Sourced::from_source(ProtectedString::plain("1024"), origin.clone())),
                Some(Sourced::from_source(ProtectedString::plain("1024"), origin.clone())),
            ),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    service.set_devices_with_origins(
        vec![Sourced::from_source(
            Device::Short(ProtectedString::plain("/dev/fuse")),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    service.set_stop_signal(Sourced::from_source(ProtectedString::plain("SIGTERM"), origin.clone()));
    service.set_exposed_ports(vec![Sourced::from_source(
        boxferry_model::ExposedPort::new(8080, Protocol::Tcp)?,
        origin.clone(),
    )]);
    let mut logging = Logging::new();
    logging.set_driver(Sourced::from_source(
        ProtectedString::plain("json-file"),
        origin.clone(),
    ));
    logging.set_options(vec![Sourced::from_source(
        LoggingOption::new(
            Sourced::from_source(Identifier::new("tag")?, origin.clone()),
            Sourced::from_source(ProtectedString::plain("demo"), origin.clone()),
        ),
        origin.clone(),
    )]);
    service.set_logging(Sourced::from_source(logging, origin.clone()));
    service.set_annotations(vec![Sourced::from_source(
        Annotation::new(
            Sourced::from_source(Identifier::new("io.example.note")?, origin.clone()),
            Sourced::from_source(ProtectedString::plain("stable"), origin.clone()),
        ),
        origin.clone(),
    )]);
    let mut attachment = NetworkAttachment::new(Identifier::new("front")?, vec!["web".to_owned()]);
    attachment.set_ipv4_address(Sourced::from_source(
        ProtectedString::plain("192.0.2.10"),
        origin.clone(),
    ));
    attachment.set_ipv6_address(Sourced::from_source(
        ProtectedString::plain("2001:db8::10"),
        origin.clone(),
    ));
    service.add_network(Sourced::from_source(attachment, origin.clone()));
    service.set_reload_action(Sourced::from_source(
        boxferry_model::ReloadAction::Signal(ProtectedString::plain("SIGHUP")),
        origin.clone(),
    ));
    let mut application = Application::new(Identifier::new("iteration-one")?);
    application.add_service(Sourced::from_source(service, origin))?;
    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.reload_action"
                && outcome.kind() == ConversionKind::Unsupported)
    );
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("output")?;
    assert!(output.text().contains("entrypoint:"));
    assert!(output.text().contains("init: true"));
    assert!(output.text().contains("stop_grace_period: \"1m30s\""));
    assert!(output.text().contains("mem_limit: \"256m\""));
    assert!(output.text().contains("expose:"));
    assert!(output.text().contains("pull_policy: \"daily\""));
    for field in [
        "hostname:",
        "pids_limit:",
        "shm_size:",
        "cap_add:",
        "cap_drop:",
        "tmpfs:",
        "sysctls:",
        "ulimits:",
        "devices:",
        "stop_signal:",
        "annotations:",
        "logging:",
        "ipv4_address:",
        "ipv6_address:",
    ] {
        assert!(output.text().contains(field), "missing {field}");
    }
    assert!(output.text().contains("web"));
    assert!(!format!("{result:?}").contains("SIGHUP"));
    Ok(())
}

#[test]
fn reports_omitted_logging_options_as_approximate_but_preserves_explicit_empty_options() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("logging-options")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    let mut logging = Logging::new();
    logging.set_driver(Sourced::from_source(
        ProtectedString::plain("json-file"),
        origin.clone(),
    ));
    service.set_logging(Sourced::from_source(logging, origin.clone()));
    let mut application = Application::new(Identifier::new("logging-options")?);
    application.add_service(Sourced::from_source(service, origin))?;
    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(plan.outcomes().iter().any(|outcome| outcome.subject() == "services.web.logging" && outcome.kind() == ConversionKind::Approximate));
    Ok(())
}

#[test]
fn reports_retained_image_build_resources_as_compose_generation_losses() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("build.yaml")?);
    let mut application = minimal_application()?;
    application.add_image_acquisition(Sourced::from_source(
        ImageAcquisition::new(Identifier::new("base-image")?),
        origin.clone(),
    ))?;
    application.add_image_build(Sourced::from_source(
        ImageBuild::new(Identifier::new("web-build")?),
        origin,
    ))?;
    let exporter = ComposeExporter::new()?;
    let plan = exporter.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Unsupported && outcome.subject() == "image_builds.web-build"
    }));
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Unsupported && outcome.subject() == "image_acquisitions.base-image"
    }));
    Ok(())
}

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
fn supports_the_provider_neutral_compose_specification_target() -> Result<(), Box<dyn Error>> {
    let application = minimal_application()?;
    let target = exact_target(COMPOSE_SPECIFICATION_TARGET, COMPOSE_SPECIFICATION_PROFILE_REVISION)?;
    let exporter = ComposeExporter::new()?;
    let plan = exporter.plan(&application, &target)?;
    assert!(plan.candidate().is_some());
    assert!(plan.diagnostics().is_empty());

    let wrong_revision = exact_target(COMPOSE_SPECIFICATION_TARGET, version(1, 0, 1))?;
    let rejected = exporter.plan(&application, &wrong_revision)?;
    assert!(rejected.candidate().is_none());
    assert!(
        rejected
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0006")
    );

    let runtime_rejected = ComposeExporter::new()?.with_runtime(ComposeRuntime::DockerEngine(version(29, 7, 1)));
    let rejected = runtime_rejected.plan(&application, &target)?;
    assert!(rejected.candidate().is_none());
    assert!(
        rejected
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0006")
    );
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
#[allow(clippy::too_many_lines)]
fn reports_native_service_and_group_runtime_fields_without_leaking_them_to_a_service() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("native-runtime.container")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    service.set_startup_notification(Sourced::from_source(StartupNotification::Healthy, origin.clone()));
    service.set_podman_args_with_origins(
        vec![Sourced::from_source(
            ProtectedString::sensitive("--secret=never-print"),
            origin.clone(),
        )],
        vec![origin.clone()],
    );

    let mut group_runtime = ServiceGroupRuntime::new();
    group_runtime.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("pod-runtime-name"),
        origin.clone(),
    ));
    group_runtime.set_service_name(Sourced::from_source(
        ProtectedString::plain("pod.service"),
        origin.clone(),
    ));
    group_runtime.set_host_mappings_with_origins(
        vec![Sourced::from_source(
            HostMapping::new(Identifier::new("host.internal")?, HostAddress::new("192.0.2.10")?),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    group_runtime.set_ports_with_origins(
        vec![Sourced::from_source(
            Port::new(8080, Some(18080), None, Protocol::Tcp)?,
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    group_runtime.set_networks_with_origins(
        vec![Sourced::from_source(
            NetworkAttachment::new(Identifier::new("pod-network")?, Vec::new()),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    group_runtime.set_user_namespace(Sourced::from_source(ProtectedString::plain("keep-id"), origin.clone()));
    group_runtime.set_mounts_with_origins(
        vec![Sourced::from_source(
            Mount::new(MountSource::Anonymous, "/pod-cache", false)?,
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    group_runtime.set_shm_size(Sourced::from_source(ProtectedString::sensitive("64m"), origin.clone()));
    group_runtime.set_exit_policy(Sourced::from_source(
        GroupExitPolicy::Raw(ProtectedString::sensitive("private-exit-policy")),
        origin.clone(),
    ));
    group_runtime.set_stop_timeout(Sourced::from_source(StopTimeout::new("30s")?, origin.clone()));
    let mut group = ServiceGroup::new(Identifier::new("pod")?, ResourceOwnership::Application);
    group.add_member(Sourced::from_source(Identifier::new("web")?, origin.clone()))?;
    group.set_runtime(Sourced::from_source(group_runtime, origin.clone()));

    let mut application = Application::new(Identifier::new("native-runtime")?);
    application.add_service(Sourced::from_source(service, origin.clone()))?;
    application.add_service_group(Sourced::from_source(group, origin.clone()))?;
    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;

    for subject in [
        "services.web.startup_notification",
        "services.web.podman_args[0]",
        "service_groups.pod",
        "service_groups.pod.runtime",
        "service_groups.pod.runtime.runtime_name",
        "service_groups.pod.runtime.service_name",
        "service_groups.pod.runtime.host_mappings[0]",
        "service_groups.pod.runtime.ports[0]",
        "service_groups.pod.runtime.networks[0]",
        "service_groups.pod.runtime.user_namespace",
        "service_groups.pod.runtime.mounts[0]",
        "service_groups.pod.runtime.shm_size",
        "service_groups.pod.runtime.exit_policy",
        "service_groups.pod.runtime.stop_timeout",
    ] {
        assert!(
            plan.outcomes().iter().any(|outcome| {
                outcome.subject() == subject
                    && outcome.kind() == ConversionKind::Unsupported
                    && outcome.origins() == [origin.clone()]
            }),
            "missing sourced loss for {subject}"
        );
    }
    let debug = format!("{plan:?}");
    assert!(!debug.contains("never-print"));
    assert!(!debug.contains("private-exit-policy"));
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("output")?;
    for native_value in [
        "pod-runtime-name",
        "pod.service",
        "pod-network",
        "/pod-cache",
        "keep-id",
    ] {
        assert!(
            !output.text().contains(native_value),
            "leaked {native_value} onto a service"
        );
    }
    Ok(())
}

#[test]
fn reports_rootfs_without_replacing_it_with_an_image_and_keeps_conflicts_invalid() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("rootfs.container")?);
    let mut rootfs_service = Service::new(Identifier::new("rootfs")?);
    rootfs_service.set_rootfs(Sourced::from_source(
        ProtectedString::sensitive("/private/rootfs"),
        origin.clone(),
    ))?;
    let mut application = Application::new(Identifier::new("rootfs")?);
    application.add_service(Sourced::from_source(rootfs_service, origin.clone()))?;
    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.rootfs.rootfs"
            && outcome.kind() == ConversionKind::Unsupported
            && outcome.origins() == [origin.clone()]
    }));
    assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());

    let mut conflicting = Service::new(Identifier::new("conflicting")?);
    conflicting.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    assert!(
        conflicting
            .set_rootfs(Sourced::from_source(ProtectedString::plain("/rootfs"), origin))
            .is_err()
    );
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

#[test]
fn generates_typed_network_fields_preserves_explicit_empties_and_reports_ipam_loss() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("network.yaml")?);
    let mut network = Network::new(Identifier::new("front")?, ResourceOwnership::Application);
    network.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("runtime-front"),
        origin.clone(),
    ));
    network.set_driver(Sourced::from_source(ProtectedString::plain("bridge"), origin.clone()));
    network.set_driver_options_with_origins(
        vec![Sourced::from_source(
            NetworkDriverOption::new(
                Sourced::from_source(Identifier::new("com.example.secret")?, origin.clone()),
                Sourced::from_source(ProtectedString::sensitive("production-secret"), origin.clone()),
            )?,
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    network.set_labels_with_origins(Vec::new(), vec![origin.clone()]);
    network.set_internal(Sourced::from_source(false, origin.clone()));
    network.set_ipv6(Sourced::from_source(true, origin.clone()));
    network.set_ipam_driver(Sourced::from_source(ProtectedString::plain("default"), origin.clone()));
    network.set_ipam_configs_with_origins(
        vec![Sourced::from_source(
            NetworkIpamConfig::new(Sourced::from_source(
                ProtectedString::plain("192.0.2.0/24"),
                origin.clone(),
            ))?,
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    let mut application = Application::new(Identifier::new("network-output")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    application.add_service(Sourced::from_source(service, origin.clone()))?;
    application.add_network(Sourced::from_source(network, origin.clone()))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    for subject in ["networks.front.ipam.driver", "networks.front.ipam.config"] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported)
        );
    }
    let debug = format!("{plan:?}");
    assert!(!debug.contains("production-secret"));
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial output")?;
    assert_eq!(
        output.text(),
        concat!(
            "name: \"network-output\"\n",
            "services:\n",
            "  \"web\":\n",
            "    image: \"example.invalid/web:1\"\n",
            "networks:\n",
            "  \"front\":\n",
            "    name: \"runtime-front\"\n",
            "    driver: \"bridge\"\n",
            "    driver_opts:\n",
            "      \"com.example.secret\": \"production-secret\"\n",
            "    enable_ipv6: true\n",
            "    internal: false\n",
            "    labels: {}\n",
        )
    );
    Ok(())
}

#[test]
fn external_network_emits_only_its_explicit_runtime_name() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("external-network.yaml")?);
    let mut network = Network::new(Identifier::new("existing")?, ResourceOwnership::External);
    network.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("platform-existing"),
        origin.clone(),
    ));
    network.set_driver(Sourced::from_source(ProtectedString::plain("bridge"), origin.clone()));
    let mut application = Application::new(Identifier::new("external-network")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::from_source(
        ImageReference::parse("example.invalid/web:1")?,
        origin.clone(),
    ));
    application.add_service(Sourced::from_source(service, origin.clone()))?;
    application.add_network(Sourced::from_source(network, origin.clone()))?;
    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(plan.outcomes().iter().any(
        |outcome| outcome.subject() == "networks.existing.driver" && outcome.kind() == ConversionKind::Unsupported
    ));
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial output")?;
    assert!(output.text().contains("external: true"));
    assert!(output.text().contains("name: \"platform-existing\""));
    assert!(!output.text().contains("driver: \"bridge\""));
    Ok(())
}

#[test]
fn exports_local_volume_driver_fields_runtime_name_and_explicit_empty_labels() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("volume-export.yaml")?);
    let mut volume = Volume::new(Identifier::new("data")?, ResourceOwnership::Application);
    volume.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("platform-data"),
        origin.clone(),
    ));
    volume.set_driver(Sourced::from_source(ProtectedString::plain("local"), origin.clone()));
    volume.set_volume_type(Sourced::from_source(ProtectedString::plain("none"), origin.clone()));
    volume.set_device(Sourced::from_source(
        ProtectedString::plain("/srv/data"),
        origin.clone(),
    ));
    volume.set_options(Sourced::from_source(ProtectedString::plain("bind"), origin.clone()));
    volume.set_labels_with_origins(Vec::new(), vec![origin.clone()]);
    let mut application = minimal_application()?;
    application.add_volume(Sourced::from_source(volume, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("exact output")?;
    assert!(output.text().contains(concat!(
        "volumes:\n",
        "  \"data\":\n",
        "    name: \"platform-data\"\n",
        "    driver: \"local\"\n",
        "    driver_opts:\n",
        "      \"type\": \"none\"\n",
        "      \"device\": \"/srv/data\"\n",
        "      \"o\": \"bind\"\n",
        "    labels: {}\n",
    )));
    Ok(())
}

#[test]
fn external_volume_emits_only_name_and_reports_quadlet_only_configuration() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("external-volume.unit")?);
    let mut volume = Volume::new(Identifier::new("data")?, ResourceOwnership::External);
    volume.set_runtime_name(Sourced::from_source(
        ProtectedString::plain("platform-data"),
        origin.clone(),
    ));
    volume.set_driver(Sourced::from_source(ProtectedString::plain("local"), origin.clone()));
    volume.set_copy(Sourced::from_source(false, origin.clone()));
    volume.set_podman_args_with_origins(
        vec![Sourced::from_source(
            ProtectedString::sensitive("--private"),
            origin.clone(),
        )],
        vec![origin.clone()],
    );
    let mut application = minimal_application()?;
    application.add_volume(Sourced::from_source(volume, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    for subject in ["volumes.data.driver", "volumes.data.copy", "volumes.data.podman_args"] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }
    assert!(!format!("{plan:?}").contains("--private"));
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial output")?;
    assert!(output.text().contains(concat!(
        "volumes:\n",
        "  \"data\":\n",
        "    name: \"platform-data\"\n",
        "    external: true\n",
    )));
    assert!(!output.text().contains("driver:"));
    Ok(())
}

#[test]
fn refuses_sensitive_volume_runtime_names_without_leaking_them() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("private-volume.unit")?);
    let mut application_owned = Volume::new(Identifier::new("application-data")?, ResourceOwnership::Application);
    application_owned.set_runtime_name(Sourced::from_source(
        ProtectedString::sensitive("private-application-volume"),
        origin.clone(),
    ));
    let mut external = Volume::new(Identifier::new("external-data")?, ResourceOwnership::External);
    external.set_runtime_name(Sourced::from_source(
        ProtectedString::sensitive("private-external-volume"),
        origin.clone(),
    ));
    let mut application = minimal_application()?;
    application.add_volume(Sourced::from_source(application_owned, origin.clone()))?;
    application.add_volume(Sourced::from_source(external, origin))?;

    let plan = ComposeExporter::new()?.plan(&application, &exact_target(DOCKER_COMPOSE_TARGET, version(2, 30, 0))?)?;
    for subject in ["volumes.application-data.name", "volumes.external-data.name"] {
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }
    let debug = format!("{plan:?}");
    assert!(!debug.contains("private-application-volume"));
    assert!(!debug.contains("private-external-volume"));
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial output")?;
    assert!(!output.text().contains("private-application-volume"));
    assert!(!output.text().contains("private-external-volume"));
    assert!(output.text().contains("  \"application-data\": {}\n"));
    assert!(output.text().contains("  \"external-data\":\n    external: true\n"));
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
