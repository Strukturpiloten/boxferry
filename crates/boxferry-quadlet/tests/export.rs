//! Public Quadlet exporter behavior and target boundaries.

use std::error::Error;

use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile};
use boxferry_model::{
    Application, Command, EnvironmentValue, EnvironmentVariable, Identifier, ImageReference, Mount, MountSource,
    Network, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, ResourceOwnership, SelinuxRelabel,
    Service, SourceId, Sourced, Volume,
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
    service.add_environment(sourced(EnvironmentVariable::new(
        id("APP_ENV")?,
        EnvironmentValue::Literal(ProtectedString::sensitive("production")),
    ))?);
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
        Some(concat!(
            "[Container]\n",
            "Image=registry.example:5000/team/web:1.3@sha256:fedcba\n",
            "Exec=php -v\n",
            "Environment=APP_ENV=production\n",
            "PublishPort=127.0.0.1:8080:80/tcp\n",
            "Volume=data.volume:/var/lib/data:ro,Z\n",
            "Volume=/srv/config:/etc/config:z\n",
            "Volume=%h/.config/example:/home/config:ro\n",
            "Network=frontend.network\n",
        ))
    );
    assert!(output.document_set().is_valid());
    assert!(output.document_set().graph().is_complete());
    assert_eq!(output.document_set().graph().edges().len(), 2);
    assert!(!format!("{output:?}").contains("production"));
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
fn groups_compatible_services_only_after_explicit_approximation_authorization() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("grouped")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;

    let mut web = Service::new(id("web")?);
    web.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    web.add_port(sourced(Port::new(80, Some(8080), None, Protocol::Tcp)?)?);
    web.add_network(sourced(NetworkAttachment::new(id("frontend")?, Vec::new()))?);
    application.add_service(sourced(web)?)?;

    let mut worker = Service::new(id("worker")?);
    worker.set_image(sourced(ImageReference::parse("example.invalid/worker:1")?)?);
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
fn retains_a_partial_candidate_but_requires_authorization_for_unresolved_intent() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(id("partial")?);
    application.add_network(sourced(Network::new(id("frontend")?, ResourceOwnership::Application))?)?;
    let mut service = Service::new(id("web")?);
    service.set_image(sourced(ImageReference::parse("example.invalid/web:1")?)?);
    service.add_environment(sourced(EnvironmentVariable::new(
        id("FROM_HOST")?,
        EnvironmentValue::Host,
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
        3
    );
    assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
    let partial = plan.authorize(LossPolicy::AllowPartial);
    let output = partial.output().ok_or("partial output expected")?;
    let container = output.file("web.container").ok_or("container expected")?.text();
    assert!(!container.contains("FROM_HOST"));
    assert!(!container.contains("./config"));
    assert!(container.contains("Network=frontend.network"));
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
