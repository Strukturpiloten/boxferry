//! Public route coverage for typed network settings and conservative IPAM boundaries.

#![cfg(all(feature = "compose", feature = "quadlet"))]

use std::error::Error;

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    model::ServiceNetworks,
    source::SourceId as ComposeSourceId,
};
use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    ComposeExporter, ComposeImporter, ComposeSource, ConversionKind, DOCKER_COMPOSE_TARGET, Identifier, ImportAdapter,
    LossPolicy, NetworkDriverOption, NetworkIpamConfig, PlatformVersion, QuadletDocumentInput, QuadletExporter,
    QuadletFile, QuadletImporter, QuadletSource, SourceId, TargetProfile, convert,
};

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(application_name, inputs)?.into_source())
}

#[test]
fn compose_network_slice_emits_all_ten_keys_at_the_floor_and_ceiling() -> Result<(), Box<dyn Error>> {
    let source = compose_network_source()?;
    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?;

    for target in [podman_target(5, 4, 0, 5, 4, 2)?, podman_target(6, 1, 0, 6, 1, 0)?] {
        let result = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
        assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
        let network = result
            .output()
            .and_then(|output| output.file("front.network"))
            .map(QuadletFile::text)
            .ok_or("exact network output expected")?;
        assert_eq!(
            network,
            concat!(
                "[Network]\nNetworkName=runtime-front\nDriver=bridge\nOptions=mtu=1500\n",
                "Label=org.example.owner=frontend\nInternal=true\nIPv6=false\n",
                "IPAMDriver=host-local\nSubnet=10.88.0.0/16\nGateway=10.88.0.1\n",
                "IPRange=10.88.1.0/24\n",
            )
        );
    }

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
fn quadlet_network_import_keeps_one_row_and_reports_compose_ipam_loss() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("network")?,
        [
            QuadletDocumentInput::new(
                "front.network",
                QuadletSourceId::new(1),
                concat!(
                    "[Network]\nNetworkName=runtime-front\nDriver=bridge\nOptions=mtu=1500\n",
                    "Label=org.example.owner=private-network-token\nInternal=true\nIPv6=false\n",
                    "IPAMDriver=host-local\nSubnet=10.88.0.0/16\nGateway=10.88.0.1\n",
                    "IPRange=10.88.1.0/24\n",
                ),
            ),
            QuadletDocumentInput::new(
                "web.container",
                QuadletSourceId::new(2),
                "[Container]\nImage=example.invalid/web:1\nNetwork=front.network\n",
            ),
        ],
    )?;
    let quadlet_importer = QuadletImporter::new()?;
    let imported = quadlet_importer.import(&source);
    let network = imported.application().ok_or("application expected")?.networks()[0].value();
    let option: &NetworkDriverOption = imported.application().ok_or("application expected")?.networks()[0]
        .value()
        .driver_options()
        .ok_or("network option expected")?[0]
        .value();
    assert_eq!(option.name().value().as_str(), "mtu");
    let row: &NetworkIpamConfig = network.ipam_configs().ok_or("IPAM row expected")?[0].value();
    assert_eq!(row.subnet().value().expose(), "10.88.0.0/16");
    assert_eq!(row.gateway().map(|value| value.value().expose()), Some("10.88.0.1"));
    assert_eq!(row.ip_range().map(|value| value.value().expose()), Some("10.88.1.0/24"));

    let version = PlatformVersion::new(2, 30, 0);
    let result = convert(
        &quadlet_importer,
        &source,
        &ComposeExporter::new()?,
        &TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?,
        LossPolicy::AllowPartial,
    )?;
    let output = result.output().ok_or("partial Compose output expected")?.text();
    assert!(output.starts_with("---\nname: network\n"));
    for fragment in [
        "name: runtime-front",
        "driver: bridge",
        "mtu: \"1500\"",
        "org.example.owner: private-network-token",
        "internal: true",
        "enable_ipv6: false",
    ] {
        assert!(output.contains(fragment), "missing {fragment} in {output}");
    }
    for subject in ["networks.front.ipam.driver", "networks.front.ipam.config"] {
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }
    assert!(!format!("{imported:?}").contains("private-network-token"));
    Ok(())
}

#[test]
fn quadlet_network_resets_duplicates_and_multi_row_ipam_stay_explicit() -> Result<(), Box<dyn Error>> {
    for (name, text) in [
        ("reset", "[Network]\nOptions=\nSubnet=\n"),
        ("duplicate", "[Network]\nOptions=mtu=1500\nOptions=mtu=1400\n"),
        (
            "ambiguous",
            concat!(
                "[Network]\nSubnet=10.88.0.0/16\nGateway=10.88.0.1\n",
                "Subnet=10.89.0.0/16\nGateway=10.89.0.1\n",
            ),
        ),
    ] {
        let source = parse_source(
            Identifier::new(name)?,
            [QuadletDocumentInput::new(
                format!("{name}.network"),
                QuadletSourceId::new(3),
                text,
            )],
        )?;
        let result = QuadletImporter::new()?.import(&source);
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
        );
    }
    Ok(())
}

#[test]
fn compose_and_quadlet_facade_reimports_preserve_protected_realtime_aliases() -> Result<(), Box<dyn Error>> {
    let compose = compose_realtime_alias_source()?;
    let quadlet = parse_source(
        Identifier::new("realtime-aliases")?,
        [
            QuadletDocumentInput::new("backend.network", QuadletSourceId::new(41), "[Network]\n"),
            QuadletDocumentInput::new(
                "realtime.container",
                QuadletSourceId::new(42),
                concat!(
                    "[Container]\nImage=example.invalid/realtime:1\nNetwork=backend.network\n",
                    "NetworkAlias=realtime-dev.supabase-realtime\nNetworkAlias=realtime\n",
                ),
            ),
        ],
    )?;
    let target_version = PlatformVersion::new(2, 30, 0);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, target_version, Some(target_version))?;
    let exporter = ComposeExporter::new()?;
    let compose_result = convert(
        &ComposeImporter::new()?,
        &compose,
        &exporter,
        &target,
        LossPolicy::ExactOnly,
    )?;
    let quadlet_result = convert(
        &QuadletImporter::new()?,
        &quadlet,
        &exporter,
        &target,
        LossPolicy::ExactOnly,
    )?;

    for (route, result) in [
        ("compose-to-compose", compose_result),
        ("quadlet-to-compose", quadlet_result),
    ] {
        assert!(!result.is_blocked(), "{route}: {:#?}", result.diagnostics());
        assert_eq!(result.diagnostics(), &[], "{route}");
        assert!(
            result
                .outcomes()
                .iter()
                .all(|outcome| outcome.kind() == ConversionKind::Exact),
            "{route}: {:#?}",
            result.outcomes()
        );
        let output = result.output().ok_or("exact Compose output expected")?;
        assert!(
            output.is_sensitive(),
            "{route} must retain protected alias classification"
        );
        let service = output
            .document()
            .service("realtime")
            .ok_or("generated realtime service expected")?;
        let ServiceNetworks::Long { networks, .. } = service.networks().ok_or("generated service networks expected")?
        else {
            return Err(format!("{route} did not generate long network syntax").into());
        };
        let aliases = networks
            .first()
            .ok_or("generated backend attachment expected")?
            .aliases()
            .iter()
            .map(|alias| alias.value().as_str())
            .collect::<Vec<_>>();
        assert_eq!(aliases, ["realtime-dev.supabase-realtime", "realtime"], "{route}");
    }
    Ok(())
}

fn compose_realtime_alias_source() -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(140);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("realtime.compose.yaml", "realtime.compose.yaml"),
        concat!(
            "---\nname: realtime-aliases\nservices:\n  realtime:\n",
            "    image: example.invalid/realtime:1\n    networks:\n      backend:\n",
            "        aliases: [realtime-dev.supabase-realtime, realtime]\nnetworks:\n  backend: {}\n",
        ),
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged project expected")?.clone();
    Ok(ComposeSource::new(project, Identifier::new("realtime-aliases")?)?
        .with_source_id(source_id, SourceId::new("realtime.compose.yaml")?))
}

fn compose_network_source() -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(120);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("network.compose.yaml", "network.compose.yaml"),
        concat!(
            "services:\n  web:\n    image: example.invalid/web:1\n    networks: [front]\n",
            "networks:\n  front:\n    name: runtime-front\n    driver: bridge\n",
            "    driver_opts: {mtu: '1500'}\n    labels: {org.example.owner: frontend}\n",
            "    internal: true\n    enable_ipv6: false\n    ipam:\n      driver: host-local\n",
            "      config:\n        - subnet: 10.88.0.0/16\n          gateway: 10.88.0.1\n",
            "          ip_range: 10.88.1.0/24\n",
        ),
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged project expected")?.clone();
    Ok(ComposeSource::new(project, Identifier::new("network")?)?
        .with_source_id(source_id, SourceId::new("network.compose.yaml")?))
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
