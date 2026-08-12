//! Quadlet-to-neutral-model adapter integration tests.

use boxferry_engine::{ConversionKind, ImportAdapter, Severity};
use boxferry_model::{
    Application, Command, Entrypoint, EnvironmentFileSyntax, EnvironmentValue, HealthcheckCommand, Identifier,
    ImageAcquisitionSetting, ImageBuildSetting, MountSource, Protocol, PullPolicy, ReloadAction, ResourceGrantSyntax,
    ResourceOwnership, RestartPolicy, SecurityOption, SelinuxRelabel, Service, ServiceDependencyCondition, SourceId,
    StartupNotification, VolumeImageSource,
};
use boxferry_quadlet::{
    QuadletDocumentInput, QuadletImporter, QuadletParseDiagnosticOrigin, QuadletParseDiagnosticSeverity,
    QuadletParseError, QuadletParseFailureStage, QuadletParseResult, QuadletSource,
};
use quadlet_lens::model::{NamedQuadletDocument, QuadletDocument, QuadletDocumentSet, QuadletUnitType};
use quadlet_lens::source::SourceId as QuadletSourceId;

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, QuadletParseError> {
    QuadletSource::parse(application_name, inputs).map(QuadletParseResult::into_source)
}

#[test]
fn imports_the_first_direct_quadlet_subset_with_provenance() -> Result<(), String> {
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let runtime = application.service_groups()[0]
        .value()
        .runtime()
        .ok_or("pod runtime expected")?
        .value();
    assert_eq!(runtime.ports().map(<[_]>::len), Some(1));
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
fn retains_rootfs_notify_and_ordered_podman_args_without_synthesizing_them() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\n",
                "Rootfs=/srv/rootfs\n",
                "Notify=false\n",
                "PodmanArgs=--replace\n",
                "PodmanArgs=--secret=private\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result
        .application()
        .and_then(|app| app.services().first())
        .ok_or("service expected")?
        .value();
    assert!(service.rootfs().is_some());
    assert!(matches!(
        service.startup_notification().map(boxferry_model::Sourced::value),
        Some(StartupNotification::Runtime)
    ));
    assert_eq!(service.podman_args().map(<[_]>::len), Some(2));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.startup_notification" && outcome.kind() == ConversionKind::Exact
    }));
    assert!(!format!("{service:?}").contains("private"));
    Ok(())
}

#[test]
fn retains_pod_resets_omitted_name_and_host_network_conflict() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "application.pod",
                QuadletSourceId::new(1),
                concat!(
                    "[Pod]\n",
                    "ServiceName=chosen\n",
                    "AddHost=host.docker.internal:host-gateway\nAddHost=\n",
                    "PublishPort=8080:80\nPublishPort=\n",
                    "Volume=\n",
                    "StopTimeout=01\n",
                ),
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/web:1\nPod=application.pod\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let runtime = result
        .application()
        .and_then(|app| app.service_groups().first())
        .ok_or("group expected")?
        .value()
        .runtime()
        .ok_or("runtime expected")?
        .value();
    assert!(runtime.runtime_name().is_none());
    assert!(runtime.host_mappings().is_some_and(<[_]>::is_empty));
    assert!(runtime.ports().is_some_and(<[_]>::is_empty));
    assert!(runtime.mounts().is_some_and(<[_]>::is_empty));
    assert!(runtime.stop_timeout().is_some());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.application.runtime.StopTimeout"
            && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn reports_pod_host_network_and_published_port_conflict_independently_of_entry_order() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "application.pod",
                QuadletSourceId::new(1),
                "[Pod]\nPublishPort=8080:80\nNetwork=host\n",
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/web:1\nPod=application.pod\n",
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.application.runtime.ports" && outcome.kind() == ConversionKind::Invalid
    }));
    Ok(())
}

#[test]
fn retains_ordered_absolute_environment_file_declarations_without_reading_them() -> Result<(), String> {
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
fn imports_released_container_settings_with_order_and_provenance() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\nImage=example/web:1\nHostName=web.example\nPidsLimit=42\nShmSize=64m\n",
                "DropCapability=NET_RAW\nAddCapability=SYS_PTRACE CAP_CHOWN\nTmpfs=/run:mode=1777\n",
                "Sysctl=net.ipv4.ip_forward=1\nUlimit=nofile=0:0\n",
                "AddDevice=/dev/fuse:/dev/fuse:rwm\nStopSignal=SIGTERM\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let service = result.application().ok_or("expected application")?.services()[0].value();
    assert_eq!(
        service.hostname().map(|value| value.value().expose()),
        Some("web.example")
    );
    assert_eq!(service.pids_limit().map(|value| value.value().expose()), Some("42"));
    assert_eq!(service.shm_size().map(|value| value.value().expose()), Some("64m"));
    assert_eq!(service.cap_drop().map(<[_]>::len), Some(1));
    assert_eq!(service.cap_add().map(<[_]>::len), Some(2));
    assert_eq!(service.tmpfs().map(<[_]>::len), Some(1));
    assert_eq!(service.sysctls().map(<[_]>::len), Some(1));
    assert_eq!(service.ulimits().map(<[_]>::len), Some(1));
    assert_eq!(service.devices().map(<[_]>::len), Some(1));
    assert_eq!(
        service.stop_signal().map(|value| value.value().expose()),
        Some("SIGTERM")
    );
    Ok(())
}

#[test]
fn retains_unreviewed_native_container_values_without_classifying_them_as_exact() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            concat!(
                "[Container]\nImage=example/web:1\nHostName=bad%h\nStopSignal=SIG\n",
                "AddCapability=SYS_PTRACE quoted-value\nTmpfs=relative:mode=1777\n",
                "Ulimit=nofile=00:0\nAddDevice=/dev/fuse:relative:rwm\n",
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
    assert_eq!(
        unsupported,
        [
            "services.web.hostname",
            "services.web.stop_signal",
            "services.web.cap_add",
            "services.web.tmpfs",
            "services.web.ulimits",
            "services.web.devices",
        ]
    );
    let service = result.application().ok_or("expected application")?.services()[0].value();
    assert_eq!(service.hostname().map(|value| value.value().expose()), Some("bad%h"));
    assert_eq!(service.stop_signal().map(|value| value.value().expose()), Some("SIG"));
    assert_eq!(service.cap_add().map(<[_]>::len), Some(1));
    assert_eq!(service.tmpfs().map(<[_]>::len), Some(1));
    assert_eq!(service.ulimits().map(<[_]>::len), Some(1));
    assert_eq!(service.devices().map(<[_]>::len), Some(1));
    Ok(())
}

#[test]
fn imports_extended_container_keys_and_redacts_sensitive_values() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [
            QuadletDocumentInput::new("frontend.network", QuadletSourceId::new(2), "[Network]\n"),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(1),
                concat!(
                    "[Container]\nImage=example/web:1\nNetwork=frontend.network\n",
                    "Entrypoint=[\"/bin/web\",\"serve\"]\nRunInit=true\nStopTimeout=30\nPull=always\nMemory=64m\n",
                    "ExposeHostPort=8080/udp\nAnnotation=io.example.note=private\nLogDriver=journald\nLogOpt=tag=web\n",
                    "IP=192.0.2.10\nIP6=2001:db8::10\nNetworkAlias=web\nReloadSignal=SIGHUP\n",
                ),
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result.application().ok_or("expected application")?.services()[0].value();
    assert!(matches!(
        service.entrypoint().map(boxferry_model::Sourced::value),
        Some(Entrypoint::Exec(_))
    ));
    assert_eq!(service.run_init().map(boxferry_model::Sourced::value), Some(&true));
    assert_eq!(service.stop_timeout().map(|value| value.value().as_str()), Some("30"));
    assert!(matches!(
        service.pull_policy().map(boxferry_model::Sourced::value),
        Some(PullPolicy::Always)
    ));
    assert_eq!(service.exposed_ports().map(<[_]>::len), Some(1));
    assert_eq!(service.annotations().map(<[_]>::len), Some(1));
    assert_eq!(
        service
            .logging()
            .and_then(|logging| logging.value().options())
            .map(<[_]>::len),
        Some(1)
    );
    assert!(matches!(
        service.reload_action().map(boxferry_model::Sourced::value),
        Some(ReloadAction::Signal(_))
    ));
    let network = &service.networks()[0].value();
    assert_eq!(
        network.ipv4_address().map(|value| value.value().expose()),
        Some("192.0.2.10")
    );
    assert_eq!(network.aliases(), ["web"]);
    assert!(!format!("{service:?}").contains("private"));
    Ok(())
}

#[test]
fn network_attachment_values_before_or_after_network_have_identical_meaning() -> Result<(), String> {
    let import = |container: &str| -> Result<_, String> {
        let source = parse_source(
            identifier("example")?,
            [
                QuadletDocumentInput::new("frontend.network", QuadletSourceId::new(2), "[Network]\n"),
                QuadletDocumentInput::new("web.container", QuadletSourceId::new(1), container),
            ],
        )
        .map_err(|error| error.to_string())?;
        Ok(QuadletImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source))
    };
    let before = import(concat!(
        "[Container]\nImage=example/web:1\nIP=192.0.2.10\nNetworkAlias=first\n",
        "NetworkAlias=second\nIP6=2001:db8::10\nNetwork=frontend.network\n",
    ))?;
    let after = import(concat!(
        "[Container]\nImage=example/web:1\nNetwork=frontend.network\nIP=192.0.2.10\n",
        "NetworkAlias=first\nNetworkAlias=second\nIP6=2001:db8::10\n",
    ))?;
    for result in [&before, &after] {
        let network = &result.application().ok_or("expected application")?.services()[0]
            .value()
            .networks()[0]
            .value();
        assert_eq!(
            network.ipv4_address().map(|value| value.value().expose()),
            Some("192.0.2.10")
        );
        assert_eq!(
            network.ipv6_address().map(|value| value.value().expose()),
            Some("2001:db8::10")
        );
        assert_eq!(network.aliases(), ["first", "second"]);
        assert_eq!(network.alias_origins().len(), 2);
    }
    Ok(())
}

#[test]
fn imports_the_shared_command_environment_port_mount_and_network_subset() -> Result<(), String> {
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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
fn parse_keeps_an_incomplete_native_document_graph_with_ordered_document_set_diagnostics() -> Result<(), String> {
    let parsed = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Container]\nImage=example/web:1\nNetwork=missing.network\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    assert!(!parsed.source().documents().is_valid());
    assert_eq!(parsed.diagnostics().len(), 1);
    assert_eq!(
        parsed.diagnostics()[0].origin(),
        QuadletParseDiagnosticOrigin::DocumentSet
    );
    assert_eq!(
        parsed.diagnostics()[0].severity(),
        QuadletParseDiagnosticSeverity::Error
    );
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(parsed.source());

    assert!(result.application().is_none());
    assert_eq!(result.outcomes()[0].kind(), ConversionKind::Invalid);
    assert_eq!(result.diagnostics()[0].code().as_str(), "BFQ1001");
    assert_eq!(result.diagnostics()[0].severity(), Severity::Error);
    Ok(())
}

#[test]
fn parse_boundary_rejects_malformed_input_before_import() -> Result<(), String> {
    let error = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            "[Container]\nImage\n",
        )],
    )
    .err()
    .ok_or("malformed source should fail")?;
    assert!(error.failures().is_empty());
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.origin() == QuadletParseDiagnosticOrigin::Syntax)
    );
    Ok(())
}

#[test]
fn parse_retains_all_native_diagnostics_without_source_contents() -> Result<(), String> {
    let error = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new("first.container", QuadletSourceId::new(7), "[Container]\nImage\n"),
            QuadletDocumentInput::new(
                "second.container",
                QuadletSourceId::new(9),
                "[Container]\nImage=secret-value-canary.image\n",
            ),
        ],
    )
    .err()
    .ok_or("detailed source should fail")?;

    let diagnostics = error.diagnostics();
    assert_eq!(diagnostics[0].origin(), QuadletParseDiagnosticOrigin::Syntax);
    assert_eq!(diagnostics[0].code(), "QLS0001");
    assert_eq!(diagnostics[0].labels()[0].source_id(), 7);
    assert!(diagnostics[0].labels()[0].start() < diagnostics[0].labels()[0].end());
    assert_eq!(diagnostics[1].origin(), QuadletParseDiagnosticOrigin::Model);
    assert_eq!(diagnostics[1].code(), "QLM0002");
    assert_eq!(diagnostics[1].labels()[0].source_id(), 7);
    assert!(diagnostics[1].labels()[0].start() < diagnostics[1].labels()[0].end());
    assert_eq!(diagnostics[2].origin(), QuadletParseDiagnosticOrigin::DocumentSet);
    assert_eq!(diagnostics[2].severity(), QuadletParseDiagnosticSeverity::Error);
    assert_eq!(diagnostics[2].labels()[0].source_id(), 9);
    for diagnostic in diagnostics {
        assert!(!diagnostic.summary().is_empty());
        for label in diagnostic.labels() {
            assert!(label.start() <= label.end());
            assert!(!label.message().is_empty());
        }
    }
    let debug = format!("{error:?}");
    assert!(!debug.contains("secret-value-canary"));
    assert!(!debug.contains("first.container"));
    assert!(error.failures().is_empty());

    Ok(())
}

#[test]
fn parse_keeps_warnings_but_rejects_structured_fatal_metadata() -> Result<(), String> {
    let parsed = QuadletSource::parse(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(8),
            "[Container]\nImage=example.invalid/one:1\nImage=example.invalid/two:1\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    assert!(parsed.source().documents().is_valid());
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.origin() == QuadletParseDiagnosticOrigin::Model
            && diagnostic.severity() == QuadletParseDiagnosticSeverity::Warning
    }));

    let error = QuadletSource::parse(
        identifier("example")?,
        [
            QuadletDocumentInput::new(
                "private/path.container",
                QuadletSourceId::new(15),
                "[Container]\nImage=x\n",
            ),
            QuadletDocumentInput::new("later.container", QuadletSourceId::new(16), "[Container]\nImage=\n"),
        ],
    )
    .err()
    .ok_or("fatal filename should fail")?;
    assert_eq!(error.failures()[0].stage(), QuadletParseFailureStage::Filename);
    assert_eq!(error.failures()[0].input_index(), Some(0));
    assert_eq!(error.failures()[0].source_id(), Some(15));
    assert!(!format!("{error:?}").contains("private/path.container"));
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.labels().iter().any(|label| label.source_id() == 16))
    );
    Ok(())
}

#[test]
fn parse_fixed_seed_corpus_is_panic_free_deterministic_and_span_bounded() -> Result<(), String> {
    for seed in 0_u32..=256 {
        let text = if seed == 191 {
            "[Container]\nImage=\nLabel=seeded-secret-canary\n".to_owned()
        } else {
            match seed % 7 {
                0 => format!("[Container]\nImage=example.invalid/{seed:x}:1\n"),
                1 => "[Container]\nImage=\n".to_owned(),
                2 => "[Container]\nImage\n".to_owned(),
                3 => format!("[Container]\nImage={seed:x}\nNetwork=missing.network\n"),
                4 => format!("[Container]\nImage={seed:x}\nImage={:x}\n", seed + 1),
                5 => format!("[Container]\nImage={seed:x}\nLabel=seed-{seed:x}\n"),
                _ => format!("[Container]\nImage={seed:x}\nEnvironment=K={seed:x}\n"),
            }
        };
        assert!(text.len() <= 64, "seed {seed} exceeded the bounded corpus size");
        let input = QuadletDocumentInput::new("web.container", QuadletSourceId::new(seed + 1), text.clone());
        let application_name = identifier("seeded")?;
        let first = std::panic::catch_unwind({
            let input = input.clone();
            let application_name = application_name.clone();
            move || QuadletSource::parse(application_name, [input])
        })
        .map_err(|_| format!("parser panicked for seed {seed}"))?;
        let second = std::panic::catch_unwind({
            let input = input.clone();
            move || QuadletSource::parse(application_name, [input])
        })
        .map_err(|_| format!("parser panicked for seed {seed}"))?;
        assert_eq!(first, second, "parser classification changed for seed {seed}");
        let diagnostics = match &first {
            Ok(result) => result.diagnostics(),
            Err(error) => error.diagnostics(),
        };
        for diagnostic in diagnostics {
            for label in diagnostic.labels() {
                assert_eq!(label.source_id(), seed + 1);
                assert!(label.start() <= label.end());
                assert!(label.end() <= text.len());
            }
        }
        if let Err(error) = &first {
            for failure in error.failures() {
                if let Some(span) = failure.span() {
                    assert_eq!(span.source_id(), seed + 1);
                    assert!(span.start() <= span.end());
                    assert!(span.end() <= text.len());
                }
            }
        }
        if seed == 191 {
            let error = first.err().ok_or("seeded invalid input should fail")?;
            assert!(!format!("{error:?}").contains("seeded-secret-canary"));
        }
    }
    Ok(())
}

#[test]
fn imports_ordered_dns_keys_and_treats_empty_assignments_as_resets() -> Result<(), String> {
    let source = parse_source(
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
    let source = parse_source(
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
    let source = parse_source(
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

#[test]
fn imports_image_and_build_resources_before_their_container_references() -> Result<(), String> {
    let source = parse_source(
        identifier("artifacts")?,
        [
            QuadletDocumentInput::new(
                "base.image",
                QuadletSourceId::new(1),
                concat!(
                    "[Image]\nImage=example.invalid/base:1\nImageTag=example.invalid/base:stable\n",
                    "ContainersConfModule=one.conf\nContainersConfModule=\nCreds=operator:secret\nDecryptionKey=private-key\n"
                ),
            ),
            QuadletDocumentInput::new(
                "web.build",
                QuadletSourceId::new(2),
                concat!(
                    "[Build]\nImageTag=example.invalid/web:1\nSetWorkingDirectory=.\nFile=Containerfile\n",
                    "BuildArg=TOKEN=private\nSecret=id=source,src=/run/secret\nEnvironment=BUILD_TOKEN=private\n",
                    "DNSOption=\nDNSOption=ndots:1\n"
                ),
            ),
            QuadletDocumentInput::new("api.container", QuadletSourceId::new(3), "[Container]\nImage=base.image\n"),
            QuadletDocumentInput::new("worker.container", QuadletSourceId::new(4), "[Container]\nImage=web.build\n"),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let app = result.application().ok_or("application expected")?;
    assert_eq!(app.image_acquisitions().len(), 1);
    assert_eq!(app.image_builds().len(), 1);
    let acquisition_settings = app.image_acquisitions()[0]
        .value()
        .settings()
        .ok_or("acquisition settings expected")?;
    let build_settings = app.image_builds()[0]
        .value()
        .settings()
        .ok_or("build settings expected")?;
    let acquisition_reference = app.services()[0]
        .value()
        .image_acquisition()
        .ok_or("acquisition reference expected")?;
    let build_reference = app.services()[1]
        .value()
        .image_build()
        .ok_or("build reference expected")?;
    assert!(matches!(
        acquisition_settings[4].value(),
        ImageAcquisitionSetting::Credentials(_)
    ));
    assert!(matches!(
        build_settings[3].value(),
        ImageBuildSetting::BuildArguments(_)
    ));
    assert_eq!(acquisition_reference.value().as_str(), "base");
    assert_eq!(build_reference.value().as_str(), "web");
    assert!(!format!("{:?}", app.image_acquisitions()).contains("operator:secret"));
    assert!(!format!("{:?}", app.image_builds()).contains("BUILD_TOKEN=private"));
    Ok(())
}

#[test]
fn duplicate_artifact_singletons_produce_one_invalid_import_outcome_without_last_wins_mapping() -> Result<(), String> {
    let parsed = QuadletDocument::parse(
        QuadletUnitType::Image,
        QuadletSourceId::new(77),
        "[Image]\nImage=example.invalid/first:1\nImage=example.invalid/second:1\n",
    )
    .map_err(|error| error.to_string())?;
    assert!(parsed.is_valid());
    let documents = QuadletDocumentSet::new(vec![
        NamedQuadletDocument::new("duplicate.image", parsed.document().clone()).map_err(|error| error.to_string())?,
    ])
    .map_err(|error| error.to_string())?;
    let source = QuadletSource::from_validated_documents(identifier("duplicate")?, documents)
        .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.application().is_some());
    let invalid = result
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
        .collect::<Vec<_>>();
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].subject(), "image_acquisitions.duplicate.Image");
    Ok(())
}

#[test]
fn imports_safe_network_settings_without_conflating_the_unit_stem_and_runtime_name() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "logical.network",
            QuadletSourceId::new(91),
            concat!(
                "[Network]\n",
                "NetworkName=runtime-network\nDriver=bridge\nOptions=mtu=1500\n",
                "Label=org.example.owner=private\nInternal=true\nIPv6=false\n",
                "IPAMDriver=host-local\nSubnet=10.88.0.0/16\nGateway=10.88.0.1\nIPRange=10.88.1.0/24\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let network = &result.application().ok_or("application expected")?.networks()[0].value();
    assert_eq!(network.name().as_str(), "logical");
    assert_eq!(
        network.runtime_name().map(|name| name.value().expose()),
        Some("runtime-network")
    );
    assert_eq!(network.driver().map(|driver| driver.value().expose()), Some("bridge"));
    assert_eq!(network.driver_options().map(<[_]>::len), Some(1));
    assert_eq!(network.labels().map(<[_]>::len), Some(1));
    assert_eq!(network.internal().map(|value| *value.value()), Some(true));
    assert_eq!(network.ipv6().map(|value| *value.value()), Some(false));
    let row = &network.ipam_configs().ok_or("IPAM row expected")?[0];
    assert_eq!(row.value().subnet().value().expose(), "10.88.0.0/16");
    assert_eq!(
        row.value().gateway().map(|value| value.value().expose()),
        Some("10.88.0.1")
    );
    assert_eq!(
        row.value().ip_range().map(|value| value.value().expose()),
        Some("10.88.1.0/24")
    );
    assert!(!format!("{network:?}").contains("private"));
    assert!(format!("{network:?}").contains("runtime-network"));
    Ok(())
}

#[test]
fn retains_network_resets_and_rejects_duplicate_option_and_label_names() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "logical.network",
            QuadletSourceId::new(93),
            concat!(
                "[Network]\n",
                "Options=mtu=1500\nOptions=mtu=9000\nOptions=\n",
                "Label=org.example.owner=one\nLabel=org.example.owner=two\nLabel=\n",
                "Subnet=\n",
            ),
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let network = &result.application().ok_or("application expected")?.networks()[0].value();
    assert!(network.driver_options().is_some_and(<[_]>::is_empty));
    assert!(network.labels().is_some_and(<[_]>::is_empty));
    assert!(network.ipam_configs().is_some_and(<[_]>::is_empty));
    assert_eq!(network.driver_options_origins().len(), 1);
    assert_eq!(network.labels_origins().len(), 1);
    assert_eq!(network.ipam_configs_origins().len(), 1);
    let unsupported = result
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .map(boxferry_engine::ConversionOutcome::subject)
        .collect::<Vec<_>>();
    assert_eq!(
        unsupported
            .iter()
            .filter(|subject| **subject == "networks.logical.driver_options")
            .count(),
        2
    );
    assert_eq!(
        unsupported
            .iter()
            .filter(|subject| **subject == "networks.logical.labels")
            .count(),
        2
    );
    Ok(())
}

#[test]
fn rejects_ambiguous_network_ipam_and_noncanonical_booleans_without_positional_zipping() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [QuadletDocumentInput::new(
            "logical.network",
            QuadletSourceId::new(92),
            "[Network]\nInternal=yes\nSubnet=10.88.0.0/16\nGateway=10.88.0.1\nSubnet=fd00::/64\nGateway=fd00::1\n",
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let network = &result.application().ok_or("application expected")?.networks()[0].value();
    assert!(network.ipam_configs().is_none());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "networks.logical.internal" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "networks.logical.ipam_configs" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn imports_typed_volume_settings_and_retains_resets_and_raw_evidence() -> Result<(), String> {
    let source = parse_source(
        identifier("example")?,
        [
            QuadletDocumentInput::new("base.image", QuadletSourceId::new(101), "[Image]\nImage=example.invalid/base:1\n"),
            QuadletDocumentInput::new("build.build", QuadletSourceId::new(102), "[Build]\nImageTag=example.invalid/build:1\n"),
            QuadletDocumentInput::new("data.volume", QuadletSourceId::new(103), "[Volume]\nVolumeName=runtime\nDriver=local\nDevice=/srv/data\nType=none\nOptions=bind\nLabel=org.example.owner=one\nLabel=\nCopy=true\nContainersConfModule=one module\nContainersConfModule=\nGlobalArgs=--log-level=debug\nPodmanArgs=--secret=private value\nUser=alice\nGroup=staff\nUID=1000\nGID=1001\nServiceName=data-custom\nImage=base.image\n"),
        ],
    ).map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let volume = result.application().ok_or("application expected")?.volumes()[0].value();
    assert_eq!(
        volume.runtime_name().map(|value| value.value().expose()),
        Some("runtime")
    );
    assert_eq!(
        volume.service_name().map(|value| value.value().expose()),
        Some("data-custom")
    );
    assert!(volume.labels().is_some_and(<[_]>::is_empty));
    assert_eq!(volume.labels_origins().len(), 1);
    assert!(volume.containers_conf_modules().is_some_and(<[_]>::is_empty));
    assert!(volume.global_args().is_some_and(|values| values.len() == 1));
    assert!(
        volume
            .podman_args()
            .is_some_and(|values| values[0].value().expose().contains("private value"))
    );
    assert!(
        matches!(volume.image_source().map(boxferry_model::Sourced::value), Some(VolumeImageSource::ImageAcquisition(name)) if name.as_str() == "base")
    );
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "volumes.data.containers_conf_modules"
                && outcome.kind() == ConversionKind::Unsupported)
    );
    Ok(())
}

fn identifier(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}
