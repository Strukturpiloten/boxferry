//! Public Quadlet round-trip coverage for the iteration-one container keys.

#![cfg(feature = "quadlet")]

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId;
use boxferry::{
    ConversionKind, Identifier, LossPolicy, PlatformVersion, QuadletDocumentInput, QuadletExporter, QuadletImporter,
    QuadletSource, TargetProfile, convert,
};

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(application_name, inputs)?.into_source())
}

#[test]
fn public_quadlet_route_preserves_iteration_one_keys_at_the_5_5_floor() -> Result<(), Box<dyn Error>> {
    let source = iteration_one_source()?;
    let target = podman_target(5, 5, 0, 6, 0, 2)?;

    let strict = convert(
        &QuadletImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(strict.is_blocked(), "{:#?}", strict.diagnostics());
    assert!(strict.output().is_none());

    let partial = convert(
        &QuadletImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    assert!(!partial.is_blocked(), "{:#?}", partial.diagnostics());
    let output = partial.output().ok_or("partial Quadlet output expected")?;
    assert_container_lines_in_order(
        output.file("web.container").map(boxferry::QuadletFile::text),
        &[
            "Image=example.invalid/web:1",
            "HostName=web.example",
            "PidsLimit=42",
            "ShmSize=64m",
            "DropCapability=NET_RAW",
            "AddCapability=SYS_PTRACE",
            "Tmpfs=/run:mode=1777",
            "Sysctl=net.ipv4.ip_forward=1",
            "Ulimit=nofile=1024:4096",
            "AddDevice=/dev/fuse:/dev/fuse:rwm",
            "StopSignal=SIGTERM",
            "Entrypoint=[\"/bin/web\",\"serve\"]",
            "RunInit=true",
            "StopTimeout=30",
            "Pull=always",
            "Memory=64m",
            "ExposeHostPort=8080/udp",
            "Annotation=\"io.example.note=private-annotation\"",
            "LogDriver=journald",
            "LogOpt=\"tag=private-log-option\"",
            "ReloadSignal=SIGHUP",
            "IP=192.0.2.10",
            "IP6=2001:db8::10",
            "NetworkAlias=web",
        ],
    )?;
    assert_container_lines_in_order(
        output.file("worker.container").map(boxferry::QuadletFile::text),
        &["Image=example.invalid/worker:1", "ReloadCmd=/bin/worker reload"],
    )?;
    assert!(output.document_set().is_valid());
    assert!(!format!("{partial:?}").contains("private-annotation"));
    assert!(!format!("{partial:?}").contains("private-log-option"));
    assert!(partial.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.run_init" && outcome.kind() == ConversionKind::Approximate
    }));
    Ok(())
}

#[test]
fn public_quadlet_route_reports_5_4_floors_and_attachment_ambiguity_without_leaking_values()
-> Result<(), Box<dyn Error>> {
    let source = iteration_one_source()?;
    let target = podman_target(5, 4, 0, 5, 4, 2)?;
    let result = convert(
        &QuadletImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let output = result.output().ok_or("partial 5.4 output expected")?;
    let web = output
        .file("web.container")
        .map(boxferry::QuadletFile::text)
        .ok_or("web output")?;
    let worker = output
        .file("worker.container")
        .map(boxferry::QuadletFile::text)
        .ok_or("worker output")?;
    assert!(!web.contains("Memory="));
    assert!(!web.contains("ReloadSignal="));
    assert!(!worker.contains("ReloadCmd="));
    for subject in [
        "services.web.memory_limit",
        "services.web.reload_action",
        "services.worker.reload_action",
    ] {
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }

    let ambiguous = parse_source(
        Identifier::new("ambiguous")?,
        [
            QuadletDocumentInput::new("front.network", SourceId::new(11), "[Network]\n"),
            QuadletDocumentInput::new("back.network", SourceId::new(12), "[Network]\n"),
            QuadletDocumentInput::new(
                "web.container",
                SourceId::new(13),
                concat!(
                    "[Container]\nImage=example.invalid/web:1\nNetwork=front.network\nNetwork=back.network\n",
                    "IP=192.0.2.10\nNetworkAlias=private-alias\nAnnotation=io.example.note=private-annotation\n",
                    "LogDriver=journald\nLogOpt=tag=private-log-option\n",
                ),
            ),
        ],
    )?;
    let strict = convert(
        &QuadletImporter::new()?,
        &ambiguous,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(strict.is_blocked());
    let partial = convert(
        &QuadletImporter::new()?,
        &ambiguous,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let text = partial
        .output()
        .and_then(|output| output.file("web.container"))
        .map(boxferry::QuadletFile::text)
        .ok_or("ambiguous partial output expected")?;
    assert!(!text.contains("IP="));
    assert!(!text.contains("NetworkAlias="));
    assert!(partial.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.networks.address" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(partial.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.networks.aliases" && outcome.kind() == ConversionKind::Unsupported
    }));
    let debug = format!("{partial:?}");
    for secret in ["private-alias", "private-annotation", "private-log-option"] {
        assert!(!debug.contains(secret));
    }
    Ok(())
}

fn iteration_one_source() -> Result<QuadletSource, Box<dyn Error>> {
    parse_source(
        Identifier::new("iteration-one")?,
        [
            QuadletDocumentInput::new("frontend.network", SourceId::new(1), "[Network]\n"),
            QuadletDocumentInput::new(
                "web.container",
                SourceId::new(2),
                concat!(
                    "[Container]\nImage=example.invalid/web:1\nNetwork=frontend.network\n",
                    "Entrypoint=[\"/bin/web\",\"serve\"]\nRunInit=true\nStopTimeout=30\nPull=always\nMemory=64m\n",
                    "ExposeHostPort=8080/udp\nAnnotation=io.example.note=private-annotation\n",
                    "LogDriver=journald\nLogOpt=tag=private-log-option\nReloadSignal=SIGHUP\n",
                    "IP=192.0.2.10\nIP6=2001:db8::10\nNetworkAlias=web\n",
                    "HostName=web.example\nPidsLimit=42\nShmSize=64m\nDropCapability=NET_RAW\n",
                    "AddCapability=SYS_PTRACE\nTmpfs=/run:mode=1777\nSysctl=net.ipv4.ip_forward=1\n",
                    "Ulimit=nofile=1024:4096\nAddDevice=/dev/fuse:/dev/fuse:rwm\nStopSignal=SIGTERM\n",
                ),
            ),
            QuadletDocumentInput::new(
                "worker.container",
                SourceId::new(3),
                "[Container]\nImage=example.invalid/worker:1\nReloadCmd=/bin/worker reload\n",
            ),
        ],
    )
}

fn podman_target(
    minimum_major: u64,
    minimum_minor: u64,
    minimum_patch: u64,
    maximum_major: u64,
    maximum_minor: u64,
    maximum_patch: u64,
) -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        "podman",
        PlatformVersion::new(minimum_major, minimum_minor, minimum_patch),
        Some(PlatformVersion::new(maximum_major, maximum_minor, maximum_patch)),
    )?)
}

fn assert_container_lines_in_order(text: Option<&str>, expected: &[&str]) -> Result<(), Box<dyn Error>> {
    let text = text.ok_or("container output expected")?;
    let mut offset = 0;
    for line in expected {
        let Some(index) = text[offset..].find(line) else {
            return Err(format!("missing `{line}` in {text}").into());
        };
        offset += index + line.len();
    }
    Ok(())
}
