//! Independent coverage for QuadletLens-owned native value decoding.

use boxferry_engine::{ConversionKind, ImportAdapter};
use boxferry_model::{Command, Entrypoint, Identifier, ImageBuildSetting, MountSource};
use boxferry_quadlet::{QuadletDocumentInput, QuadletImporter, QuadletParseError, QuadletParseResult, QuadletSource};
use quadlet_lens::source::SourceId as QuadletSourceId;

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, QuadletParseError> {
    QuadletSource::parse(application_name, inputs).map(QuadletParseResult::into_source)
}

fn identifier(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}

#[test]
fn native_command_resets_stay_ordered_private_and_free_of_singleton_errors() -> Result<(), String> {
    let input = concat!(
        "[Container]\n",
        "Image=example.invalid/private:1\n",
        "Exec=/bin/echo first\n",
        "Exec=\n",
        "Exec=/bin/echo \"quoted-secret-130\"\n",
        "Entrypoint=[\"/old\"]\n",
        "Entrypoint=\n",
        "Entrypoint=[\"/bin/sh\",\"-eu\"]\n",
        "Volume=/srv/cache.image:/cache:ro\n",
        "Mount=type=bind,source=/srv/long.image,destination=/long,readonly\n",
        "Volume=/srv/conflict:/conflict:ro,rw\n",
    );
    let source = parse_source(
        identifier("native-command-order")?,
        [QuadletDocumentInput::new(
            "private.container",
            QuadletSourceId::new(300),
            input,
        )],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let service = result.application().ok_or("application")?.services()[0].value();

    let command = service.command().ok_or("command")?;
    let Command::Exec(arguments) = command.value() else {
        return Err("exec command expected".to_owned());
    };
    assert_eq!(
        arguments
            .iter()
            .map(boxferry_model::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["/bin/echo", "quoted-secret-130"]
    );
    assert!(arguments.iter().all(boxferry_model::ProtectedString::is_sensitive));
    let command_text = "/bin/echo \"quoted-secret-130\"";
    let command_start = input.find(command_text).ok_or("command span")?;
    let command_span = command.origins()[0].span().ok_or("spanned command origin")?;
    assert_eq!(
        (command_span.start(), command_span.end()),
        (command_start, command_start + command_text.len())
    );

    let Entrypoint::Exec(arguments) = service.entrypoint().ok_or("entrypoint")?.value() else {
        return Err("exec entrypoint expected".to_owned());
    };
    assert_eq!(
        arguments
            .iter()
            .map(boxferry_model::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["/bin/sh", "-eu"]
    );
    assert!(arguments.iter().all(boxferry_model::ProtectedString::is_sensitive));
    assert_eq!(service.mounts().len(), 2);
    assert!(
        service
            .mounts()
            .iter()
            .all(|mount| matches!(mount.value().source(), MountSource::HostPath(_)))
    );
    assert!(result.outcomes().iter().all(|outcome| {
        !matches!(
            outcome.subject(),
            "services.private.command" | "services.private.entrypoint"
        ) || outcome.kind() != ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.private.mounts[2]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(!format!("{result:?}").contains("quoted-secret-130"));
    Ok(())
}

#[test]
fn later_contextual_native_values_suppress_stale_effective_state() -> Result<(), String> {
    let source = parse_source(
        identifier("effective-native-state")?,
        [
            QuadletDocumentInput::new(
                "builder.build",
                QuadletSourceId::new(304),
                concat!(
                    "[Build]\n",
                    "ImageTag=example.invalid/build:1\n",
                    "Environment=KEEP=kept STALE=old-build\n",
                    "Environment=STALE=%h\n",
                ),
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(305),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "Exec=/bin/true\n",
                    "Exec=/bin/echo %h\n",
                    "Entrypoint=[\"/bin/true\"]\n",
                    "Entrypoint=/bin/echo %h\n",
                    "Environment=KEEP=kept STALE=old-container\n",
                    "Environment=STALE=%h\n",
                ),
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("application")?;
    let service = application.services()[0].value();
    assert!(service.command().is_none());
    assert!(service.entrypoint().is_none());
    assert_eq!(service.environment().len(), 1);
    assert_eq!(service.environment()[0].value().name().as_str(), "KEEP");

    let environment = application.image_builds()[0]
        .value()
        .settings()
        .ok_or("build settings")?
        .iter()
        .find_map(|setting| match setting.value() {
            ImageBuildSetting::Environment(values) => Some(values),
            _ => None,
        })
        .ok_or("build environment")?;
    assert_eq!(environment.values().len(), 1);
    assert_eq!(environment.values()[0].value().name().expose(), "KEEP");
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.command" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.entrypoint" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.environment.STALE" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "image_builds.builder.Environment.STALE" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(!format!("{result:?}").contains("old-container"));
    assert!(!format!("{result:?}").contains("old-build"));
    Ok(())
}

#[test]
fn malformed_native_environment_does_not_expose_stale_assignments() -> Result<(), String> {
    let source = parse_source(
        identifier("malformed-native-state")?,
        [
            QuadletDocumentInput::new(
                "malformed-build.build",
                QuadletSourceId::new(306),
                concat!(
                    "[Build]\n",
                    "ImageTag=example.invalid/build:1\n",
                    "Environment=STALE=old-build\n",
                    "Environment=\"BROKEN=value\n",
                ),
            ),
            QuadletDocumentInput::new(
                "malformed.container",
                QuadletSourceId::new(307),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "Environment=STALE=old-container\n",
                    "Environment=\"BROKEN=value\n",
                ),
            ),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("application")?;
    let service = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "malformed")
        .ok_or("malformed service")?;
    assert!(service.value().environment().is_empty());

    let build = application
        .image_builds()
        .iter()
        .find(|build| build.value().name().as_str() == "malformed-build")
        .ok_or("malformed build")?;
    assert!(
        build
            .value()
            .settings()
            .ok_or("build settings")?
            .iter()
            .all(|setting| !matches!(setting.value(), ImageBuildSetting::Environment(_)))
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.malformed.environment" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "image_builds.malformed-build.Environment" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(!format!("{result:?}").contains("old-container"));
    assert!(!format!("{result:?}").contains("old-build"));
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn document_native_views_preserve_pod_build_resets_and_exact_lens_spans() -> Result<(), String> {
    let build_input = concat!(
        "[Build]\n",
        "ImageTag=example.invalid/build:1\n",
        "Environment=\"BEFORE=hello build\" TWO=\n",
        "Environment=\n",
        "Environment=AFTER=\"quoted build\"\n",
        "Environment=DEFERRED=%h\n",
    );
    let pod_input = concat!(
        "[Pod]\n",
        "PublishPort=[::1]:18080:8080/tcp\n",
        "PublishPort=\n",
        "PublishPort=127.0.0.1:18081:8081/udp\n",
        "Volume=old:/old:ro\n",
        "Volume=\n",
        "Volume=/srv/pod.image:/pod:ro\n",
        "PublishPort=20000-20001:9000-9001/udp\n",
        "Volume=relative:relative\n",
        "PublishPort=broken\n",
        "PublishPort=%h:80\n",
        "Volume=a:b:c:d\n",
        "Volume=%h/data:/deferred\n",
    );
    let container_input = concat!(
        "[Container]\n",
        "Image=example.invalid/web:1\n",
        "Pod=application.pod\n",
        "Exec=/bin/true\n",
        "Exec=/usr/bin/app \"unterminated\n",
        "Exec=/bin/echo %h\n",
    );
    let source = parse_source(
        identifier("document-native-views")?,
        [
            QuadletDocumentInput::new("builder.build", QuadletSourceId::new(301), build_input),
            QuadletDocumentInput::new("application.pod", QuadletSourceId::new(302), pod_input),
            QuadletDocumentInput::new("web.container", QuadletSourceId::new(303), container_input),
        ],
    )
    .map_err(|error| error.to_string())?;
    let result = QuadletImporter::new()
        .map_err(|error| error.to_string())?
        .import(&source);
    let application = result.application().ok_or("application")?;

    let runtime = application.service_groups()[0]
        .value()
        .runtime()
        .ok_or("pod runtime")?
        .value();
    let ports = runtime.ports().ok_or("pod ports")?;
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].value().host_address(), Some("127.0.0.1"));
    assert_eq!(ports[0].value().published(), Some(18081));
    assert_eq!(runtime.ports_origins().len(), 2);
    let mounts = runtime.mounts().ok_or("pod mounts")?;
    assert_eq!(mounts.len(), 1);
    assert!(matches!(
        mounts[0].value().source(),
        MountSource::HostPath(path) if path == "/srv/pod.image"
    ));
    assert_eq!(runtime.mounts_origins().len(), 2);

    let environments = application.image_builds()[0]
        .value()
        .settings()
        .ok_or("build settings")?
        .iter()
        .filter_map(|setting| match setting.value() {
            ImageBuildSetting::Environment(values) => Some(values),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(environments.len(), 3);
    assert_eq!(environments[0].values().len(), 2);
    assert!(environments[1].values().is_empty());
    assert_eq!(environments[2].values()[0].value().name().expose(), "AFTER");
    assert_eq!(
        environments[2].values()[0]
            .value()
            .value()
            .ok_or("build environment value")?
            .expose(),
        "quoted build"
    );

    let expected_native_spans = [
        ("QLM0029", 302, pod_input, "PublishPort=broken", "broken"),
        ("QLM0030", 302, pod_input, "PublishPort=%h:80", "%h:80"),
        ("QLM0031", 302, pod_input, "Volume=a:b:c:d", "a:b:c:d"),
        (
            "QLM0032",
            302,
            pod_input,
            "Volume=%h/data:/deferred",
            "%h/data:/deferred",
        ),
        (
            "QLM0033",
            303,
            container_input,
            "Exec=/usr/bin/app \"unterminated",
            "/usr/bin/app \"unterminated",
        ),
        ("QLM0034", 303, container_input, "Exec=/bin/echo %h", "/bin/echo %h"),
    ];
    for (code, source_id, input, entry, value) in expected_native_spans {
        let finding = result
            .diagnostics()
            .iter()
            .filter_map(boxferry_engine::Diagnostic::native_finding)
            .find(|finding| finding.code() == code)
            .ok_or_else(|| format!("missing native finding {code}"))?;
        let label = finding.labels().first().ok_or("native finding label")?;
        let entry_start = input.find(entry).ok_or("native entry span")?;
        let value_start = entry_start + entry.find(value).ok_or("native value span")?;
        assert_eq!(label.source_id(), source_id);
        assert_eq!((label.start(), label.end()), (value_start, value_start + value.len()));
    }
    assert!(!format!("{result:?}").contains("quoted build"));
    Ok(())
}
