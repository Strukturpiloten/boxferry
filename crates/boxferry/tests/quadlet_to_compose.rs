//! Public Quadlet-to-Compose route contract.

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId;
use boxferry::{
    ComposeExporter, DOCKER_COMPOSE_TARGET, Identifier, LossPolicy, PlatformVersion, QuadletDocumentInput,
    QuadletImporter, QuadletSource, TargetProfile, convert,
};

#[test]
fn quadlet_input_composes_with_the_existing_compose_exporter() -> Result<(), Box<dyn Error>> {
    let source = QuadletSource::parse(
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
            "name: \"example\"\n",
            "services:\n",
            "  \"web\":\n",
            "    container_name: \"web-runtime\"\n",
            "    image: \"example.invalid/web:1\"\n",
            "    restart: \"no\"\n"
        )
    );
    Ok(())
}

#[test]
fn shared_quadlet_service_fields_convert_to_compose_without_loss() -> Result<(), Box<dyn Error>> {
    let source = QuadletSource::parse(
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
            "name: \"example\"\n",
            "services:\n",
            "  \"web\":\n",
            "    image: \"example.invalid/web:1\"\n",
            "    command:\n",
            "      - \"serve\"\n",
            "      - \"--port=8080\"\n",
            "    environment:\n",
            "      - \"MODE=production\"\n",
            "    labels:\n",
            "      \"org.example.role\": \"frontend\"\n",
            "    user: \"1001:1002\"\n",
            "    group_add:\n",
            "      - \"44\"\n",
            "    working_dir: \"/srv/app\"\n",
            "    read_only: true\n",
            "    extra_hosts:\n",
            "      - \"database=192.0.2.10\"\n",
            "    ports:\n",
            "      - target: 80\n",
            "        published: \"8080\"\n",
            "        host_ip: \"127.0.0.1\"\n",
            "        protocol: \"tcp\"\n",
            "    volumes:\n",
            "      - type: \"volume\"\n",
            "        source: \"data\"\n",
            "        target: \"/var/lib/data\"\n",
            "        read_only: true\n",
            "      - type: \"bind\"\n",
            "        source: \"/srv/config\"\n",
            "        target: \"/etc/config\"\n",
            "    networks:\n",
            "      \"frontend\": {}\n",
            "networks:\n",
            "  \"frontend\": {}\n",
            "volumes:\n",
            "  \"data\": {}\n",
        )
    );
    Ok(())
}

#[test]
fn converts_quadlet_environment_files_after_parser_parity_authorization() -> Result<(), Box<dyn Error>> {
    let source = QuadletSource::parse(
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
            "name: \"example\"\n",
            "services:\n",
            "  \"web\":\n",
            "    image: \"example.invalid/web:1\"\n",
            "    env_file:\n",
            "      - \"/etc/example/default.env\"\n",
            "      - \"/run/credentials/private.env\"\n",
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
