//! Quadlet-to-neutral-model adapter integration tests.

use boxferry_engine::{ConversionKind, ImportAdapter, Severity};
use boxferry_model::{
    Application, Command, EnvironmentFileSyntax, EnvironmentValue, HealthcheckCommand, Identifier, MountSource,
    Protocol, ResourceGrantSyntax, ResourceOwnership, RestartPolicy, SecurityOption, SelinuxRelabel, Service,
    ServiceDependencyCondition, SourceId,
};
use boxferry_quadlet::{QuadletDocumentInput, QuadletImporter, QuadletSource, QuadletSourceError};
use quadlet_lens::source::SourceId as QuadletSourceId;

#[test]
fn imports_the_first_direct_quadlet_subset_with_provenance() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                "[Container]\nImage=registry.example/team/web:1.2@sha256:abcd\nContainerName=web-runtime\n",
            ),
            QuadletDocumentInput::new("frontend.network", QuadletSourceId::new(2), "[Network]\n"),
            QuadletDocumentInput::new("data.volume", QuadletSourceId::new(3), "[Volume]\n"),
        ],
    )
    .map_err(|error| error.to_string())?
    .with_source_id(
        QuadletSourceId::new(1),
        SourceId::new("quadlet/web.container").map_err(|error| error.to_string())?,
    );

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );

    let application = result.application().ok_or("expected imported application")?;
    assert_eq!(application.name().as_str(), "example");
    assert_eq!(application.services().len(), 1);
    let service = application.services()[0].value();
    assert_eq!(service.name().as_str(), "web");
    assert_eq!(
        service.image().map(|image| image.value().as_str()),
        Some("registry.example/team/web:1.2@sha256:abcd")
    );
    assert_eq!(
        service.runtime_name().map(|name| name.value().expose()),
        Some("web-runtime")
    );
    assert_eq!(
        service
            .image()
            .and_then(|image| image.origins().first())
            .map(|origin| origin.source_id().as_str()),
        Some("quadlet/web.container")
    );
    assert!(
        service
            .image()
            .and_then(|image| image.origins().first())
            .and_then(boxferry_model::Provenance::span)
            .is_some()
    );
    assert_eq!(
        application.networks()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert_eq!(application.networks()[0].value().name().as_str(), "frontend");
    assert_eq!(
        application.volumes()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert_eq!(application.volumes()[0].value().name().as_str(), "data");
    Ok(())
}

#[test]
fn imports_owned_pod_membership_independently_of_document_order() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                "[Container]\nImage=example.invalid/web:1\nPod=application.pod\n",
            ),
            QuadletDocumentInput::new(
                "worker.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/worker:1\nPod=application.pod\n",
            ),
            QuadletDocumentInput::new(
                "application.pod",
                QuadletSourceId::new(3),
                "[Pod]\nPodName=application\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?
    .with_source_id(
        QuadletSourceId::new(1),
        SourceId::new("quadlet/web.container").map_err(|error| error.to_string())?,
    )
    .with_source_id(
        QuadletSourceId::new(2),
        SourceId::new("quadlet/worker.container").map_err(|error| error.to_string())?,
    )
    .with_source_id(
        QuadletSourceId::new(3),
        SourceId::new("quadlet/application.pod").map_err(|error| error.to_string())?,
    );

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );

    let application = result.application().ok_or("expected imported application")?;
    assert_eq!(application.service_groups().len(), 1);
    let group = &application.service_groups()[0];
    assert_eq!(group.value().name().as_str(), "application");
    assert_eq!(group.value().ownership(), ResourceOwnership::Application);
    assert_eq!(
        group
            .value()
            .members()
            .iter()
            .map(|member| member.value().as_str())
            .collect::<Vec<_>>(),
        ["web", "worker"]
    );
    assert_eq!(group.origins()[0].source_id().as_str(), "quadlet/application.pod");
    assert_eq!(
        group.value().members()[0].origins()[0].source_id().as_str(),
        "quadlet/web.container"
    );
    Ok(())
}

#[test]
fn keeps_unrepresentable_pod_defaults_and_pod_scoped_values_explicit() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "application.pod",
                QuadletSourceId::new(1),
                "[Pod]\nPublishPort=8080:80\n",
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/web:1\nPod=application.pod\nPod=application.pod\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    assert_eq!(application.service_groups().len(), 1);
    assert_eq!(application.service_groups()[0].value().members().len(), 1);
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
            .count(),
        1
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Exact)
            .filter(|outcome| outcome.subject() == "services.web.service_group")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn retains_ordered_absolute_environment_file_declarations_without_reading_them() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "EnvironmentFile=/definitely/not/read/default.env\n",
                "EnvironmentFile=/run/credentials/private.env\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    let environment_files = application.services()[0].value().environment_files();
    assert_eq!(environment_files.len(), 2);
    assert_eq!(
        environment_files
            .iter()
            .map(|declaration| declaration.value().path().expose())
            .collect::<Vec<_>>(),
        ["/definitely/not/read/default.env", "/run/credentials/private.env"]
    );
    assert!(environment_files.iter().all(
        |declaration| declaration.value().syntax() == EnvironmentFileSyntax::Short
            && declaration.value().required().is_none()
            && declaration.value().format().is_none()
    ));
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Approximate)
            .count(),
        2
    );
    assert_eq!(
        result
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFQ1004")
            .count(),
        2
    );
    assert!(!format!("{environment_files:?}").contains("private.env"));
    Ok(())
}

#[test]
fn leaves_context_dependent_environment_file_paths_explicit() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "EnvironmentFile=./relative.env\n",
                "EnvironmentFile=%h/private.env\n",
                "EnvironmentFile=/path requiring/native-quoting.env\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(
        result.application().ok_or("expected application")?.services()[0]
            .value()
            .environment_files()
            .is_empty()
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        3
    );
    assert!(!format!("{:?}", result.diagnostics()).contains("private.env"));
    Ok(())
}

#[test]
fn reports_every_unmapped_quadlet_entry_instead_of_dropping_it() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Unit]\nDescription=Web\n[Container]\nImage=example/web:1\nExec=serve --port 8080\nEnvironment=TOKEN=never-print-this\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);

    assert!(result.application().is_some());
    let unsupported: Vec<_> = result
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .map(boxferry_engine::ConversionOutcome::subject)
        .collect();
    assert_eq!(unsupported, ["services.web.quadlet.Description"]);
    assert_eq!(result.diagnostics().len(), 1);
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() == "BFQ1003" && diagnostic.severity() == Severity::Warning)
    );
    assert!(!format!("{:?}", result.diagnostics()).contains("never-print-this"));
    let service = &result.application().ok_or("expected application")?.services()[0];
    assert!(matches!(
        service.value().command().map(boxferry_model::Sourced::value),
        Some(Command::Exec(_))
    ));
    assert_eq!(service.value().environment().len(), 1);
    assert!(!format!("{service:?}").contains("never-print-this"));
    Ok(())
}

#[test]
fn reports_released_container_settings_until_quadlet_import_mapping_is_defined() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\nImage=example/web:1\nHostName=web.example\nPidsLimit=42\nShmSize=64m\n",
                "DropCapability=NET_RAW\nAddCapability=SYS_PTRACE\nTmpfs=/run:mode=1777\n",
                "Sysctl=net.ipv4.ip_forward=1\nUlimit=nofile=1024:4096\n",
                "AddDevice=/dev/fuse:/dev/fuse:rwm\nStopSignal=SIGTERM\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let unsupported = result
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .map(boxferry_engine::ConversionOutcome::subject)
        .collect::<Vec<_>>();
    assert_eq!(unsupported.len(), 10);
    for key in [
        "HostName",
        "PidsLimit",
        "ShmSize",
        "DropCapability",
        "AddCapability",
        "Tmpfs",
        "Sysctl",
        "Ulimit",
        "AddDevice",
        "StopSignal",
    ] {
        assert!(
            unsupported.iter().any(|subject| subject.ends_with(key)),
            "missing {key}: {unsupported:?}"
        );
    }
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() == "BFQ1003")
    );
    Ok(())
}

#[test]
fn imports_the_shared_command_environment_port_mount_and_network_subset() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "Exec=serve --port=8080\n",
                    "Environment=TOKEN=private-value\n",
                    "Label=org.example.role=frontend\n",
                    "AddHost=host.docker.internal:host-gateway\n",
                    "AddHost=database;database.internal:[2001:db8::10]\n",
                    "User=1001\n",
                    "Group=1002\n",
                    "GroupAdd=44\n",
                    "GroupAdd=keep-groups\n",
                    "UserNS=keep-id:uid=1001,gid=1002\n",
                    "WorkingDir=/srv/app\n",
                    "ReadOnly=true\n",
                    "PublishPort=127.0.0.1:8080:80/tcp\n",
                    "PublishPort=53/udp\n",
                    "Volume=data.volume:/var/lib/data:ro,Z\n",
                    "Volume=/srv/config:/etc/config:z\n",
                    "Volume=shared-data:/srv/shared\n",
                    "Network=frontend.network\n",
                    "Network=external-net\n",
                ),
            ),
            QuadletDocumentInput::new("frontend.network", QuadletSourceId::new(2), "[Network]\n"),
            QuadletDocumentInput::new("data.volume", QuadletSourceId::new(3), "[Volume]\n"),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );

    let application = result.application().ok_or("expected imported application")?;
    let service = application.services()[0].value();
    assert_command_and_environment(service)?;
    assert_metadata_and_execution_context(service);
    assert_ports_mounts_and_networks(application);
    Ok(())
}

fn assert_metadata_and_execution_context(service: &Service) {
    let label = service.labels()[0].value();
    assert_eq!(label.name().as_str(), "org.example.role");
    assert_eq!(label.value().expose(), "frontend");
    assert_eq!(
        service
            .host_mappings()
            .iter()
            .map(|mapping| (mapping.value().hostname().as_str(), mapping.value().address().raw()))
            .collect::<Vec<_>>(),
        [
            ("host.docker.internal", "host-gateway"),
            ("database", "[2001:db8::10]"),
            ("database.internal", "[2001:db8::10]"),
        ]
    );
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(
        service
            .supplementary_groups()
            .iter()
            .map(|group| group.value().expose())
            .collect::<Vec<_>>(),
        ["44", "keep-groups"]
    );
    assert_eq!(
        service.user_namespace().map(|value| value.value().expose()),
        Some("keep-id:uid=1001,gid=1002")
    );
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/srv/app")
    );
    assert_eq!(
        service.read_only_root_filesystem().map(boxferry_model::Sourced::value),
        Some(&true)
    );
}

fn assert_command_and_environment(service: &Service) -> Result<(), String> {
    let Some(Command::Exec(arguments)) = service.command().map(boxferry_model::Sourced::value) else {
        return Err("expected an exec-form command".to_owned());
    };
    assert_eq!(
        arguments
            .iter()
            .map(boxferry_model::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["serve", "--port=8080"]
    );
    assert_eq!(service.environment()[0].value().name().as_str(), "TOKEN");
    assert!(matches!(
        service.environment()[0].value().value(),
        EnvironmentValue::Literal(value) if value.expose() == "private-value"
    ));
    assert!(!format!("{service:?}").contains("private-value"));
    Ok(())
}

fn assert_ports_mounts_and_networks(application: &Application) {
    let service = application.services()[0].value();
    let port = service.ports()[0].value();
    assert_eq!(port.host_address(), Some("127.0.0.1"));
    assert_eq!(port.published(), Some(8080));
    assert_eq!(port.container(), 80);
    assert_eq!(port.protocol(), &Protocol::Tcp);
    let random_port = service.ports()[1].value();
    assert_eq!(random_port.host_address(), None);
    assert_eq!(random_port.published(), None);
    assert_eq!(random_port.container(), 53);
    assert_eq!(random_port.protocol(), &Protocol::Udp);

    assert_eq!(service.mounts().len(), 3);
    assert!(matches!(
        service.mounts()[0].value().source(),
        MountSource::Volume(name) if name.as_str() == "data"
    ));
    assert!(service.mounts()[0].value().read_only());
    assert_eq!(
        service.mounts()[0].value().selinux_relabel(),
        Some(SelinuxRelabel::Private)
    );
    assert!(matches!(
        service.mounts()[1].value().source(),
        MountSource::HostPath(path) if path == "/srv/config"
    ));
    assert_eq!(
        service.mounts()[1].value().selinux_relabel(),
        Some(SelinuxRelabel::Shared)
    );
    assert!(matches!(
        service.mounts()[2].value().source(),
        MountSource::Volume(name) if name.as_str() == "shared-data"
    ));

    assert_eq!(
        service
            .networks()
            .iter()
            .map(|network| network.value().network().as_str())
            .collect::<Vec<_>>(),
        ["frontend", "external-net"]
    );
    assert_eq!(
        application.networks()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert_eq!(
        application.networks()[1].value().ownership(),
        ResourceOwnership::External
    );
    assert_eq!(
        application.volumes()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert_eq!(application.volumes()[1].value().name().as_str(), "shared-data");
    assert_eq!(
        application.volumes()[1].value().ownership(),
        ResourceOwnership::External
    );
}

#[test]
fn ambiguous_native_forms_remain_explicit_and_do_not_enter_the_neutral_model() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "Exec=sh -c \"echo private-command\"\n",
                "Environment=\"TOKEN=private value\"\n",
                "PublishPort=[::1]:8080:80/tcp\n",
                "Volume=%h/data:/srv/data\n",
                "Network=host\n",
                "Network=bridge\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    let service = application.services()[0].value();
    assert!(service.command().is_none());
    assert!(service.environment().is_empty());
    assert!(service.ports().is_empty());
    assert!(service.mounts().is_empty());
    assert!(service.networks().is_empty());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        6
    );
    let debug = format!("{:?}", result.diagnostics());
    assert!(!debug.contains("private-command"));
    assert!(!debug.contains("private value"));
    Ok(())
}

#[test]
fn continued_scalar_values_are_not_silently_truncated() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "image.container",
                QuadletSourceId::new(1),
                "[Container]\nImage=example.invalid/\\\n web:1\n",
            ),
            QuadletDocumentInput::new(
                "name.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/name:1\nContainerName=name-\\\n runtime\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    assert!(application.services()[0].value().image().is_none());
    assert!(application.services()[1].value().runtime_name().is_none());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert!(result.diagnostics().iter().all(|diagnostic| {
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "reason" && field.value().redacted().contains("continued physical values"))
    }));
    Ok(())
}

#[test]
fn invalid_singleton_and_execution_context_combinations_do_not_enter_the_model() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "Image=example.invalid/replacement:1\n",
                "Group=1002\n",
                "ReadOnly=yes\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result.application().ok_or("expected imported application")?.services()[0].value();
    assert_eq!(
        service.image().map(|image| image.value().as_str()),
        Some("example.invalid/web:1")
    );
    assert!(service.group().is_none());
    assert!(service.read_only_root_filesystem().is_none());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
            .count(),
        3
    );
    Ok(())
}

#[test]
fn imports_complete_sibling_dependencies_and_the_exact_no_restart_policy() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                concat!(
                    "[Unit]\n",
                    "Wants=cache.container\n",
                    "After=backend.container cache.container\n",
                    "Requires=backend.service\n",
                    "[Service]\n",
                    "Restart=no\n",
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                ),
            ),
            QuadletDocumentInput::new(
                "cache.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/cache:1\n",
            ),
            QuadletDocumentInput::new(
                "backend.container",
                QuadletSourceId::new(3),
                "[Container]\nImage=example.invalid/backend:1\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );

    let application = result.application().ok_or("expected imported application")?;
    let web = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "web")
        .ok_or("expected web service")?
        .value();
    assert_eq!(
        web.restart_policy().map(boxferry_model::Sourced::value),
        Some(&RestartPolicy::Never)
    );
    assert_eq!(
        web.dependencies()
            .iter()
            .map(|dependency| dependency.value().service().as_str())
            .collect::<Vec<_>>(),
        ["cache", "backend"]
    );
    assert_eq!(
        web.dependencies()
            .iter()
            .map(|dependency| dependency.value().is_required())
            .collect::<Vec<_>>(),
        [false, true]
    );
    assert!(web.dependencies().iter().all(|dependency| {
        dependency.value().condition().map(boxferry_model::Sourced::value) == Some(&ServiceDependencyCondition::Started)
            && dependency.origins().len() == 2
    }));
    Ok(())
}

#[test]
fn reports_systemd_restart_approximations_and_non_application_dependencies() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "always.container",
                QuadletSourceId::new(1),
                "[Service]\nRestart=always\n[Container]\nImage=example.invalid/always:1\n",
            ),
            QuadletDocumentInput::new(
                "failure.container",
                QuadletSourceId::new(2),
                "[Service]\nRestart=on-failure\n[Container]\nImage=example.invalid/failure:1\n",
            ),
            QuadletDocumentInput::new(
                "other.container",
                QuadletSourceId::new(3),
                concat!(
                    "[Unit]\n",
                    "Requires=network-online.target\n",
                    "After=network-online.target\n",
                    "[Service]\n",
                    "Restart=on-success\n",
                    "[Container]\n",
                    "Image=example.invalid/other:1\n",
                ),
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    let always = application.services()[0].value();
    let failure = application.services()[1].value();
    let other = application.services()[2].value();
    assert_eq!(
        always.restart_policy().map(boxferry_model::Sourced::value),
        Some(&RestartPolicy::Always)
    );
    assert_eq!(
        failure.restart_policy().map(boxferry_model::Sourced::value),
        Some(&RestartPolicy::on_failure(None))
    );
    assert!(other.restart_policy().is_none());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Approximate)
            .count(),
        2
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        3
    );
    assert_eq!(
        result
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFQ1004")
            .count(),
        2
    );
    Ok(())
}

#[test]
fn incomplete_or_conflicting_systemd_dependency_pairs_do_not_enter_the_model() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "required-only.container",
                QuadletSourceId::new(1),
                "[Unit]\nRequires=backend.container\n[Container]\nImage=example.invalid/required:1\n",
            ),
            QuadletDocumentInput::new(
                "after-only.container",
                QuadletSourceId::new(2),
                "[Unit]\nAfter=backend.container\n[Container]\nImage=example.invalid/after:1\n",
            ),
            QuadletDocumentInput::new(
                "conflict.container",
                QuadletSourceId::new(3),
                concat!(
                    "[Unit]\n",
                    "Requires=backend.container\n",
                    "Wants=backend.container\n",
                    "After=backend.container\n",
                    "[Container]\n",
                    "Image=example.invalid/conflict:1\n",
                ),
            ),
            QuadletDocumentInput::new(
                "backend.container",
                QuadletSourceId::new(4),
                "[Container]\nImage=example.invalid/backend:1\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    assert!(application.services()[0].value().dependencies().is_empty());
    assert!(application.services()[1].value().dependencies().is_empty());
    assert!(application.services()[2].value().dependencies().is_empty());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn imports_regular_quadlet_health_checks_and_explicit_disable_intent() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "HealthCmd=[\"CMD\",\"curl\",\"--fail\",\"http://127.0.0.1:8080/health\"]\n",
                    "HealthInterval=30s\n",
                    "HealthTimeout=5s\n",
                    "HealthRetries=3\n",
                    "HealthStartPeriod=10s\n",
                ),
            ),
            QuadletDocumentInput::new(
                "disabled.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/disabled:1\nHealthCmd=none\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let application = result.application().ok_or("expected imported application")?;
    let healthcheck = application.services()[0]
        .value()
        .healthcheck()
        .ok_or("expected regular health check")?
        .value();
    let Some(HealthcheckCommand::Exec(arguments)) = healthcheck.command().map(boxferry_model::Sourced::value) else {
        return Err("expected exec health command".to_owned());
    };
    assert_eq!(
        arguments
            .iter()
            .map(boxferry_model::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["curl", "--fail", "http://127.0.0.1:8080/health"]
    );
    assert_eq!(healthcheck.interval().map(|value| value.value().as_str()), Some("30s"));
    assert_eq!(healthcheck.timeout().map(|value| value.value().as_str()), Some("5s"));
    assert_eq!(healthcheck.retries().map(|value| value.value().as_str()), Some("3"));
    assert_eq!(
        healthcheck.start_period().map(|value| value.value().as_str()),
        Some("10s")
    );
    assert!(!format!("{healthcheck:?}").contains("127.0.0.1"));

    let disabled = application.services()[1]
        .value()
        .healthcheck()
        .ok_or("expected disabled health check")?
        .value();
    assert_eq!(disabled.disabled().map(boxferry_model::Sourced::value), Some(&true));
    assert!(disabled.command().is_none());
    Ok(())
}

#[test]
fn rejects_ambiguous_or_invalid_quadlet_health_values_without_leaking_commands() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "HealthCmd=curl --header=$PRIVATE_TOKEN http://127.0.0.1\n",
                "HealthInterval=disable\n",
                "HealthTimeout=5fortnights\n",
                "HealthRetries=three\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(
        result.application().ok_or("expected application")?.services()[0]
            .value()
            .healthcheck()
            .is_none()
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
            .count(),
        2
    );
    assert!(!format!("{:?}", result.diagnostics()).contains("PRIVATE_TOKEN"));
    Ok(())
}

#[test]
fn imports_external_podman_mount_secrets_and_ordered_grant_options() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "Secret=db-password\n",
                    "Secret=tls-cert,type=mount,target=/etc/tls/cert.pem,uid=1001,gid=1002,mode=0440\n",
                ),
            ),
            QuadletDocumentInput::new(
                "worker.container",
                QuadletSourceId::new(2),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/worker:1\n",
                    "Secret=db-password,target=worker-password\n",
                ),
            ),
        ],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let application = result.application().ok_or("expected imported application")?;
    assert_eq!(application.secrets().len(), 2);
    assert!(application.secrets().iter().all(|secret| {
        secret.value().ownership() == ResourceOwnership::External && secret.value().material().is_none()
    }));
    assert_eq!(
        application
            .secrets()
            .iter()
            .map(|secret| secret.value().name().as_str())
            .collect::<Vec<_>>(),
        ["db-password", "tls-cert"]
    );

    let web = application.services()[0].value();
    assert_eq!(web.secret_grants().len(), 2);
    let short = web.secret_grants()[0].value();
    assert_eq!(short.source().expose(), "db-password");
    assert_eq!(short.syntax(), ResourceGrantSyntax::Short);
    let long = web.secret_grants()[1].value();
    assert_eq!(long.source().expose(), "tls-cert");
    assert_eq!(long.syntax(), ResourceGrantSyntax::Long);
    assert_eq!(
        long.target().map(|value| value.value().expose()),
        Some("/etc/tls/cert.pem")
    );
    assert_eq!(long.uid().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(long.gid().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(long.mode().map(|value| value.value().expose()), Some("0440"));
    assert_eq!(
        application.services()[1].value().secret_grants()[0]
            .value()
            .target()
            .map(|value| value.value().expose()),
        Some("worker-password")
    );
    Ok(())
}

#[test]
fn leaves_environment_and_unreviewed_secret_forms_explicit() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "Secret=api-key,type=env,target=API_KEY\n",
                "Secret=provider-secret,provider=example\n",
                "Secret=invalid-owner,uid=owner\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;

    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("expected imported application")?;
    assert!(application.secrets().is_empty());
    assert!(application.services()[0].value().secret_grants().is_empty());
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn rejects_an_unresolved_native_document_graph() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Container]\nImage=example/web:1\nNetwork=missing.network\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    assert!(!source.documents().is_valid());
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);

    assert!(result.application().is_none());
    assert_eq!(result.outcomes()[0].kind(), ConversionKind::Invalid);
    assert_eq!(result.diagnostics()[0].code().as_str(), "BFQ1001");
    assert_eq!(result.diagnostics()[0].severity(), Severity::Error);
    Ok(())
}

#[test]
fn parse_boundary_rejects_malformed_input_before_import() -> Result<(), String> {
    let error = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Container]\nImage\n",
        )],
    )
    .err()
    .ok_or("malformed source should fail")?;
    assert!(matches!(error, QuadletSourceError::InvalidDocument { .. }));
    Ok(())
}

#[test]
fn imports_ordered_dns_keys_and_treats_empty_assignments_as_resets() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Container]\nImage=example.invalid/web:1\nDNS=1.1.1.1\nDNS=8.8.8.8\nDNS=\nDNS=9.9.9.9\nDNSOption=ndots:5\nDNSOption=\nDNSSearch=example.test\nDNSSearch=\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .ok_or("service expected")?
        .value();
    assert_eq!(
        service
            .dns_servers()
            .unwrap_or_default()
            .iter()
            .map(|value| value.value().expose())
            .collect::<Vec<_>>(),
        ["9.9.9.9"]
    );
    assert!(service.dns_options().is_some_and(<[_]>::is_empty));
    assert!(service.dns_search_domains().is_some_and(<[_]>::is_empty));
    assert_eq!(service.dns_servers_origins().len(), 2);
    assert_eq!(service.dns_options_origins().len(), 1);
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.dns" && outcome.kind() == ConversionKind::Unsupported)
    );
    Ok(())
}

#[test]
fn imports_security_options_in_native_order_with_sensitive_provenance() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "AppArmor=profile\n",
                "NoNewPrivileges=yes\n",
                "SeccompProfile=unconfined\n",
                "SecurityLabelDisable=false\n",
                "SecurityLabelFileType=container_file_t\n",
                "SecurityLabelLevel=s0:c1,c2\n",
                "SecurityLabelNested=true\n",
                "SecurityLabelType=container_t\n",
                "Mask=/proc/acpi:/sys/firmware\n",
                "Unmask=ALL\n",
                "Mask=/proc/acpi:/sys/firmware\n",
                "Unmask=ALL\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?
    .with_source_id(
        QuadletSourceId::new(1),
        SourceId::new("quadlet/security.container").map_err(|error| error.to_string())?,
    );
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .ok_or("service expected")?
        .value();
    let options = service.security_options().ok_or("security options expected")?;
    assert_eq!(options.len(), 12);
    assert!(matches!(options[0].value(), SecurityOption::AppArmor(value) if value.expose() == "profile"));
    assert!(matches!(options[1].value(), SecurityOption::NoNewPrivileges(true)));
    assert!(matches!(options[2].value(), SecurityOption::SeccompProfile(value) if value.expose() == "unconfined"));
    assert!(matches!(
        options[3].value(),
        SecurityOption::SecurityLabelDisable(false)
    ));
    assert!(
        matches!(options[4].value(), SecurityOption::SecurityLabelFileType(value) if value.expose() == "container_file_t")
    );
    assert!(matches!(options[5].value(), SecurityOption::SecurityLabelLevel(value) if value.expose() == "s0:c1,c2"));
    assert!(matches!(options[6].value(), SecurityOption::SecurityLabelNested(true)));
    assert!(matches!(options[7].value(), SecurityOption::SecurityLabelType(value) if value.expose() == "container_t"));
    assert!(matches!(options[8].value(), SecurityOption::Mask(value) if value.expose() == "/proc/acpi:/sys/firmware"));
    assert!(matches!(options[9].value(), SecurityOption::Unmask(value) if value.expose() == "ALL"));
    assert!(matches!(options[10].value(), SecurityOption::Mask(value) if value.expose() == "/proc/acpi:/sys/firmware"));
    assert!(matches!(options[11].value(), SecurityOption::Unmask(value) if value.expose() == "ALL"));
    assert_eq!(service.security_options_origins().len(), 12);
    assert_eq!(
        options[0].origins()[0].source_id().as_str(),
        "quadlet/security.container"
    );
    assert!(!format!("{options:?}").contains("container_file_t"));
    Ok(())
}

#[test]
fn keeps_security_option_resets_unsafe_values_singletons_and_selinux_conflicts_explicit() -> Result<(), String> {
    let source = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "Mask=\n",
                "NoNewPrivileges=not-a-boolean\n",
                "SeccompProfile=\"quoted\"\n",
                "AppArmor=first\n",
                "AppArmor=second\n",
                "SecurityLabelDisable=true\n",
                "SecurityLabelType=container_t\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .ok_or("service expected")?
        .value();
    let options = service
        .security_options()
        .ok_or("security options should remain explicitly present")?;
    assert_eq!(options.len(), 3);
    assert!(matches!(options[0].value(), SecurityOption::AppArmor(value) if value.expose() == "first"));
    assert!(matches!(options[1].value(), SecurityOption::SecurityLabelDisable(true)));
    assert!(matches!(options[2].value(), SecurityOption::SecurityLabelType(value) if value.expose() == "container_t"));
    assert_eq!(service.security_options_origins().len(), 6);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_options[0]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_options.apparmor" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_options" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

fn identifier(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}
