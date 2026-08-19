//! Public Quadlet-to-Compose route contract.

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId;
use boxferry::{
    ComposeExporter, ConversionKind, DOCKER_COMPOSE_TARGET, Identifier, LossPolicy, PlatformVersion,
    QuadletDocumentInput, QuadletImporter, QuadletSource, TargetProfile, convert,
};

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(application_name, inputs)?.into_source())
}

#[test]
fn quadlet_input_composes_with_the_existing_compose_exporter() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            SourceId::new(1),
            "[Service]\nRestart=no\n[Container]\nImage=example.invalid/web:1\nContainerName=web-runtime\n",
        )],
    )?;
    let importer = QuadletImporter::new()?;
    let exporter = ComposeExporter::new()?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;

    let result = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    let output = result.output().ok_or("expected generated Compose output")?;
    assert_eq!(
        output.text(),
        concat!(
            "---\n",
            "name: example\n",
            "services:\n",
            "  web:\n",
            "    container_name: web-runtime\n",
            "    image: example.invalid/web:1\n",
            "    restart: \"no\"\n"
        )
    );
    Ok(())
}

#[test]
fn pod_runtime_settings_remain_group_scoped_and_never_leak_into_compose_services() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("grouped")?,
        [
            QuadletDocumentInput::new("frontend.network", SourceId::new(1), "[Network]\n"),
            QuadletDocumentInput::new("cache.volume", SourceId::new(2), "[Volume]\n"),
            QuadletDocumentInput::new(
                "grouped.pod",
                SourceId::new(3),
                concat!(
                    "[Pod]\nPodName=grouped\nServiceName=grouped-service\n",
                    "AddHost=host.docker.internal:host-gateway\nPublishPort=8080:80\n",
                    "Network=frontend.network\nUserNS=keep-id\nVolume=cache.volume:/cache\n",
                    "ShmSize=64m\nExitPolicy=continue\nStopTimeout=30\n",
                ),
            ),
            QuadletDocumentInput::new(
                "web.container",
                SourceId::new(4),
                "[Container]\nImage=example.invalid/web:1\nPod=grouped.pod\n",
            ),
        ],
    )?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;
    let result = convert(
        &QuadletImporter::new()?,
        &source,
        &ComposeExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let output = result.output().ok_or("partial Compose output expected")?.text();
    assert!(output.contains("image: example.invalid/web:1"));
    for native_group_value in [
        "grouped-service",
        "host.docker.internal",
        "8080",
        "keep-id",
        "64m",
        "continue",
        "30",
    ] {
        assert!(
            !output.contains(native_group_value),
            "group runtime leaked into Compose: {native_group_value} in {output}"
        );
    }
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "service_groups.grouped.runtime" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn shared_quadlet_service_fields_convert_to_compose_without_loss() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("example")?,
        [
            QuadletDocumentInput::new(
                "web.container",
                SourceId::new(1),
                concat!(
                    "[Container]\n",
                    "Image=example.invalid/web:1\n",
                    "Exec=serve --port=8080\n",
                    "Environment=MODE=production\n",
                    "Label=org.example.role=frontend\n",
                    "AddHost=database:192.0.2.10\n",
                    "User=1001\n",
                    "Group=1002\n",
                    "GroupAdd=44\n",
                    "WorkingDir=/srv/app\n",
                    "ReadOnly=true\n",
                    "PublishPort=127.0.0.1:8080:80/tcp\n",
                    "Volume=data.volume:/var/lib/data:ro\n",
                    "Volume=/srv/config:/etc/config\n",
                    "Network=frontend.network\n",
                ),
            ),
            QuadletDocumentInput::new("frontend.network", SourceId::new(2), "[Network]\n"),
            QuadletDocumentInput::new("data.volume", SourceId::new(3), "[Volume]\n"),
        ],
    )?;
    let importer = QuadletImporter::new()?;
    let exporter = ComposeExporter::new()?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;

    let result = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    let output = result.output().ok_or("expected generated Compose output")?;
    assert_eq!(
        output.text(),
        concat!(
            "---\n",
            "name: example\n",
            "services:\n",
            "  web:\n",
            "    image: example.invalid/web:1\n",
            "    command:\n",
            "      - serve\n",
            "      - \"--port=8080\"\n",
            "    environment:\n",
            "      - MODE=production\n",
            "    labels:\n",
            "      org.example.role: frontend\n",
            "    user: \"1001:1002\"\n",
            "    group_add:\n",
            "      - \"44\"\n",
            "    working_dir: /srv/app\n",
            "    read_only: true\n",
            "    extra_hosts:\n",
            "      - database=192.0.2.10\n",
            "    ports:\n",
            "      - target: 80\n",
            "        published: \"8080\"\n",
            "        host_ip: 127.0.0.1\n",
            "        protocol: tcp\n",
            "    volumes:\n",
            "      - type: volume\n",
            "        source: data\n",
            "        target: /var/lib/data\n",
            "        read_only: true\n",
            "      - type: bind\n",
            "        source: /srv/config\n",
            "        target: /etc/config\n",
            "    networks:\n",
            "      frontend: {}\n",
            "networks:\n",
            "  frontend: {}\n",
            "volumes:\n",
            "  data: {}\n",
        )
    );
    Ok(())
}

#[test]
fn public_quadlet_to_compose_route_reconstructs_canonical_security_options() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("security")?,
        [QuadletDocumentInput::new(
            "web.container",
            SourceId::new(4),
            concat!(
                "[Container]\nImage=example.invalid/web:1\n",
                "AppArmor=profile-a\nNoNewPrivileges=false\nSeccompProfile=secret-seccomp.json\n",
                "SecurityLabelDisable=true\nSecurityLabelFileType=container_file_t\n",
                "SecurityLabelLevel=s0:c1,c2\nSecurityLabelNested=true\n",
                "SecurityLabelType=container_t\nMask=/proc/acpi\nUnmask=/proc/acpi\nMask=/proc/acpi\n",
            ),
        )],
    )?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;
    let result = convert(
        &QuadletImporter::new()?,
        &source,
        &ComposeExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    let output = result
        .output()
        .ok_or("partial Compose security-option output expected")?;
    assert!(!format!("{output:?}").contains("secret-seccomp.json"));
    assert!(output.text().contains(concat!(
        "security_opt:\n",
        "      - apparmor=profile-a\n",
        "      - no-new-privileges:false\n",
        "      - seccomp=secret-seccomp.json\n",
        "      - label:disable\n",
        "      - label:filetype:container_file_t\n",
        "      - label:level:s0:c1,c2\n",
        "      - label:nested\n",
        "      - label:type:container_t\n",
        "      - mask=/proc/acpi\n",
        "      - unmask=/proc/acpi\n",
        "      - mask=/proc/acpi\n",
    )));
    for index in 0..11 {
        assert!(result.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.security_options[{index}]")
                && outcome.kind() == ConversionKind::Exact
        }));
    }
    Ok(())
}

#[test]
fn false_quadlet_selinux_booleans_remain_neutral_but_are_not_invented_in_compose() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("security-false")?,
        [QuadletDocumentInput::new(
            "web.container",
            SourceId::new(5),
            concat!(
                "[Container]\nImage=example.invalid/web:1\n",
                "SecurityLabelDisable=false\nSecurityLabelNested=false\nMask=/proc/acpi\n",
            ),
        )],
    )?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;
    let result = convert(
        &QuadletImporter::new()?,
        &source,
        &ComposeExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let output = result.output().ok_or("partial Compose output expected")?;
    assert!(output.text().contains("- mask=/proc/acpi"));
    assert!(!output.text().contains("label:disable:false"));
    assert!(!output.text().contains("label:nested:false"));
    for index in [0, 1] {
        assert!(result.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.security_options[{index}]")
                && outcome.kind() == ConversionKind::Exact
        }));
        assert!(result.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.security_opt[{index}]")
                && outcome.kind() == ConversionKind::Unsupported
        }));
    }
    Ok(())
}

#[test]
fn converts_quadlet_environment_files_after_parser_parity_authorization() -> Result<(), Box<dyn Error>> {
    let source = parse_source(
        Identifier::new("example")?,
        [QuadletDocumentInput::new(
            "web.container",
            SourceId::new(1),
            concat!(
                "[Container]\n",
                "Image=example.invalid/web:1\n",
                "EnvironmentFile=/etc/example/default.env\n",
                "EnvironmentFile=/run/credentials/private.env\n",
            ),
        )],
    )?;
    let importer = QuadletImporter::new()?;
    let exporter = ComposeExporter::new()?;
    let version = PlatformVersion::new(5, 3, 1);
    let target = TargetProfile::new(DOCKER_COMPOSE_TARGET, version, Some(version))?;

    let strict = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(strict.is_blocked());
    assert!(strict.output().is_none());

    let approximate = convert(&importer, &source, &exporter, &target, LossPolicy::AllowApproximate)?;
    assert!(!approximate.is_blocked(), "{:#?}", approximate.diagnostics());
    assert_eq!(
        approximate.output().ok_or("expected generated Compose output")?.text(),
        concat!(
            "---\n",
            "name: example\n",
            "services:\n",
            "  web:\n",
            "    image: example.invalid/web:1\n",
            "    env_file:\n",
            "      - /etc/example/default.env\n",
            "      - /run/credentials/private.env\n",
        )
    );
    assert_eq!(
        approximate
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFC0007")
            .count(),
        0
    );
    Ok(())
}
