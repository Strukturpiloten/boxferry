//! Offline facade acquisition contracts; the only peer is a bounded authored Unix socket.

#![cfg(all(feature = "docker", unix))]

use std::{
    error::Error,
    fs,
    io::{Read, Write},
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use boxferry::{
    DockerImporter, DockerPromotionPolicy, DockerResourceDecisions, DockerSelection, DockerSource, DockerSourceError,
    Identifier, ImportAdapter, acquire_docker_source,
    docker::docker_lens::{
        acquisition::{AcquisitionError, Endpoint, LimitError, Limits, NativeId, Selector},
        decoder::DecodeError,
        version::DaemonMode,
    },
};

const VERSION: &str = r#"{"Version":"28.1.1","ApiVersion":"1.49","MinAPIVersion":"1.41"}"#;
const INFO: &str = r#"{"SecurityOptions":["name=rootless"],"Labels":["private-daemon-canary"]}"#;
const CONTAINER_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

fn limits() -> Limits {
    Limits {
        max_requests: 4,
        max_selected_resources: 1,
        max_expansions: 1,
        max_response_bytes: 4096,
        max_total_bytes: 16384,
        max_elapsed: Duration::from_secs(3),
    }
}

struct SocketDirectory(PathBuf);

impl SocketDirectory {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = std::env::temp_dir().join(format!(
            "bf-docker-source-{}-{}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory)?;
        Ok(Self(directory))
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint::unix_socket(self.0.join("peer.sock"))
    }
}

impl Drop for SocketDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("peer.sock"));
        let _ = fs::remove_dir(&self.0);
    }
}

fn serve(listener: &UnixListener, script: Vec<(&'static str, String)>) -> Result<Vec<String>, String> {
    listener.set_nonblocking(true).map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut requests = Vec::new();
    for (expected_path, body) in script {
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= Duration::from_secs(4) {
                        return Err("authored socket accept deadline exceeded".into());
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            if request.len() >= 8192 {
                return Err("authored socket request budget exceeded".into());
            }
            let mut byte = [0_u8; 1];
            stream.read_exact(&mut byte).map_err(|error| error.to_string())?;
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).map_err(|error| error.to_string())?;
        let first_line = request.lines().next().ok_or("request line missing")?;
        if first_line != format!("GET {expected_path} HTTP/1.1") {
            return Err(format!("unexpected request: {first_line}"));
        }
        requests.push(first_line.to_owned());
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(requests)
}

type AcquisitionResult = Result<DockerSource, DockerSourceError>;

fn acquire_script(
    script: Vec<(&'static str, String)>,
    selector: Selector,
) -> Result<(AcquisitionResult, Vec<String>), Box<dyn Error>> {
    let directory = SocketDirectory::new()?;
    let endpoint = directory.endpoint();
    let listener = UnixListener::bind(endpoint.path())?;
    let server = thread::spawn(move || serve(&listener, script));
    let result = acquire_docker_source(
        Identifier::new("caller-selected")?,
        &endpoint,
        selector,
        limits(),
        &AtomicBool::new(false),
    );
    let requests = server.join().map_err(|_| "authored socket thread panicked")??;
    Ok((result, requests))
}

#[test]
fn discovery_retains_context_and_does_not_become_selected_intent() -> Result<(), Box<dyn Error>> {
    let (source, requests) = acquire_script(
        vec![
            ("/version", VERSION.into()),
            ("/v1.49/info", INFO.into()),
            ("/v1.49/containers/json?all=1", "[]".into()),
        ],
        Selector::Discovery,
    )?;
    let source = source?;
    assert_eq!(requests.len(), 3);
    assert_eq!(source.selection(), DockerSelection::Discovery);
    assert_eq!(source.application_name().as_str(), "caller-selected");
    assert_eq!(source.promotion_policy(), DockerPromotionPolicy::conservative());
    assert_eq!(source.resource_decisions(), &DockerResourceDecisions::new());
    assert!(source.public_label_decisions().entries().is_empty());
    assert_eq!(source.inventory().version.daemon.mode, DaemonMode::Rootless);
    assert!(source.inventory().version.daemon.release.is_some());
    assert_eq!(source.inventory().version.requested_api_versions.len(), 1);
    assert!(source.inventory().containers.is_empty());
    let imported = DockerImporter::new()?.import(&source);
    assert!(
        imported
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFD0002")
    );
    assert!(!format!("{:?}", imported.diagnostics()).contains("private-daemon-canary"));
    Ok(())
}

#[test]
fn exact_container_selector_is_forwarded_without_listing_or_implicit_promotion() -> Result<(), Box<dyn Error>> {
    let inspect = format!(
        r#"{{"Id":"{CONTAINER_ID}","Name":"/private-container-canary","Config":{{"Image":"private-image-canary"}},"Mounts":[]}}"#,
    );
    let (source, requests) = acquire_script(
        vec![
            ("/version", VERSION.into()),
            ("/v1.49/info", INFO.into()),
            (
                "/v1.49/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/json",
                inspect,
            ),
        ],
        Selector::ContainerIds(vec![NativeId::new(CONTAINER_ID.into()).ok_or("authored ID invalid")?]),
    )?;
    let source = source?;
    assert_eq!(requests.len(), 3);
    assert!(!requests.iter().any(|request| request.contains("?all=")));
    assert_eq!(source.selection(), DockerSelection::Explicit);
    assert_eq!(source.promotion_policy(), DockerPromotionPolicy::conservative());
    assert_eq!(source.inventory().containers.len(), 1);
    assert_eq!(source.inventory().selected_roots.len(), 1);
    let imported = DockerImporter::new()?.import(&source);
    let diagnostics = format!("{:?}", imported.diagnostics());
    for canary in [
        "private-container-canary",
        "private-image-canary",
        "private-daemon-canary",
    ] {
        assert!(!diagnostics.contains(canary));
    }
    Ok(())
}

#[test]
fn decode_failure_is_distinct_and_value_free() -> Result<(), Box<dyn Error>> {
    let (result, requests) = acquire_script(
        vec![
            ("/version", VERSION.into()),
            ("/v1.49/info", r#"["private-decode-canary"]"#.into()),
            ("/v1.49/containers/json?all=1", "[]".into()),
        ],
        Selector::Discovery,
    )?;
    assert_eq!(requests.len(), 3);
    let error = result.err().ok_or("malformed native info must fail decoding")?;
    assert!(matches!(error, DockerSourceError::Decode(DecodeError::InvalidShape(_))));
    assert!(!error.to_string().contains("private-decode-canary"));
    assert!(!format!("{error:?}").contains("peer.sock"));
    Ok(())
}

#[test]
fn caller_cancellation_limits_and_endpoint_errors_are_not_hidden() -> Result<(), Box<dyn Error>> {
    let directory = SocketDirectory::new()?;
    let endpoint = directory.endpoint();
    let cancelled = AtomicBool::new(true);
    let application = Identifier::new("caller-selected")?;
    let result = acquire_docker_source(
        application.clone(),
        &endpoint,
        Selector::Discovery,
        limits(),
        &cancelled,
    );
    assert!(matches!(
        result,
        Err(DockerSourceError::Acquisition(AcquisitionError::Cancelled))
    ));
    let mut invalid_limits = limits();
    invalid_limits.max_requests = 0;
    let result = acquire_docker_source(
        application.clone(),
        &endpoint,
        Selector::Discovery,
        invalid_limits,
        &cancelled,
    );
    assert!(matches!(
        result,
        Err(DockerSourceError::Acquisition(AcquisitionError::Budget(
            LimitError::Invalid
        )))
    ));
    let relative = Endpoint::unix_socket("private-relative-endpoint-canary".into());
    let result = acquire_docker_source(
        application,
        &relative,
        Selector::Discovery,
        limits(),
        &AtomicBool::new(false),
    );
    let error = result.err().ok_or("relative endpoint must be refused")?;
    assert_eq!(error, DockerSourceError::Acquisition(AcquisitionError::Endpoint));
    assert!(!error.to_string().contains("private-relative-endpoint-canary"));
    assert!(!format!("{error:?}").contains("private-relative-endpoint-canary"));
    Ok(())
}

#[cfg(all(feature = "compose", feature = "quadlet"))]
#[test]
fn promoted_empty_exposed_ports_keep_field_origins_through_compose_and_quadlet() -> Result<(), Box<dyn Error>> {
    use boxferry::docker::docker_lens::observation::{Availability, Origin};
    use boxferry::{
        COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ConversionKind,
        ExportAdapter, PlatformVersion, ProvenanceKind, QuadletExporter, TargetProfile,
    };

    let inspect =
        format!(r#"{{"Id":"{CONTAINER_ID}","Config":{{"Image":"registry.invalid/app:1","ExposedPorts":{{}}}}}}"#);
    let (source, _) = acquire_script(
        vec![
            ("/version", VERSION.into()),
            ("/v1.49/info", INFO.into()),
            (
                "/v1.49/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/json",
                inspect,
            ),
        ],
        Selector::ContainerIds(vec![NativeId::new(CONTAINER_ID.into()).ok_or("authored ID invalid")?]),
    )?;
    let source = source?.with_promotion_policy(DockerPromotionPolicy::portable_application());
    let native = &source.inventory().containers[0].exposed_ports;
    assert_eq!(native.availability, Availability::Empty);
    assert_eq!(native.origin, Origin::Effective);
    let imported = DockerImporter::new()?.import(&source);
    let application = imported.application().ok_or("missing imported application")?;
    let service = application.services()[0].value();
    assert_eq!(service.exposed_ports().map(<[_]>::len), Some(0));
    let expected = [ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision];
    assert_eq!(
        service
            .exposed_ports_origins()
            .iter()
            .map(boxferry::Provenance::kind)
            .collect::<Vec<_>>(),
        expected,
    );

    let compose_target = TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    let compose = ComposeExporter::new()?.plan(application, &compose_target)?;
    let compose_subject = format!("services.{}.expose", service.name().as_str());
    let exposed = compose
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == compose_subject)
        .ok_or("missing Compose empty-expose outcome")?;
    assert_eq!(exposed.kind(), ConversionKind::Exact);
    assert_eq!(exposed.origins(), service.exposed_ports_origins());
    assert!(
        compose
            .candidate()
            .ok_or("missing Compose candidate")?
            .text()
            .contains("expose: []")
    );

    let quadlet_target = TargetProfile::new(
        "podman",
        PlatformVersion::new(6, 1, 2),
        Some(PlatformVersion::new(6, 1, 2)),
    )?;
    let quadlet = QuadletExporter::new()?.plan(application, &quadlet_target)?;
    let quadlet_subject = format!("services.{}.exposed_ports", service.name().as_str());
    let exposed = quadlet
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == quadlet_subject)
        .ok_or("missing Quadlet empty-exposed-ports refusal")?;
    assert_eq!(exposed.kind(), ConversionKind::Unsupported);
    assert_eq!(exposed.origins(), service.exposed_ports_origins());
    Ok(())
}

#[cfg(feature = "quadlet")]
fn acquire_literal_bind_source(source_path: &str) -> Result<DockerSource, Box<dyn Error>> {
    let inspect = serde_json::json!({
        "Id": CONTAINER_ID,
        "Config": {"Image": "registry.invalid/app:1"},
        "Mounts": [{"Type": "bind", "Source": source_path, "Destination": "/data/$TARGET%i",
            "RW": true, "Mode": "rw", "Propagation": "rprivate"}],
    })
    .to_string();
    let (source, requests) = acquire_script(
        vec![
            ("/version", VERSION.into()),
            ("/v1.49/info", INFO.into()),
            (
                "/v1.49/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/json",
                inspect,
            ),
        ],
        Selector::ContainerIds(vec![NativeId::new(CONTAINER_ID.into()).ok_or("authored ID invalid")?]),
    )?;
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| request.starts_with("GET ")));
    Ok(source?)
}

#[cfg(feature = "quadlet")]
#[test]
fn acquired_docker_bind_paths_stay_literal_through_quadlet_reimport() -> Result<(), Box<dyn Error>> {
    use boxferry::{
        ExportAdapter, MountSource, PlatformVersion, QuadletDocumentInput, QuadletExporter, QuadletImporter,
        QuadletSource, TargetProfile,
    };
    let source = acquire_literal_bind_source("/srv/%h")?
        .with_promotion_policy(DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true));
    let imported = DockerImporter::new()?.import(&source);
    let application = imported.application().ok_or("missing literal bind application")?;
    let service = application.services()[0].value();
    assert_eq!(service.mounts().len(), 1);
    assert!(service.mounts()[0].value().paths_are_literal());
    assert!(matches!(service.mounts()[0].value().source(), MountSource::HostPath(source) if source == "/srv/%h"));
    assert_eq!(service.mounts()[0].value().target(), "/data/$TARGET%i");
    assert!(service.execution_environment().is_none());
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 1, 2)),
    )?;
    let planned = QuadletExporter::new()?.plan(application, &target)?;
    assert!(
        !planned.outcomes().iter().any(|outcome| matches!(
            outcome.kind(),
            boxferry::ConversionKind::Unsupported | boxferry::ConversionKind::Invalid,
        )),
        "unexpected Quadlet literal-path outcomes: {:?}",
        planned.outcomes(),
    );
    let output = planned.candidate().ok_or("missing literal bind Quadlet candidate")?;
    assert_eq!(output.files().len(), 1);
    let text = output.files()[0].text();
    assert!(
        text.lines().any(|line| line == "Volume=/srv/%%h:/data/$$TARGET%%i"),
        "{text}"
    );
    let reimported = QuadletImporter::new()?.import(
        &QuadletSource::parse(
            Identifier::new("caller-selected")?,
            [QuadletDocumentInput::new(
                "docker-service-1.container",
                boxferry::quadlet::quadlet_lens::source::SourceId::new(1),
                text,
            )],
        )?
        .into_source(),
    );
    let service = reimported
        .application()
        .ok_or("missing reimported bind application")?
        .services()[0]
        .value();
    assert_eq!(service.mounts().len(), 1);
    assert!(service.execution_environment().is_none());
    let mount = service.mounts()[0].value();
    assert!(matches!(mount.source(), MountSource::HostPath(source) if source == "/srv/%h"));
    assert_eq!(mount.target(), "/data/$TARGET%i");
    assert!(mount.paths_are_literal());
    assert!(!mount.read_only());
    Ok(())
}

#[cfg(feature = "quadlet")]
#[test]
fn acquired_docker_dollar_bind_source_reports_native_dependency_limit_even_under_partial_policy()
-> Result<(), Box<dyn Error>> {
    use boxferry::{
        ConversionKind, ExportAdapter, LossPolicy, MountSource, PlatformVersion, QuadletExporter, TargetProfile,
    };
    // QuadletLens ADR 0022 explicitly rejects literal dollar-bearing bind sources:
    // older generators cannot preserve their inferred mount dependencies. This is
    // separate from the supported literal-dollar container target above.
    let source = acquire_literal_bind_source("/srv/$ROOT%h")?
        .with_promotion_policy(DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true));
    let imported = DockerImporter::new()?.import(&source);
    let application = imported.application().ok_or("missing dollar-source application")?;
    let service = application.services()[0].value();
    assert_eq!(service.mounts().len(), 1);
    assert!(service.execution_environment().is_none());
    let mount = service.mounts()[0].value();
    assert!(mount.paths_are_literal());
    assert!(matches!(mount.source(), MountSource::HostPath(source) if source == "/srv/$ROOT%h"));
    assert_eq!(mount.target(), "/data/$TARGET%i");
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 1, 2)),
    )?;
    let planned = QuadletExporter::new()?.plan(application, &target)?;
    let losses = planned
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() != ConversionKind::Exact)
        .collect::<Vec<_>>();
    assert_eq!(
        losses.len(),
        1,
        "unexpected dollar-source outcomes: {:?}",
        planned.outcomes()
    );
    assert_eq!(losses[0].subject(), "services.docker-service-1.mounts[0]");
    assert_eq!(losses[0].kind(), ConversionKind::Unsupported);
    assert_eq!(
        losses[0].diagnostic().map(boxferry::DiagnosticCode::as_str),
        Some("BFQ0003")
    );
    let diagnostic = planned
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFQ0003")
        .ok_or("missing native dollar-source limitation diagnostic")?;
    assert!(
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "subject" && field.value().expose() == "services.docker-service-1.mounts[0]")
    );
    let reason = diagnostic
        .fields()
        .iter()
        .find(|field| field.name() == "reason")
        .ok_or("missing native dollar-source refusal reason")?
        .value()
        .expose();
    assert!(
        reason.contains("absolute literal bind sources with dollars"),
        "{reason}"
    );
    assert!(reason.contains("inferred mount dependencies"), "{reason}");
    let authorized = planned.authorize(LossPolicy::AllowPartial);
    let output = authorized
        .output()
        .ok_or("partial policy should retain the unrelated image/service candidate")?;
    assert_eq!(output.files().len(), 1);
    let text = output.files()[0].text();
    assert!(
        !text.lines().any(|line| line.starts_with("Volume=")),
        "unsafe bind emitted under partial policy: {text}"
    );
    assert!(!text.contains("$ROOT"));
    assert!(!text.contains("${ROOT}"));
    assert!(service.execution_environment().is_none());
    Ok(())
}
