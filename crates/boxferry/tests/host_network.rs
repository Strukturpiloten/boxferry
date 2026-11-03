//! Host namespace mappings, conflicts, and explicit unsupported-mode boundaries.

#![cfg(all(feature = "compose", feature = "quadlet"))]

use std::error::Error;

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};
use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeImporter,
    ComposeSource, ConversionKind, ExportAdapter, Identifier, ImportAdapter, LossPolicy, Network, NetworkAttachment,
    NetworkMode, PlatformVersion, Port, Protocol, QuadletDocumentInput, QuadletExporter, QuadletGroupingPolicy,
    QuadletImporter, QuadletSource, ResourceOwnership, SourceId, Sourced, TargetProfile, convert,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn host_network_survives_compose_quadlet_compose_with_source_evidence() -> TestResult {
    let source = compose_source("    network_mode: host\n    expose: ['9090']\n")?;
    let compose_adapter = ComposeImporter::new()?;
    let imported = compose_adapter.import(&source);
    assert!(imported.diagnostics().is_empty(), "{:?}", imported.diagnostics());
    let application = imported.application().ok_or("application expected")?;
    let service = application.services()[0].value();
    let mode = service.network_mode().ok_or("host mode expected")?;
    assert_eq!(mode.value(), &NetworkMode::Host);
    assert!(!mode.origins().is_empty());
    assert!(service.networks().is_empty());
    assert!(application.networks().is_empty());
    assert_eq!(service.exposed_ports().ok_or("exposure expected")?.len(), 1);

    for version in [PlatformVersion::new(5, 4, 0), PlatformVersion::new(6, 1, 2)] {
        let target = TargetProfile::new("podman", version, Some(version))?;
        let converted = convert(
            &compose_adapter,
            &source,
            &QuadletExporter::new()?,
            &target,
            LossPolicy::ExactOnly,
        )?;
        assert!(converted.diagnostics().is_empty(), "{:?}", converted.diagnostics());
        let output = converted.output().ok_or("Quadlet output expected")?;
        let text = output.file("web.container").ok_or("container expected")?.text();
        assert!(text.lines().any(|line| line == "Network=host"));
        assert!(!text.contains("host.network"));
        assert!(converted.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.network_mode"
                && outcome.kind() == ConversionKind::Exact
                && outcome.origins() == mode.origins()
        }));
        let quadlet = quadlet_source(text)?;
        let reimported = QuadletImporter::new()?.import(&quadlet);
        assert!(reimported.diagnostics().is_empty(), "{:?}", reimported.diagnostics());
        let reimported_application = reimported.application().ok_or("reimported application expected")?;
        let reimported_mode = reimported_application.services()[0]
            .value()
            .network_mode()
            .ok_or("host mode expected")?;
        assert_eq!(reimported_mode.value(), &NetworkMode::Host);
        assert!(!reimported_mode.origins().is_empty());
        assert!(reimported_application.networks().is_empty());

        let compose = convert(
            &QuadletImporter::new()?,
            &quadlet,
            &ComposeExporter::new()?,
            &compose_target()?,
            LossPolicy::ExactOnly,
        )?;
        let text = compose.output().ok_or("Compose output expected")?.text();
        assert!(text.contains("    network_mode: host\n"));
        assert!(!text.contains("    networks:"));
        let fresh = compose_text_source(text)?;
        let fixed = convert(
            &compose_adapter,
            &fresh,
            &ComposeExporter::new()?,
            &compose_target()?,
            LossPolicy::ExactOnly,
        )?;
        assert_eq!(fixed.output().ok_or("fixed point expected")?.text(), text);
    }
    Ok(())
}

#[test]
fn merged_compose_host_override_retains_every_authored_origin() -> TestResult {
    let base_id = ComposeSourceId::new(10);
    let override_id = ComposeSourceId::new(11);
    let loaded = LoadedProject::load([
        DocumentInput::new(
            base_id,
            DocumentOrigin::new("base.yaml", "base.yaml"),
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n    network_mode: none\n",
        ),
        DocumentInput::new(
            override_id,
            DocumentOrigin::new("override.yaml", "override.yaml"),
            "---\nservices:\n  web:\n    network_mode: host\n",
        ),
    ])?;
    let merged = merge_project(&loaded, None);
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("host-example")?,
    )?
    .with_source_id(base_id, SourceId::new("base.yaml")?)
    .with_source_id(override_id, SourceId::new("override.yaml")?);
    let imported = ComposeImporter::new()?.import(&source);
    assert!(imported.diagnostics().is_empty(), "{:?}", imported.diagnostics());
    let application = imported.application().ok_or("application expected")?;
    let mode = application.services()[0]
        .value()
        .network_mode()
        .ok_or("host expected")?;
    assert_eq!(mode.value(), &NetworkMode::Host);
    assert_eq!(
        mode.origins()
            .iter()
            .map(|origin| origin.source_id().as_str())
            .collect::<Vec<_>>(),
        ["base.yaml", "override.yaml"]
    );
    assert!(mode.origins().iter().all(|origin| origin.span().is_some()));
    let plan = QuadletExporter::new()?.plan(application, &quadlet_target()?)?;
    let outcome = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.web.network_mode")
        .ok_or("host export outcome expected")?;
    assert_eq!(outcome.kind(), ConversionKind::Exact);
    assert_eq!(outcome.origins(), mode.origins());
    assert!(!plan.authorize(LossPolicy::ExactOnly).is_blocked());
    Ok(())
}

#[test]
fn mixed_host_and_named_network_services_keep_their_distinct_topology() -> TestResult {
    let source = compose_text_source(concat!(
        "---\nname: host-example\nservices:\n",
        "  hostsvc:\n    image: example.invalid/host:1\n    network_mode: host\n",
        "  bridged:\n    image: example.invalid/bridged:1\n    networks: [front]\n",
        "networks:\n  front: {}\n",
    ))?;
    let result = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &quadlet_target()?,
        LossPolicy::ExactOnly,
    )?;
    assert!(result.diagnostics().is_empty(), "{:?}", result.diagnostics());
    let output = result.output().ok_or("Quadlet output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["front.network", "hostsvc.container", "bridged.container"]
    );
    let host = output
        .file("hostsvc.container")
        .ok_or("host container expected")?
        .text();
    let bridged = output
        .file("bridged.container")
        .ok_or("bridged container expected")?
        .text();
    assert_eq!(
        host.lines()
            .filter(|line| line.starts_with("Network="))
            .collect::<Vec<_>>(),
        ["Network=host"]
    );
    assert_eq!(
        bridged
            .lines()
            .filter(|line| line.starts_with("Network="))
            .collect::<Vec<_>>(),
        ["Network=front.network"]
    );
    assert!(output.file("host.network").is_none());
    let reimport = QuadletSource::parse(
        Identifier::new("host-example")?,
        [
            QuadletDocumentInput::new("hostsvc.container", QuadletSourceId::new(1), host),
            QuadletDocumentInput::new("bridged.container", QuadletSourceId::new(2), bridged),
            QuadletDocumentInput::new(
                "front.network",
                QuadletSourceId::new(3),
                output.file("front.network").ok_or("network resource expected")?.text(),
            ),
        ],
    )?
    .into_source();
    let imported = QuadletImporter::new()?.import(&reimport);
    assert!(imported.diagnostics().is_empty(), "{:?}", imported.diagnostics());
    let application = imported.application().ok_or("application expected")?;
    assert_eq!(application.networks().len(), 1);
    assert_eq!(application.networks()[0].value().name().as_str(), "front");
    let host = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "hostsvc")
        .ok_or("host service expected")?
        .value();
    let bridged = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "bridged")
        .ok_or("bridged service expected")?
        .value();
    assert_eq!(
        host.network_mode().ok_or("host mode expected")?.value(),
        &NetworkMode::Host
    );
    assert!(host.networks().is_empty());
    assert!(bridged.network_mode().is_none());
    assert_eq!(bridged.networks()[0].value().network().as_str(), "front");
    Ok(())
}

#[test]
fn quadlet_host_fixed_point_and_unreviewed_target_versions() -> TestResult {
    let source = quadlet_source("[Container]\nImage=example.invalid/web:1\nNetwork=host\n")?;
    let result = convert(
        &QuadletImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &quadlet_target()?,
        LossPolicy::ExactOnly,
    )?;
    let text = result
        .output()
        .ok_or("Quadlet output expected")?
        .file("web.container")
        .ok_or("container expected")?
        .text();
    assert_eq!(text, "[Container]\nImage=example.invalid/web:1\nNetwork=host\n");
    let fresh = quadlet_source(text)?;
    let fixed = convert(
        &QuadletImporter::new()?,
        &fresh,
        &QuadletExporter::new()?,
        &quadlet_target()?,
        LossPolicy::ExactOnly,
    )?;
    assert_eq!(
        fixed
            .output()
            .ok_or("fixed point expected")?
            .file("web.container")
            .ok_or("container expected")?
            .text(),
        text
    );
    for version in [PlatformVersion::new(5, 3, 9), PlatformVersion::new(6, 1, 3)] {
        let target = TargetProfile::new("podman", version, Some(version))?;
        let result = convert(
            &QuadletImporter::new()?,
            &source,
            &QuadletExporter::new()?,
            &target,
            LossPolicy::AllowPartial,
        )?;
        assert!(result.output().is_none());
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "target.versions" && outcome.kind() == ConversionKind::Invalid)
        );
    }
    Ok(())
}

#[test]
fn named_host_network_is_distinct_from_host_namespace() -> TestResult {
    let source = QuadletSource::parse(
        Identifier::new("host-example")?,
        [
            QuadletDocumentInput::new("host.network", QuadletSourceId::new(2), "[Network]\n"),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(3),
                "[Container]\nImage=example.invalid/web:1\nNetwork=host.network\n",
            ),
        ],
    )?
    .into_source();
    let imported = QuadletImporter::new()?.import(&source);
    assert!(imported.diagnostics().is_empty(), "{:?}", imported.diagnostics());
    let application = imported.application().ok_or("application expected")?;
    let service = application.services()[0].value();
    assert!(service.network_mode().is_none());
    assert_eq!(service.networks()[0].value().network().as_str(), "host");
    Ok(())
}

#[test]
fn omitted_mode_keeps_ordinary_networking_and_other_compose_modes_stay_unsupported() -> TestResult {
    let omitted = ComposeImporter::new()?.import(&compose_source("")?);
    assert!(
        omitted.application().ok_or("application expected")?.services()[0]
            .value()
            .network_mode()
            .is_none()
    );
    for mode in ["none", "service:other", "container:other", "bridge", "pasta"] {
        let source = compose_source(&format!("    network_mode: {mode}\n"))?;
        let imported = ComposeImporter::new()?.import(&source);
        assert!(
            imported.application().ok_or("application expected")?.services()[0]
                .value()
                .network_mode()
                .is_none()
        );
        assert!(
            imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services.web.network_mode" && outcome.kind() == ConversionKind::Unsupported
            }),
            "{mode}: {:?}",
            imported.outcomes()
        );
        let result = convert(
            &ComposeImporter::new()?,
            &source,
            &QuadletExporter::new()?,
            &quadlet_target()?,
            LossPolicy::ExactOnly,
        )?;
        assert!(result.is_blocked());
    }
    Ok(())
}

#[test]
fn compose_host_conflicts_retain_host_intent_and_fail_under_every_policy() -> TestResult {
    for declarations in [
        "    ports: ['8080:80']\n",
        "    networks: [front]\n",
        "    networks: []\n",
    ] {
        let text = format!(
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n    network_mode: host\n{declarations}networks:\n  front: {{}}\n"
        );
        let source = compose_text_source(&text)?;
        let imported = ComposeImporter::new()?.import(&source);
        assert_eq!(
            imported.application().ok_or("application expected")?.services()[0]
                .value()
                .network_mode()
                .ok_or("host expected")?
                .value(),
            &NetworkMode::Host
        );
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Invalid)
        );
        for policy in [
            LossPolicy::ExactOnly,
            LossPolicy::AllowApproximate,
            LossPolicy::AllowPartial,
        ] {
            assert!(
                convert(
                    &ComposeImporter::new()?,
                    &source,
                    &QuadletExporter::new()?,
                    &quadlet_target()?,
                    policy
                )
                .is_err()
            );
        }
    }
    Ok(())
}

#[test]
fn quadlet_host_conflicts_are_invalid_in_either_declaration_order() -> TestResult {
    for conflict in ["Network=front\n", "PublishPort=8080:80\n"] {
        for entries in [format!("Network=host\n{conflict}"), format!("{conflict}Network=host\n")] {
            let source = quadlet_source(&format!("[Container]\nImage=example.invalid/web:1\n{entries}"))?;
            let imported = QuadletImporter::new()?.import(&source);
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.kind() == ConversionKind::Invalid),
                "{imported:?}"
            );
            assert!(
                convert(
                    &QuadletImporter::new()?,
                    &source,
                    &ComposeExporter::new()?,
                    &compose_target()?,
                    LossPolicy::AllowPartial
                )
                .is_err()
            );
        }
    }
    let source = QuadletSource::parse(
        Identifier::new("host-example")?,
        [
            QuadletDocumentInput::new("group.pod", QuadletSourceId::new(2), "[Pod]\n"),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(3),
                "[Container]\nImage=example.invalid/web:1\nNetwork=host\nPod=group.pod\n",
            ),
        ],
    )?
    .into_source();
    let imported = QuadletImporter::new()?.import(&source);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    Ok(())
}

#[test]
fn quadlet_other_network_modes_remain_explicit() -> TestResult {
    for mode in ["none", "bridge", "private", "slirp4netns", "pasta", "host:option"] {
        let source = quadlet_source(&format!("[Container]\nImage=example.invalid/web:1\nNetwork={mode}\n"))?;
        let imported = QuadletImporter::new()?.import(&source);
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Unsupported),
            "{mode}: {imported:?}"
        );
        assert!(
            imported.application().ok_or("application expected")?.services()[0]
                .value()
                .network_mode()
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn repeated_quadlet_host_declarations_are_invalid_without_silent_repair() -> TestResult {
    let source = quadlet_source("[Container]\nImage=example.invalid/web:1\nNetwork=host\nNetwork=host\n")?;
    let imported = QuadletImporter::new()?.import(&source);
    let application = imported.application().ok_or("application expected")?;
    let origins = application.services()[0]
        .value()
        .network_mode()
        .ok_or("host expected")?
        .origins();
    assert_eq!(origins.len(), 2);
    let invalid = imported
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.web.network_mode" && outcome.kind() == ConversionKind::Invalid)
        .ok_or("duplicate host selection must be invalid")?;
    assert_eq!(invalid.origins(), origins);
    assert_eq!(
        imported
            .outcomes()
            .iter()
            .filter(|outcome| {
                outcome.subject() == "services.web.network_mode" && outcome.kind() == ConversionKind::Exact
            })
            .count(),
        1
    );
    for policy in [LossPolicy::ExactOnly, LossPolicy::AllowPartial] {
        assert!(
            convert(
                &QuadletImporter::new()?,
                &source,
                &ComposeExporter::new()?,
                &compose_target()?,
                policy
            )
            .is_err()
        );
        assert!(
            convert(
                &QuadletImporter::new()?,
                &source,
                &QuadletExporter::new()?,
                &quadlet_target()?,
                policy
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn neutral_host_conflicts_and_pod_grouping_cannot_be_authorized_as_partial() -> TestResult {
    let imported = ComposeImporter::new()?.import(&compose_source("    network_mode: host\n")?);
    let application = imported.application().ok_or("application expected")?;
    for grouping in [
        QuadletGroupingPolicy::SinglePod,
        QuadletGroupingPolicy::PreserveSingleGroup,
    ] {
        let plan = QuadletExporter::new()?
            .with_grouping_policy(grouping)
            .plan(application, &quadlet_target()?)?;
        assert!(plan.outcomes().iter().any(
            |outcome| outcome.subject() == "services.web.network_mode" && outcome.kind() == ConversionKind::Invalid
        ));
        assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());
    }
    for attach_network in [false, true] {
        let mut conflicted = boxferry::Application::new(Identifier::new("host-example")?);
        let mut service = application.services()[0].value().clone();
        if attach_network {
            let name = Identifier::new("front")?;
            conflicted.add_network(Sourced::generated(Network::new(
                name.clone(),
                ResourceOwnership::Application,
            )))?;
            service.add_network(Sourced::generated(NetworkAttachment::new(name, Vec::new())));
        } else {
            service.add_port(Sourced::generated(Port::new(80, Some(8080), None, Protocol::Tcp)?));
        }
        conflicted.add_service(Sourced::generated(service))?;
        let compose = ComposeExporter::new()?.plan(&conflicted, &compose_target()?)?;
        let quadlet = QuadletExporter::new()?.plan(&conflicted, &quadlet_target()?)?;
        assert!(compose.authorize(LossPolicy::AllowPartial).output().is_none());
        assert!(quadlet.authorize(LossPolicy::AllowPartial).output().is_none());
    }
    Ok(())
}

fn compose_source(declarations: &str) -> Result<ComposeSource, Box<dyn Error>> {
    compose_text_source(&format!(
        "---\nname: host-example\nservices:\n  web:\n    image: example.invalid/web:1\n{declarations}"
    ))
}

fn compose_text_source(text: &str) -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(1);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("compose.yaml", "compose.yaml"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    Ok(ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("host-example")?,
    )?
    .with_source_id(source_id, SourceId::new("compose.yaml")?))
}

fn quadlet_source(text: &str) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(
        Identifier::new("host-example")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            text,
        )],
    )?
    .into_source())
}

fn compose_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?)
}

fn quadlet_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 1, 2)),
    )?)
}
