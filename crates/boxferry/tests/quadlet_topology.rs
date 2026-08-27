//! Public facade coverage for Quadlet container and pod topology settings.

#![cfg(feature = "quadlet")]

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    ConversionKind, GroupExitPolicy, Identifier, ImageReference, ImportAdapter, LossPolicy, ModelError,
    PlatformVersion, ProtectedString, QuadletDocumentInput, QuadletExporter, QuadletGroupingPolicy, QuadletImporter,
    QuadletSource, Service, ServiceGroupRuntime, Sourced, StartupNotification, TargetProfile, convert,
};

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(application_name, inputs)?.into_source())
}

#[test]
fn facade_reexports_topology_model_types_and_rejects_rootfs_image_conflicts() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    assert!(matches!(
        service.set_rootfs(Sourced::generated(ProtectedString::sensitive("/private/rootfs"))),
        Err(ModelError::RootfsImageSourceConflict { .. })
    ));

    let mut runtime = ServiceGroupRuntime::new();
    runtime.set_exit_policy(Sourced::generated(GroupExitPolicy::Continue));
    service.set_startup_notification(Sourced::generated(StartupNotification::Healthy));
    assert!(matches!(
        runtime.exit_policy().map(Sourced::value),
        Some(GroupExitPolicy::Continue)
    ));
    assert!(matches!(
        service.startup_notification().map(Sourced::value),
        Some(StartupNotification::Healthy)
    ));
    assert!(!format!("{service:?}").contains("/private/rootfs"));
    Ok(())
}

#[test]
fn facade_preserves_topology_keys_with_their_documented_podman_floors() -> Result<(), Box<dyn Error>> {
    let source = topology_source()?;
    let importer = QuadletImporter::new()?;
    let exporter = QuadletExporter::new()?.with_grouping_policy(QuadletGroupingPolicy::PreserveSingleGroup);

    let floor_54 = convert(
        &importer,
        &source,
        &exporter,
        &podman_target(5, 4, 0, 5, 4, 2)?,
        LossPolicy::AllowPartial,
    )?;
    let pod_54 = pod_text(&floor_54)?;
    assert_topology_output(pod_54, false, false);
    for subject in [
        "service_groups.topology.runtime.exit_policy",
        "service_groups.topology.runtime.stop_timeout",
    ] {
        assert!(
            floor_54
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }

    let floor_56 = convert(
        &importer,
        &source,
        &exporter,
        &podman_target(5, 6, 0, 5, 6, 2)?,
        LossPolicy::AllowPartial,
    )?;
    assert_topology_output(pod_text(&floor_56)?, true, false);

    let floor_57 = convert(
        &importer,
        &source,
        &exporter,
        &podman_target(5, 7, 0, 6, 0, 2)?,
        LossPolicy::AllowApproximate,
    )?;
    assert_topology_output(pod_text(&floor_57)?, true, true);
    let rootfs = convert(
        &QuadletImporter::new()?,
        &rootfs_source()?,
        &QuadletExporter::new()?,
        &podman_target(5, 7, 0, 6, 0, 2)?,
        LossPolicy::AllowPartial,
    )?;
    let container = rootfs
        .output()
        .and_then(|output| output.file("rootfs.container"))
        .map(boxferry::QuadletFile::text)
        .ok_or("rootfs container output expected")?;
    assert!(container.contains("Rootfs=/srv/rootfs"));
    assert!(container.contains("Notify=healthy"));
    assert_eq!(container.matches("PodmanArgs=").count(), 2);
    assert!(container.contains("PodmanArgs=--replace"));
    assert!(container.contains("PodmanArgs=--secret=private-value"));
    assert!(!format!("{rootfs:?}").contains("private-value"));

    let ceiling = convert(
        &importer,
        &source,
        &exporter,
        &podman_target(6, 1, 0, 6, 1, 0)?,
        LossPolicy::AllowApproximate,
    )?;
    assert_topology_output(pod_text(&ceiling)?, true, true);

    let beyond_ceiling = convert(
        &importer,
        &source,
        &exporter,
        &podman_target(6, 1, 1, 6, 1, 1)?,
        LossPolicy::AllowPartial,
    )?;
    assert!(beyond_ceiling.is_blocked());
    assert!(beyond_ceiling.output().is_none());
    assert!(
        beyond_ceiling
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "target.versions" && outcome.kind() == ConversionKind::Invalid })
    );
    Ok(())
}

#[test]
fn facade_retains_pod_resets_and_omitted_runtime_name_without_synthesis() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("resets")?,
        [
            QuadletDocumentInput::new(
                "resets.pod",
                QuadletSourceId::new(1),
                concat!(
                    "[Pod]\nServiceName=chosen\nAddHost=host.docker.internal:host-gateway\nAddHost=\n",
                    "PublishPort=8080:80\nPublishPort=\nVolume=cache.volume:/cache\nVolume=\n",
                ),
            ),
            QuadletDocumentInput::new("cache.volume", QuadletSourceId::new(2), "[Volume]\n"),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(3),
                "[Container]\nImage=example.invalid/web:1\nPod=resets.pod\n",
            ),
        ],
    )?;
    let quadlet_importer = QuadletImporter::new()?;
    let imported = quadlet_importer.import(&source);
    let runtime = imported
        .application()
        .and_then(|application| application.service_groups().first())
        .and_then(|group| group.value().runtime())
        .map(Sourced::value)
        .ok_or("pod runtime expected")?;
    assert!(runtime.runtime_name().is_none());
    assert!(runtime.host_mappings().is_some_and(<[_]>::is_empty));
    assert!(runtime.ports().is_some_and(<[_]>::is_empty));
    assert!(runtime.mounts().is_some_and(<[_]>::is_empty));

    let result = convert(
        &quadlet_importer,
        &source,
        &QuadletExporter::new()?.with_grouping_policy(QuadletGroupingPolicy::PreserveSingleGroup),
        &podman_target(5, 7, 0, 6, 0, 2)?,
        LossPolicy::AllowPartial,
    )?;
    let pod = pod_text(&result)?;
    assert!(pod.contains("ServiceName=chosen"));
    for omitted in ["PodName=", "AddHost=", "PublishPort=", "Volume="] {
        assert!(!pod.contains(omitted), "unexpected reset synthesis: {pod}");
    }
    Ok(())
}

#[test]
fn facade_rejects_host_network_with_published_ports() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("conflict")?,
        [
            QuadletDocumentInput::new(
                "conflict.pod",
                QuadletSourceId::new(1),
                "[Pod]\nPodName=conflict\nPublishPort=8080:80\nNetwork=host\n",
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/web:1\nPod=conflict.pod\n",
            ),
        ],
    )?;
    let result = QuadletImporter::new()?.import(&source);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.conflict.runtime.ports" && outcome.kind() == ConversionKind::Invalid
    }));
    Ok(())
}

fn topology_source() -> Result<QuadletSource, Box<dyn Error>> {
    parse_source(
        Identifier::new("topology")?,
        [
            QuadletDocumentInput::new("frontend.network", QuadletSourceId::new(1), "[Network]\n"),
            QuadletDocumentInput::new("cache.volume", QuadletSourceId::new(2), "[Volume]\n"),
            QuadletDocumentInput::new(
                "topology.pod",
                QuadletSourceId::new(3),
                concat!(
                    "[Pod]\nPodName=topology\nServiceName=topology-service\n",
                    "AddHost=host.docker.internal:host-gateway\nPublishPort=8080:80/tcp\n",
                    "Network=frontend.network\nUserNS=keep-id\nVolume=cache.volume:/cache\n",
                    "ShmSize=64m\nExitPolicy=continue\nStopTimeout=30\n",
                ),
            ),
            QuadletDocumentInput::new(
                "member.container",
                QuadletSourceId::new(4),
                "[Container]\nImage=example.invalid/member:1\nPod=topology.pod\n",
            ),
        ],
    )
}

fn rootfs_source() -> Result<QuadletSource, Box<dyn Error>> {
    parse_source(
        Identifier::new("rootfs")?,
        [QuadletDocumentInput::new(
            "rootfs.container",
            QuadletSourceId::new(5),
            concat!(
                "[Container]\nRootfs=/srv/rootfs\nNotify=healthy\n",
                "PodmanArgs=--replace\nPodmanArgs=--secret=private-value\n",
            ),
        )],
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

fn pod_text(result: &boxferry::ConversionResult<boxferry::QuadletOutput>) -> Result<&str, Box<dyn Error>> {
    result
        .output()
        .and_then(|output| output.file("topology.pod").or_else(|| output.file("resets.pod")))
        .map(boxferry::QuadletFile::text)
        .ok_or_else(|| format!("pod output expected: {:#?}", result.diagnostics()).into())
}

fn assert_topology_output(text: &str, exit_policy: bool, stop_timeout: bool) {
    for line in [
        "PodName=topology",
        "ServiceName=topology-service",
        "AddHost=host.docker.internal:host-gateway",
        "PublishPort=8080:80/tcp",
        "Network=frontend.network",
        "UserNS=keep-id",
        "Volume=cache.volume:/cache",
        "ShmSize=64m",
    ] {
        assert!(text.contains(line), "missing {line} in {text}");
    }
    assert_eq!(text.contains("ExitPolicy=continue"), exit_policy, "{text}");
    assert_eq!(text.contains("StopTimeout=30"), stop_timeout, "{text}");
}
