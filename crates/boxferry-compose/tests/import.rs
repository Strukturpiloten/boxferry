//! Public adapter behavior backed by the repository fixture contract.

use std::{error::Error, fs, path::PathBuf};

use boxferry_compose::{ComposeImporter, ComposeSource};
use boxferry_engine::{ConversionKind, ImportAdapter, Severity};
use boxferry_model::{
    BuildSourceDeclaration, BuildSyntax, Command, ConfigMaterial, Entrypoint, EnvironmentFileFormat,
    EnvironmentFileSyntax, EnvironmentValue, HealthcheckCommand, HostAddressKind, Identifier, ImageBuildSetting,
    MountSource, ProtectedString, Protocol, PullPolicy, ResourceGrantSyntax, ResourceOwnership, RestartPolicy,
    SecretMaterial, SecurityOption, SelinuxRelabel, Service, ServiceDependencyCondition, SourceBuildSetting, SourceId,
};

#[test]
fn imports_iteration_one_compose_fields_without_conflating_command_or_published_ports() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(111);
    let project = merged_project([(
        id,
        "iteration-one.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    command: [serve]\n    entrypoint: [/bin/web]\n    init: true\n    stop_grace_period: 1m30s\n    pull_policy: daily\n    mem_limit: 256m\n    expose: [8080, 5353/udp]\n    annotations: {io.example.note: stable}\n    logging: {driver: json-file, options: {tag: demo}}\n    networks:\n      front:\n        aliases: [web]\n        ipv4_address: 192.0.2.20\n        ipv6_address: 2001:db8::20\nnetworks: {front: {}}\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("iteration-one")?)?
        .with_source_id(id, SourceId::new("iteration-one.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let service = result.application().ok_or("application")?.services()[0].value();
    assert!(
        matches!(service.entrypoint().map(boxferry_model::Sourced::value), Some(Entrypoint::Exec(values)) if values[0].expose() == "/bin/web")
    );
    assert!(
        matches!(service.command().map(boxferry_model::Sourced::value), Some(Command::Exec(values)) if values[0].expose() == "serve")
    );
    assert_eq!(service.run_init().map(|value| *value.value()), Some(true));
    assert_eq!(
        service.stop_timeout().map(|value| value.value().as_str()),
        Some("1m30s")
    );
    assert!(matches!(
        service.pull_policy().map(boxferry_model::Sourced::value),
        Some(PullPolicy::Daily)
    ));
    assert_eq!(service.memory_limit().map(|value| value.value().expose()), Some("256m"));
    assert_eq!(service.exposed_ports().map(<[_]>::len), Some(2));
    assert_eq!(service.ports().len(), 0);
    assert_eq!(
        service
            .annotations()
            .map(|values| values[0].value().name().value().as_str()),
        Some("io.example.note")
    );
    assert_eq!(
        service
            .logging()
            .and_then(|value| value.value().driver())
            .map(|value| value.value().expose()),
        Some("json-file")
    );
    let attachment = service.networks().first().ok_or("network attachment")?.value();
    assert_eq!(attachment.aliases(), ["web"]);
    assert_eq!(
        attachment.ipv4_address().map(|value| value.value().expose()),
        Some("192.0.2.20")
    );
    assert_eq!(
        attachment.ipv6_address().map(|value| value.value().expose()),
        Some("2001:db8::20")
    );
    Ok(())
}

#[test]
fn keeps_compose_runtime_fields_on_the_service_without_inventing_a_group_runtime() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(113);
    let project = merged_project([(
        id,
        "service-scope.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    extra_hosts: [host.docker.internal:host-gateway]\n    ports: [127.0.0.1:18080:8080]\n    userns_mode: keep-id\n    volumes: [cache:/var/cache/web]\n    shm_size: 64m\n    stop_grace_period: 30s\n    networks: [front]\nnetworks: {front: {}}\nvolumes: {cache: {}}\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("service-scope")?)?
        .with_source_id(id, SourceId::new("service-scope.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let application = result.application().ok_or("application")?;
    let service = application.services().first().ok_or("service")?.value();
    assert!(application.service_groups().is_empty());
    assert_eq!(service.host_mappings().len(), 1);
    assert_eq!(service.ports().len(), 1);
    assert_eq!(service.networks().len(), 1);
    assert_eq!(service.mounts().len(), 1);
    assert_eq!(
        service.user_namespace().map(|value| value.value().expose()),
        Some("keep-id")
    );
    assert_eq!(service.shm_size().map(|value| value.value().expose()), Some("64m"));
    assert_eq!(service.stop_timeout().map(|value| value.value().as_str()), Some("30s"));
    for origins in [
        service.host_mappings()[0].origins(),
        service.ports()[0].origins(),
        service.networks()[0].origins(),
        service.mounts()[0].origins(),
        service.user_namespace().ok_or("user namespace")?.origins(),
        service.shm_size().ok_or("shm size")?.origins(),
        service.stop_timeout().ok_or("stop timeout")?.origins(),
    ] {
        assert_eq!(origins[0].source_id().as_str(), "service-scope.yaml");
    }
    Ok(())
}

#[test]
fn retains_empty_entrypoint_and_logging_options_without_a_driver() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(112);
    let project = merged_project([(
        id,
        "empty-iteration-one.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    entrypoint: []\n    logging:\n      options: {}\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("empty-iteration-one")?)?
        .with_source_id(id, SourceId::new("empty-iteration-one.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let service = result.application().ok_or("application")?.services()[0].value();
    assert!(matches!(
        service.entrypoint().map(boxferry_model::Sourced::value),
        Some(Entrypoint::Empty)
    ));
    assert_eq!(service.logging().and_then(|value| value.value().driver()), None);
    assert_eq!(
        service
            .logging()
            .and_then(|value| value.value().options())
            .map(<[_]>::len),
        Some(0)
    );
    assert!(result.outcomes().iter().any(|outcome| outcome.subject() == "services.web.logging" && outcome.kind() == ConversionKind::Unsupported));
    Ok(())
}

#[test]
fn imports_network_definitions_with_runtime_identity_protected_options_and_associated_ipam_rows()
-> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(114);
    let project = merged_project([(
        id,
        "network.yaml",
        "services: {web: {image: example.invalid/web:1, networks: [front, driver-only, existing]}}\nnetworks:\n  front:\n    name: runtime-front\n    driver: bridge\n    driver_opts: {com.example.secret: production-secret}\n    labels: {com.example.role: frontend}\n    internal: false\n    enable_ipv6: true\n    ipam:\n      driver: default\n      config:\n        - subnet: 192.0.2.0/24\n          gateway: 192.0.2.1\n          ip_range: 192.0.2.64/26\n        - subnet: 2001:db8::/64\n          gateway: 2001:db8::1\n  driver-only:\n    ipam: {driver: default}\n  existing:\n    external: true\n    name: platform-existing\n",
    )])?;
    let source =
        ComposeSource::new(project, Identifier::new("networks")?)?.with_source_id(id, SourceId::new("network.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let application = result.application().ok_or("application")?;
    let front = application
        .networks()
        .iter()
        .find(|network| network.value().name().as_str() == "front")
        .ok_or("front")?
        .value();
    assert_eq!(
        front.runtime_name().map(|value| value.value().expose()),
        Some("runtime-front")
    );
    assert_eq!(front.driver().map(|value| value.value().expose()), Some("bridge"));
    assert_eq!(front.driver_options().map(<[_]>::len), Some(1));
    assert!(
        !front.driver_options().ok_or("driver options")?[0]
            .value()
            .value()
            .value()
            .is_sensitive()
    );
    assert_eq!(front.internal().map(|value| *value.value()), Some(false));
    assert_eq!(front.ipv6().map(|value| *value.value()), Some(true));
    assert_eq!(front.ipam_driver().map(|value| value.value().expose()), Some("default"));
    let rows = front.ipam_configs().ok_or("IPAM rows")?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value().subnet().value().expose(), "192.0.2.0/24");
    assert_eq!(
        rows[0].value().gateway().map(|value| value.value().expose()),
        Some("192.0.2.1")
    );
    assert_eq!(rows[1].value().subnet().value().expose(), "2001:db8::/64");
    assert!(
        application
            .networks()
            .iter()
            .find(|network| network.value().name().as_str() == "driver-only")
            .ok_or("driver-only")?
            .value()
            .ipam_configs()
            .is_none()
    );
    assert_eq!(
        application
            .networks()
            .iter()
            .find(|network| network.value().name().as_str() == "existing")
            .ok_or("existing")?
            .value()
            .ownership(),
        ResourceOwnership::External
    );
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "networks.front.driver_opts"
                && outcome.origins()[0].source_id().as_str() == "network.yaml")
    );
    Ok(())
}

#[test]
fn redacts_network_values_when_sensitive_interpolation_marks_the_definition_sensitive() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(115);
    let loaded = LoadedProject::load([DocumentInput::new(
        id,
        DocumentOrigin::new("sensitive-network.yaml", "."),
        "services: {web: {image: example.invalid/web:1, networks: [private]}}\nnetworks:\n  private:\n    driver: ${NETWORK_DRIVER}\n    driver_opts: {token: plaintext-network-token}\n",
    )])?;
    let mut environment = MapEnvironment::new();
    let _ = environment.insert_sensitive("NETWORK_DRIVER", "bridge");
    let interpolation = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let project = merged.project().ok_or("merged project")?.clone();
    let source = ComposeSource::new(project, Identifier::new("sensitive-network")?)?
        .with_source_id(id, SourceId::new("sensitive-network.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let network = result.application().ok_or("application")?.networks()[0].value();
    assert!(
        network.driver_options().ok_or("driver options")?[0]
            .value()
            .value()
            .value()
            .is_sensitive()
    );
    assert!(!format!("{result:?}").contains("plaintext-network-token"));
    Ok(())
}
use compose_lens::{
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::{MergedProject, merge_project},
    profiles::{ProfileRequest, ProfileSelection, select_profiles},
    source::SourceId as ComposeSourceId,
};

const COMPOSE_SOURCE_ID: u32 = 71;
const OVERRIDE_SOURCE_ID: u32 = 72;

#[test]
fn imports_the_core_fixture_without_loss_and_excludes_inactive_profiles() -> Result<(), Box<dyn Error>> {
    let base = fixture_text("compose.yaml")?;
    let overlay = fixture_text("compose.override.yaml")?;
    let (project, selection) = processed_project(&base, &overlay, &ProfileRequest::new())?;
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?)
        .with_source_id(
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            SourceId::new("compose.override.yaml")?,
        )
        .with_profile_selection(selection);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let application = result.application().ok_or("application expected")?;
    assert_eq!(application.name().as_str(), "ferry-demo");
    assert_eq!(application.services().len(), 1);
    assert_eq!(application.volumes().len(), 1);
    assert_eq!(application.networks().len(), 1);

    let web = application.services()[0].value();
    assert_eq!(web.name().as_str(), "web");
    assert_core_runtime_name(web);
    assert_eq!(
        web.image().map(|image| image.value().as_str()),
        Some("registry.example:5000/team/web:1.3@sha256:fedcba")
    );
    assert!(matches!(
        web.command().map(boxferry_model::Sourced::value),
        Some(Command::Exec(values))
            if values
                .iter()
                .map(ProtectedString::expose)
                .eq(["php", "-v"])
    ));
    assert_core_execution_context(web);
    assert_core_restart_policy(web);
    assert_eq!(web.environment().len(), 4);
    assert!(matches!(web.environment()[1].value().value(), EnvironmentValue::Host));
    assert_core_healthcheck(web)?;
    assert!(matches!(
        web.environment()[2].value().value(),
        EnvironmentValue::Literal(value) if value.is_sensitive() && value.expose() == "9090"
    ));
    assert_eq!(web.host_mappings().len(), 3);
    assert_eq!(
        web.host_mappings()[0].value().hostname().as_str(),
        "host.docker.internal"
    );
    assert_eq!(
        web.host_mappings()[0].value().address().kind(),
        HostAddressKind::HostGateway
    );
    assert_eq!(web.host_mappings()[1].value().address().raw(), "[::1]");
    assert_eq!(
        web.host_mappings()[1].value().address().kind(),
        HostAddressKind::Ipv6 { bracketed: true }
    );
    assert_eq!(web.host_mappings()[2].value().hostname().as_str(), "database");
    assert_eq!(
        web.host_mappings()[2]
            .origins()
            .iter()
            .map(|origin| origin.source_id().as_str())
            .collect::<Vec<_>>(),
        ["compose.override.yaml"]
    );
    assert_eq!(web.ports().len(), 2);
    assert_eq!(web.ports()[0].value().container(), 80);
    assert_eq!(web.ports()[0].value().published(), Some(8080));
    assert_eq!(web.ports()[0].value().host_address(), Some("127.0.0.1"));
    assert!(matches!(web.ports()[0].value().protocol(), Protocol::Tcp));
    assert_eq!(web.mounts().len(), 4);
    assert!(matches!(web.mounts()[0].value().source(), MountSource::Volume(name) if name.as_str() == "data"));
    assert!(web.mounts()[0].value().read_only());
    assert_eq!(web.mounts()[1].value().selinux_relabel(), Some(SelinuxRelabel::Private));
    assert_eq!(web.mounts()[2].value().selinux_relabel(), Some(SelinuxRelabel::Shared));
    assert_eq!(web.mounts()[3].value().selinux_relabel(), Some(SelinuxRelabel::Private));
    assert_eq!(web.networks()[0].value().aliases(), ["web.local"]);
    assert_eq!(
        web.image()
            .map(|image| {
                image
                    .origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
    Ok(())
}

#[test]
fn imports_released_container_settings_with_order_and_provenance() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    hostname: web.example\n",
        "    pids_limit: '00042'\n",
        "    shm_size: 64m\n",
        "    cap_drop: [NET_RAW]\n",
        "    cap_add: [SYS_PTRACE]\n",
        "    tmpfs: [/run:mode=1777]\n",
        "    sysctls:\n",
        "      net.ipv4.ip_forward: '1'\n",
        "    ulimits:\n",
        "      nofile:\n",
        "        soft: 1024\n",
        "        hard: 4096\n",
        "    devices: [/dev/fuse:/dev/fuse:rwm]\n",
        "    stop_signal: SIGTERM\n",
    );
    let compose_source_id = ComposeSourceId::new(98);
    let project = merged_project([(compose_source_id, "settings.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("settings")?)?
        .with_source_id(compose_source_id, SourceId::new("settings.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .ok_or("service expected")?
        .value();
    assert_eq!(
        service.hostname().map(|value| value.value().expose()),
        Some("web.example")
    );
    assert_eq!(service.pids_limit().map(|value| value.value().expose()), Some("00042"));
    assert_eq!(service.shm_size().map(|value| value.value().expose()), Some("64m"));
    assert_eq!(
        service.cap_drop().map(|values| values[0].value().expose()),
        Some("NET_RAW")
    );
    assert_eq!(
        service.cap_add().map(|values| values[0].value().expose()),
        Some("SYS_PTRACE")
    );
    assert_eq!(
        service.tmpfs().map(|values| values[0].value().expose()),
        Some("/run:mode=1777")
    );
    assert_eq!(
        service.sysctls().map(|values| values[0].value().name().expose()),
        Some("net.ipv4.ip_forward")
    );
    assert_eq!(
        service
            .ulimits()
            .map(|values| values[0].value().hard().map(|value| value.value().expose())),
        Some(Some("4096"))
    );
    assert!(
        matches!(service.devices().map(|values| values[0].value()), Some(boxferry_model::Device::Short(value)) if value.expose() == "/dev/fuse:/dev/fuse:rwm")
    );
    assert_eq!(
        service.stop_signal().map(|value| value.value().expose()),
        Some("SIGTERM")
    );
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(())
}

#[test]
fn imports_scalar_build_before_its_referencing_service() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(101);
    let project = merged_project([(
        source_id,
        "scalar-build.compose.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    build: ./web\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("builds")?)?
        .with_source_id(source_id, SourceId::new("scalar-build.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let application = result.application().ok_or("application expected")?;
    assert_eq!(application.image_builds().len(), 1);
    let build = application.image_builds()[0].value();
    assert!(matches!(
        build.source_declaration().map(boxferry_model::Sourced::value),
        Some(BuildSourceDeclaration::Scalar(value)) if value.expose() == "./web"
    ));
    assert_eq!(
        application.services()[0]
            .value()
            .image_build()
            .map(|reference| reference.value().as_str()),
        Some(build.name().as_str())
    );
    assert_eq!(
        application.services()[0]
            .value()
            .image_build()
            .map(|reference| reference.origins().len()),
        Some(1)
    );
    assert!(matches!(
        build.settings().and_then(|settings| settings.first()).map(boxferry_model::Sourced::value),
        Some(ImageBuildSetting::ImageTags(values))
            if values.values()[0].value().expose() == "example.invalid/web:1"
    ));
    Ok(())
}

#[test]
fn imports_safe_sequence_build_arguments_without_rewriting_source_syntax() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(104);
    let project = merged_project([(
        source_id,
        "list-args.compose.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    build:\n      args: [ONE=1, BARE, '${DEFERRED}=x', TWO=two=parts]\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("builds")?)?
        .with_source_id(source_id, SourceId::new("list-args.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let build = result
        .application()
        .and_then(|application| application.image_builds().first())
        .ok_or("build expected")?
        .value();
    let source_arguments = build
        .source_declaration()
        .and_then(|declaration| declaration.value().structured_settings())
        .and_then(|settings| {
            settings
                .iter()
                .find(|setting| matches!(setting.value(), SourceBuildSetting::Arguments(_)))
        })
        .ok_or("source arguments")?;
    assert!(matches!(
        source_arguments.value(),
        SourceBuildSetting::Arguments(values)
            if values.syntax() == BuildSyntax::Sequence && values.values().len() == 4
    ));
    let overlap = build
        .settings()
        .and_then(|settings| {
            settings
                .iter()
                .find(|setting| matches!(setting.value(), ImageBuildSetting::BuildArguments(_)))
        })
        .ok_or("overlap arguments")?;
    assert!(matches!(
        overlap.value(),
        ImageBuildSetting::BuildArguments(values)
            if values.syntax() == BuildSyntax::Sequence
                && values.values().iter().map(|value| value.value().name().expose()).eq(["ONE", "TWO"])
                && values.values()[1].value().value().map(ProtectedString::expose) == Some("two=parts")
    ));
    Ok(())
}

#[test]
fn does_not_duplicate_a_service_image_that_is_an_explicit_build_tag() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(105);
    let project = merged_project([(
        source_id,
        "duplicate-tag.compose.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    build:\n      tags: [example.invalid/web:1]\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("builds")?)?
        .with_source_id(source_id, SourceId::new("duplicate-tag.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let build = result
        .application()
        .and_then(|application| application.image_builds().first())
        .ok_or("build expected")?
        .value();
    let tag_settings = build
        .settings()
        .into_iter()
        .flatten()
        .filter(|setting| matches!(setting.value(), ImageBuildSetting::ImageTags(_)))
        .collect::<Vec<_>>();
    assert_eq!(tag_settings.len(), 1);
    assert!(matches!(
        tag_settings[0].value(),
        ImageBuildSetting::ImageTags(values) if values.values().len() == 1
    ));
    Ok(())
}

#[test]
fn imports_inline_recipe_as_source_only_build_intent() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(103);
    let project = merged_project([(
        source_id,
        "inline-build.compose.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    build:\n      dockerfile_inline: |\n        FROM scratch\n",
    )])?;
    let source = ComposeSource::new(project, Identifier::new("builds")?)?
        .with_source_id(source_id, SourceId::new("inline-build.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let build = result
        .application()
        .and_then(|application| application.image_builds().first())
        .ok_or("build expected")?
        .value();
    assert!(matches!(
        build.source_declaration()
            .and_then(|declaration| declaration.value().structured_settings())
            .and_then(|settings| settings.first())
            .map(boxferry_model::Sourced::value),
        Some(SourceBuildSetting::InlineRecipe(value)) if value.expose() == "FROM scratch\n"
    ));
    assert!(matches!(
        build.settings().and_then(|settings| settings.first()).map(boxferry_model::Sourced::value),
        Some(ImageBuildSetting::ImageTags(values)) if values.values()[0].value().expose() == "example.invalid/web:1"
    ));
    Ok(())
}

#[test]
fn imports_structured_build_fields_empty_resets_and_safe_overlap() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(102);
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    build:\n",
        "      additional_contexts: []\n",
        "      args: {}\n",
        "      cache_from: []\n",
        "      cache_to: []\n",
        "      context: ./web\n",
        "      dockerfile: Containerfile\n",
        "      entitlements: []\n",
        "      extra_hosts: []\n",
        "      isolation: default\n",
        "      labels: [com.example.first=one, com.example.first=one]\n",
        "      network: host\n",
        "      no_cache: true\n",
        "      no_cache_filter: []\n",
        "      platforms: []\n",
        "      privileged: false\n",
        "      provenance: mode=max\n",
        "      pull: true\n",
        "      sbom: true\n",
        "      secrets: []\n",
        "      shm_size: 64m\n",
        "      ssh: [default=/run/secret-agent]\n",
        "      tags: [example.invalid/web:one, example.invalid/web:one]\n",
        "      target: release\n",
        "      ulimits: {}\n",
    );
    let project = merged_project([(source_id, "structured-build.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("builds")?)?
        .with_source_id(source_id, SourceId::new("structured-build.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let build = result
        .application()
        .and_then(|application| application.image_builds().first())
        .ok_or("build expected")?
        .value();
    let declaration = build.source_declaration().ok_or("source declaration")?.value();
    assert_eq!(declaration.syntax(), BuildSyntax::Structured);
    let settings = declaration.structured_settings().ok_or("structured settings")?;
    assert_eq!(settings.len(), 24);
    assert!(matches!(
        settings[0].value(),
        SourceBuildSetting::AdditionalContexts(values) if values.values().is_empty()
    ));
    assert!(matches!(
        settings[1].value(),
        SourceBuildSetting::Arguments(values) if values.syntax() == BuildSyntax::Mapping && values.values().is_empty()
    ));
    assert!(matches!(
        settings
            .iter()
            .find(|setting| matches!(setting.value(), SourceBuildSetting::Ssh(_)))
            .map(boxferry_model::Sourced::value),
        Some(SourceBuildSetting::Ssh(values)) if values.values()[0].value().is_sensitive()
    ));
    assert!(matches!(
        build
            .settings()
            .and_then(|settings| {
                settings
                    .iter()
                    .find(|setting| matches!(setting.value(), ImageBuildSetting::RecipeFile(_)))
            })
            .map(boxferry_model::Sourced::value),
        Some(ImageBuildSetting::RecipeFile(value)) if value.expose() == "Containerfile"
    ));
    assert!(matches!(
        build
            .settings()
            .and_then(|settings| {
                settings
                    .iter()
                    .find(|setting| matches!(setting.value(), ImageBuildSetting::Labels(_)))
            })
            .map(boxferry_model::Sourced::value),
        Some(ImageBuildSetting::Labels(values)) if values.values().len() == 2
    ));
    assert!(matches!(
        build
            .settings()
            .and_then(|settings| {
                settings.iter().find(|setting| {
                    matches!(setting.value(), ImageBuildSetting::ImageTags(values) if values.values().len() == 2)
                })
            })
            .map(boxferry_model::Sourced::value),
        Some(ImageBuildSetting::ImageTags(values)) if values.values().len() == 2
    ));
    let debug = format!("{build:?}");
    assert!(!debug.contains("/run/secret-agent"));
    assert!(debug.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn imports_dns_collections_with_order_empty_state_and_special_outcomes() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    dns: [1.1.1.1, 8.8.8.8]\n",
        "    dns_opt: [ndots:5, none, none]\n",
        "    dns_search: []\n",
    );
    let source_id = ComposeSourceId::new(99);
    let source = ComposeSource::new(
        merged_project([(source_id, "dns.compose.yaml", text)])?,
        Identifier::new("dns")?,
    )?
    .with_source_id(source_id, SourceId::new("dns.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
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
        ["1.1.1.1", "8.8.8.8"]
    );
    assert_eq!(
        service
            .dns_options()
            .unwrap_or_default()
            .iter()
            .map(|value| value.value().expose())
            .collect::<Vec<_>>(),
        ["ndots:5", "none", "none"]
    );
    assert!(!service.dns_options_origins().is_empty());
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.dns_opt" && outcome.kind() == ConversionKind::Invalid)
    );
    assert!(service.dns_search_domains().is_some_and(<[_]>::is_empty));
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.dns_search"
                && outcome.kind() == ConversionKind::Unsupported)
    );
    Ok(())
}

#[test]
fn imports_mapping_extra_hosts_without_requiring_ip_only_values() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    extra_hosts:\n",
        "      database: 192.0.2.10\n",
        "      host.docker.internal: host-gateway\n",
    );
    let compose_source_id = ComposeSourceId::new(74);
    let project = merged_project([(compose_source_id, "hosts.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("hosts")?)?
        .with_source_id(compose_source_id, SourceId::new("hosts.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let mappings = result
        .application()
        .and_then(|application| application.services().first())
        .map(|service| service.value().host_mappings())
        .ok_or("host mappings expected")?;
    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].value().address().kind(), HostAddressKind::Ipv4);
    assert_eq!(mappings[1].value().address().kind(), HostAddressKind::HostGateway);
    assert!(mappings.iter().all(|mapping| !mapping.origins().is_empty()));
    Ok(())
}

#[test]
fn imports_ordered_environment_file_declarations_without_reading_them() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    env_file:\n",
        "      - ./base.env\n",
        "      - path: config/production.env\n",
        "        required: true\n",
        "      - path: ./raw.env\n",
        "        format: raw\n",
    );
    let compose_source_id = ComposeSourceId::new(84);
    let project = merged_project([(compose_source_id, "environment-files.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("environment-files")?)?
        .with_source_id(compose_source_id, SourceId::new("environment-files.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let files = result
        .application()
        .and_then(|application| application.services().first())
        .map(|service| service.value().environment_files())
        .ok_or("environment files expected")?;
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].value().path().expose(), "./base.env");
    assert_eq!(files[0].value().syntax(), EnvironmentFileSyntax::Short);
    assert!(files[0].value().required().is_none());
    assert_eq!(files[1].value().syntax(), EnvironmentFileSyntax::Long);
    assert_eq!(
        files[1].value().required().map(boxferry_model::Sourced::value),
        Some(&true)
    );
    assert!(matches!(
        files[2].value().format().map(boxferry_model::Sourced::value),
        Some(EnvironmentFileFormat::Raw)
    ));
    assert!(files.iter().all(|file| !file.origins().is_empty()));
    assert!(
        files[1]
            .value()
            .required()
            .is_some_and(|required| !required.origins().is_empty())
    );
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(())
}

#[test]
fn imports_every_compose_service_restart_policy() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  disabled:\n",
        "    image: example.invalid/disabled:1\n",
        "    restart: \"no\"\n",
        "  always:\n",
        "    image: example.invalid/always:1\n",
        "    restart: always\n",
        "  failure:\n",
        "    image: example.invalid/failure:1\n",
        "    restart: on-failure\n",
        "  limited:\n",
        "    image: example.invalid/limited:1\n",
        "    restart: on-failure:7\n",
        "  stopped:\n",
        "    image: example.invalid/stopped:1\n",
        "    restart: unless-stopped\n",
    );
    let compose_source_id = ComposeSourceId::new(82);
    let project = merged_project([(compose_source_id, "restart.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("restart")?)?
        .with_source_id(compose_source_id, SourceId::new("restart.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let application = result.application().ok_or("application expected")?;
    let policies = application
        .services()
        .iter()
        .map(|service| service.value().restart_policy().map(boxferry_model::Sourced::value))
        .collect::<Vec<_>>();
    assert!(matches!(policies[0], Some(RestartPolicy::Never)));
    assert!(matches!(policies[1], Some(RestartPolicy::Always)));
    assert!(matches!(
        policies[2],
        Some(RestartPolicy::OnFailure { maximum_retries: None })
    ));
    assert!(matches!(
        policies[3],
        Some(RestartPolicy::OnFailure {
            maximum_retries: Some(maximum_retries),
        }) if maximum_retries.get() == 7
    ));
    assert!(matches!(policies[4], Some(RestartPolicy::UnlessStopped)));
    assert!(application.services().iter().all(|service| {
        service
            .value()
            .restart_policy()
            .is_some_and(|restart| !restart.origins().is_empty())
    }));
    Ok(())
}

#[test]
fn rejects_unresolved_zero_and_unrepresentable_restart_limits_without_erasing_services() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  unresolved:\n",
        "    image: example.invalid/unresolved:1\n",
        "    restart: ${RESTART_POLICY}\n",
        "  zero:\n",
        "    image: example.invalid/zero:1\n",
        "    restart: on-failure:0\n",
        "  overflow:\n",
        "    image: example.invalid/overflow:1\n",
        "    restart: on-failure:18446744073709551616\n",
    );
    let compose_source_id = ComposeSourceId::new(83);
    let project = merged_project([(compose_source_id, "invalid-restart.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("invalid-restart")?)?
        .with_source_id(compose_source_id, SourceId::new("invalid-restart.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let application = result.application().ok_or("partial application expected")?;
    assert_eq!(application.services().len(), 3);
    assert!(
        application
            .services()
            .iter()
            .all(|service| service.value().restart_policy().is_none())
    );
    for service in ["unresolved", "zero", "overflow"] {
        assert!(result.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.{service}.restart_policy")
                && outcome.kind() == ConversionKind::Invalid
                && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0005")
                && !outcome.origins().is_empty()
        }));
    }
    Ok(())
}

#[test]
fn imports_service_labels_from_mapping_and_sequence_forms_with_provenance() -> Result<(), Box<dyn Error>> {
    let base_id = ComposeSourceId::new(76);
    let override_id = ComposeSourceId::new(77);
    let base = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    labels:\n",
        "      com.example.empty:\n",
        "      com.example.boolean: true\n",
        "      com.example.number: 42\n",
        "      com.example.channel: testing\n",
        "  worker:\n",
        "    image: example.invalid/worker:1\n",
        "    labels:\n",
        "      - com.example.key-only\n",
        "      - com.example.message=hello world\n",
    );
    let overlay = concat!(
        "services:\n",
        "  web:\n",
        "    labels:\n",
        "      com.example.channel: stable\n",
    );
    let project = merged_project([
        (base_id, "labels.compose.yaml", base),
        (override_id, "labels.override.yaml", overlay),
    ])?;
    let source = ComposeSource::new(project, Identifier::new("labels")?)?
        .with_source_id(base_id, SourceId::new("labels.compose.yaml")?)
        .with_source_id(override_id, SourceId::new("labels.override.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let application = result.application().ok_or("application expected")?;
    let web = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "web")
        .ok_or("web service expected")?;
    assert_eq!(web.value().labels().len(), 4);
    assert_eq!(web.value().labels()[0].value().value().expose(), "");
    assert_eq!(web.value().labels()[1].value().value().expose(), "true");
    assert_eq!(web.value().labels()[2].value().value().expose(), "42");
    assert_eq!(web.value().labels()[3].value().value().expose(), "stable");
    assert_eq!(
        web.value().labels()[3]
            .origins()
            .iter()
            .map(|origin| origin.source_id().as_str())
            .collect::<Vec<_>>(),
        [
            "labels.compose.yaml",
            "labels.override.yaml",
            "labels.compose.yaml",
            "labels.override.yaml",
        ]
    );
    let worker = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "worker")
        .ok_or("worker service expected")?;
    assert_eq!(worker.value().labels().len(), 2);
    assert_eq!(
        worker.value().labels()[0].value().name().as_str(),
        "com.example.key-only"
    );
    assert_eq!(worker.value().labels()[0].value().value().expose(), "");
    assert_eq!(worker.value().labels()[1].value().value().expose(), "hello world");
    assert!(worker.value().labels().iter().all(|label| !label.origins().is_empty()));
    Ok(())
}

#[test]
fn protects_interpolated_label_values_and_rejects_ambiguous_sensitive_compact_entries() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    labels:\n",
        "      com.example.token: ${PRIVATE_LABEL}\n",
        "  worker:\n",
        "    image: example.invalid/worker:1\n",
        "    labels:\n",
        "      - com.example.token=${PRIVATE_LABEL}\n",
    );
    let compose_source_id = ComposeSourceId::new(78);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("sensitive-label.compose.yaml", "."),
        text,
    )])?;
    let mut environment = MapEnvironment::new();
    let _ = environment.insert_sensitive("PRIVATE_LABEL", "never-print-this");
    let interpolation = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("sensitive-label")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("sensitive-label.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let application = result.application().ok_or("application expected")?;
    let web = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "web")
        .ok_or("web service expected")?;
    assert!(web.value().labels()[0].value().value().is_sensitive());
    assert_eq!(web.value().labels()[0].value().value().expose(), "never-print-this");
    let worker = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "worker")
        .ok_or("worker service expected")?;
    assert!(worker.value().labels().is_empty());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.worker.labels[0]" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(!format!("{result:?}").contains("never-print-this"));
    Ok(())
}

#[test]
fn imports_config_and_secret_definitions_and_ordered_service_grants() -> Result<(), Box<dyn Error>> {
    let text = config_and_secret_compose();
    let compose_source_id = ComposeSourceId::new(75);
    let project = merged_project([(compose_source_id, "resources.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("resources")?)?
        .with_source_id(compose_source_id, SourceId::new("resources.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let application = result.application().ok_or("application expected")?;
    assert_eq!(application.configs().len(), 2);
    assert_eq!(application.secrets().len(), 2);
    assert_eq!(
        application.configs()[0].value().ownership(),
        ResourceOwnership::Application
    );
    assert!(matches!(
        application.configs()[0]
            .value()
            .material()
            .map(boxferry_model::Sourced::value),
        Some(ConfigMaterial::Content(value)) if value.expose() == "debug=true\n"
    ));
    assert_eq!(
        application.configs()[1].value().ownership(),
        ResourceOwnership::External
    );
    assert_eq!(
        application.configs()[1]
            .value()
            .runtime_name()
            .map(|value| value.value().expose()),
        Some("runtime-config")
    );
    assert!(matches!(
        application.secrets()[0]
            .value()
            .material()
            .map(boxferry_model::Sourced::value),
        Some(SecretMaterial::Environment(value)) if value.expose() == "APP_SECRET"
    ));
    assert_eq!(
        application.secrets()[1].value().ownership(),
        ResourceOwnership::External
    );
    assert_eq!(
        application.secrets()[1]
            .value()
            .runtime_name()
            .map(|value| value.value().expose()),
        Some("runtime-secret")
    );

    let web = application.services()[0].value();
    assert_eq!(web.config_grants().len(), 2);
    assert_eq!(web.secret_grants().len(), 2);
    assert_eq!(web.config_grants()[0].value().syntax(), ResourceGrantSyntax::Short);
    assert_eq!(web.config_grants()[1].value().syntax(), ResourceGrantSyntax::Long);
    assert_eq!(
        web.config_grants()[1]
            .value()
            .target()
            .map(|value| value.value().expose()),
        Some("/etc/example.conf")
    );
    assert_eq!(
        web.secret_grants()[1]
            .value()
            .mode()
            .map(|value| value.value().expose()),
        Some("0400")
    );
    assert!(
        web.config_grants()
            .iter()
            .chain(web.secret_grants())
            .all(|grant| !grant.origins().is_empty())
    );
    Ok(())
}

#[test]
fn imports_multifile_secret_grants_and_redacts_a_sensitive_runtime_name() -> Result<(), Box<dyn Error>> {
    let base = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    secrets:\n",
        "      - source: token\n",
        "        target: primary-token\n",
        "secrets:\n",
        "  token:\n",
        "    external: true\n",
        "    name: ${PRIVATE_SECRET_NAME}\n",
    );
    let overlay = concat!(
        "services:\n",
        "  web:\n",
        "    secrets:\n",
        "      - source: token\n",
        "        target: primary-token\n",
        "        mode: \"0400\"\n",
        "      - source: token\n",
        "        target: secondary-token\n",
    );
    let base_id = ComposeSourceId::new(82);
    let overlay_id = ComposeSourceId::new(83);
    let loaded = LoadedProject::load([
        DocumentInput::new(base_id, DocumentOrigin::new("base.compose.yaml", "."), base),
        DocumentInput::new(overlay_id, DocumentOrigin::new("override.compose.yaml", "."), overlay),
    ])?;
    let mut environment = MapEnvironment::new();
    let _ = environment.insert_sensitive("PRIVATE_SECRET_NAME", "production-token");
    let interpolation = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("multifile-secret")?,
    )?
    .with_source_id(base_id, SourceId::new("base.compose.yaml")?)
    .with_source_id(overlay_id, SourceId::new("override.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let application = result.application().ok_or("application expected")?;
    let runtime_name = application.secrets()[0]
        .value()
        .runtime_name()
        .ok_or("runtime name expected")?;
    assert!(runtime_name.value().is_sensitive());
    assert_eq!(runtime_name.value().expose(), "production-token");
    assert!(!format!("{application:?}").contains("production-token"));

    let grants = application.services()[0].value().secret_grants();
    assert_eq!(grants.len(), 2);
    assert_eq!(grants[0].origins().len(), 2);
    assert_eq!(
        grants[0].value().mode().map(|value| value.value().expose()),
        Some("0400")
    );
    assert_eq!(
        grants[1].value().target().map(|value| value.value().expose()),
        Some("secondary-token")
    );
    Ok(())
}

#[test]
fn imports_effective_dependencies_with_order_and_field_provenance() -> Result<(), Box<dyn Error>> {
    let base = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    depends_on:\n",
        "      database:\n",
        "        condition: service_started\n",
        "        required: true\n",
        "  database:\n",
        "    image: example.invalid/database:1\n",
        "  cache:\n",
        "    image: example.invalid/cache:1\n",
    );
    let overlay = concat!(
        "services:\n",
        "  web:\n",
        "    depends_on:\n",
        "      database:\n",
        "        condition: service_healthy\n",
        "        restart: true\n",
        "      cache:\n",
        "        required: false\n",
    );
    let (project, selection) = processed_project(base, overlay, &ProfileRequest::new())?;
    let source = ComposeSource::new(project, Identifier::new("dependencies")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?)
        .with_source_id(
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            SourceId::new("compose.override.yaml")?,
        )
        .with_profile_selection(selection);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let dependencies = result
        .application()
        .and_then(|application| {
            application
                .services()
                .iter()
                .find(|service| service.value().name().as_str() == "web")
        })
        .map(|service| service.value().dependencies())
        .ok_or("web dependencies expected")?;

    assert_eq!(
        dependencies
            .iter()
            .map(|dependency| dependency.value().service().as_str())
            .collect::<Vec<_>>(),
        ["database", "cache"]
    );
    assert_eq!(
        dependencies[0]
            .origins()
            .iter()
            .map(|origin| origin.source_id().as_str())
            .collect::<Vec<_>>(),
        ["compose.yaml", "compose.override.yaml"]
    );
    assert!(matches!(
        dependencies[0].value().condition().map(boxferry_model::Sourced::value),
        Some(ServiceDependencyCondition::Healthy)
    ));
    assert_eq!(
        dependencies[0]
            .value()
            .condition()
            .map(|condition| condition.origins().len()),
        Some(2)
    );
    assert_eq!(
        dependencies[0].value().restart().map(boxferry_model::Sourced::value),
        Some(&true)
    );
    assert!(dependencies[0].value().is_required());
    assert!(!dependencies[1].value().is_required());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.depends_on[0].condition"
            && outcome.kind() == ConversionKind::Exact
            && outcome.origins().len() == 2
    }));
    Ok(())
}

#[test]
fn imports_short_dependencies_with_source_defaults() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    depends_on: [database, cache]\n",
        "  database:\n",
        "    image: example.invalid/database:1\n",
        "  cache:\n",
        "    image: example.invalid/cache:1\n",
    );
    let compose_source_id = ComposeSourceId::new(79);
    let project = merged_project([(compose_source_id, "dependencies.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("dependencies")?)?
        .with_source_id(compose_source_id, SourceId::new("dependencies.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let dependencies = result
        .application()
        .and_then(|application| application.services().first())
        .map(|service| service.value().dependencies())
        .ok_or("dependencies expected")?;
    assert_eq!(dependencies.len(), 2);
    assert!(dependencies.iter().all(|dependency| {
        dependency.value().condition().is_none()
            && dependency.value().restart().is_none()
            && dependency.value().required().is_none()
            && dependency.value().is_required()
            && !dependency.origins().is_empty()
    }));
    Ok(())
}

#[test]
fn imports_execution_identity_context_with_order_and_provenance() -> Result<(), Box<dyn Error>> {
    let base = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    user: \"1001:1002\"\n",
        "    userns_mode: keep-id\n",
        "    group_add: [audio, \"44\"]\n",
        "    working_dir: /srv/base\n",
        "    read_only: false\n",
    );
    let overlay = concat!(
        "services:\n",
        "  web:\n",
        "    working_dir: /srv/app\n",
        "    read_only: true\n",
    );
    let (project, selection) = processed_project(base, overlay, &ProfileRequest::new())?;
    let source = ComposeSource::new(project, Identifier::new("execution-context")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?)
        .with_source_id(
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            SourceId::new("compose.override.yaml")?,
        )
        .with_profile_selection(selection);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .map(boxferry_model::Sourced::value)
        .ok_or("service expected")?;
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(
        service.user_namespace().map(|value| value.value().expose()),
        Some("keep-id")
    );
    assert_eq!(
        service
            .supplementary_groups()
            .iter()
            .map(|value| value.value().expose())
            .collect::<Vec<_>>(),
        ["audio", "44"]
    );
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/srv/app")
    );
    assert_eq!(
        service.read_only_root_filesystem().map(boxferry_model::Sourced::value),
        Some(&true)
    );
    assert_eq!(
        service
            .working_directory()
            .map(|value| {
                value
                    .origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
    assert_eq!(service.supplementary_groups()[0].origins().len(), 1);
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "services.web.working_directory" && outcome.origins().len() == 2 })
    );
    Ok(())
}

#[test]
fn reports_unresolved_read_only_expression_without_erasing_the_service() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    read_only: ${READ_ONLY}\n",
    );
    let compose_source_id = ComposeSourceId::new(80);
    let project = merged_project([(compose_source_id, "read-only.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("read-only")?)?
        .with_source_id(compose_source_id, SourceId::new("read-only.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .map(boxferry_model::Sourced::value)
        .ok_or("service expected")?;
    assert!(service.read_only_root_filesystem().is_none());
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.read_only_root_filesystem" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFC0005" && diagnostic.severity() == Severity::Error })
    );
    Ok(())
}

#[test]
fn retains_sensitive_interpolated_identity_without_debug_disclosure() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    user: \"${PRIVATE_USER}:private-group\"\n",
    );
    let compose_source_id = ComposeSourceId::new(81);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("sensitive-user.compose.yaml", "."),
        text,
    )])?;
    let mut environment = MapEnvironment::new();
    let _ = environment.insert_sensitive("PRIVATE_USER", "private-user");
    let interpolation = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("sensitive-user")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("sensitive-user.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .map(boxferry_model::Sourced::value)
        .ok_or("service expected")?;
    assert!(service.user().is_some_and(|value| value.value().is_sensitive()));
    assert!(service.group().is_some_and(|value| value.value().is_sensitive()));
    assert_eq!(service.user().map(|value| value.value().expose()), Some("private-user"));
    assert!(!format!("{service:?}").contains("private-user"));
    assert!(!format!("{service:?}").contains("private-group"));
    Ok(())
}

#[test]
fn imports_all_typed_security_options_in_order_with_provenance_and_redaction() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    security_opt:\n",
        "      - apparmor=profile-a\n",
        "      - no-new-privileges:false\n",
        "      - seccomp=${PRIVATE_SECCOMP}\n",
        "      - label:disable\n",
        "      - label:filetype:container_file_t\n",
        "      - label:level:s0:c1,c2\n",
        "      - label:nested\n",
        "      - label:type:container_t\n",
        "      - mask=/proc/acpi:/proc/kcore\n",
        "      - unmask=/proc/acpi\n",
        "      - mask=/proc/acpi:/proc/kcore\n",
    );
    let compose_source_id = ComposeSourceId::new(106);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("security.compose.yaml", "."),
        text,
    )])?;
    let mut environment = MapEnvironment::new();
    let _ = environment.insert_sensitive("PRIVATE_SECCOMP", "/run/credentials/seccomp.json");
    let interpolation = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("security")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("security.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .map(boxferry_model::Sourced::value)
        .ok_or("service expected")?;
    let options = service.security_options().ok_or("security options expected")?;
    assert_eq!(options.len(), 11);
    assert!(matches!(options[0].value(), SecurityOption::AppArmor(profile) if profile.expose() == "profile-a"));
    assert!(matches!(options[1].value(), SecurityOption::NoNewPrivileges(false)));
    assert!(
        matches!(options[2].value(), SecurityOption::SeccompProfile(profile) if profile.is_sensitive() && profile.expose() == "/run/credentials/seccomp.json")
    );
    assert!(matches!(options[3].value(), SecurityOption::SecurityLabelDisable(true)));
    assert!(
        matches!(options[4].value(), SecurityOption::SecurityLabelFileType(value) if value.expose() == "container_file_t")
    );
    assert!(matches!(options[5].value(), SecurityOption::SecurityLabelLevel(value) if value.expose() == "s0:c1,c2"));
    assert!(matches!(options[6].value(), SecurityOption::SecurityLabelNested(true)));
    assert!(matches!(options[7].value(), SecurityOption::SecurityLabelType(value) if value.expose() == "container_t"));
    assert!(matches!(options[8].value(), SecurityOption::Mask(value) if value.expose() == "/proc/acpi:/proc/kcore"));
    assert!(matches!(options[9].value(), SecurityOption::Unmask(value) if value.expose() == "/proc/acpi"));
    assert!(matches!(options[10].value(), SecurityOption::Mask(value) if value.expose() == "/proc/acpi:/proc/kcore"));
    assert!(options.iter().all(|option| !option.origins().is_empty()));
    assert!(!service.security_options_origins().is_empty());
    assert!(!format!("{service:?}").contains("/run/credentials/seccomp.json"));
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.security_opt[10]")
    );
    Ok(())
}

#[test]
fn distinguishes_empty_security_options_from_omission() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  empty:\n",
        "    image: example.invalid/empty:1\n",
        "    security_opt: []\n",
        "  omitted:\n",
        "    image: example.invalid/omitted:1\n",
    );
    let compose_source_id = ComposeSourceId::new(107);
    let source = ComposeSource::new(
        merged_project([(compose_source_id, "security-empty.compose.yaml", text)])?,
        Identifier::new("security-empty")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("security-empty.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let application = result.application().ok_or("application expected")?;
    let empty = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "empty");
    let omitted = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "omitted");
    assert!(empty.is_some_and(|service| service.value().security_options().is_some_and(<[_]>::is_empty)));
    assert!(omitted.is_some_and(|service| service.value().security_options().is_none()));
    Ok(())
}

#[test]
fn reports_security_option_conflicts_and_untyped_values_without_dropping_typed_evidence() -> Result<(), Box<dyn Error>>
{
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    security_opt:\n",
        "      - apparmor=first\n",
        "      - apparmor=second\n",
        "      - label:disable\n",
        "      - label:type:container_t\n",
        "      - mask=/proc/acpi\n",
        "      - mask=/proc/acpi\n",
        "      - apparmor = near-miss\n",
        "      - ${DEFERRED}\n",
        "      - raw-provider-option=value\n",
        "      - \"\"\n",
    );
    let compose_source_id = ComposeSourceId::new(108);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("security-conflict.compose.yaml", "."),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("security-conflict")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("security-conflict.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    let service = result
        .application()
        .and_then(|application| application.services().first())
        .map(boxferry_model::Sourced::value)
        .ok_or("service expected")?;
    assert_eq!(service.security_options().map(<[_]>::len), Some(6));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_opt" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_opt" && outcome.kind() == ConversionKind::Unsupported
    }));
    for index in [6, 7, 9] {
        assert!(result.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.security_opt[{index}]")
                && outcome.kind() == ConversionKind::Invalid
        }));
    }
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_opt[8]" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn reports_non_string_security_option_scalars_as_invalid() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    security_opt: [7]\n",
    );
    let compose_source_id = ComposeSourceId::new(109);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("security-scalar.compose.yaml", "."),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("security-scalar")?,
    )?
    .with_source_id(compose_source_id, SourceId::new("security-scalar.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Invalid && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0005")
    }));
    Ok(())
}

#[test]
fn rejects_implicit_profile_guessing() -> Result<(), Box<dyn Error>> {
    let text = fixture_text("compose.yaml")?;
    let project = merged_project([(ComposeSourceId::new(COMPOSE_SOURCE_ID), "compose.yaml", text.as_str())])?;
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFC0002" && diagnostic.severity() == Severity::Error })
    );
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    Ok(())
}

#[test]
fn reports_port_ranges_as_policy_controlled_unsupported_intent() -> Result<(), Box<dyn Error>> {
    let text = "services:\n  web:\n    image: example.invalid/web\n    ports: [\"8000-8002:80-82\"]\n";
    let compose_source_id = ComposeSourceId::new(73);
    let project = merged_project([(compose_source_id, "ranges.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("ranges")?)?
        .with_source_id(compose_source_id, SourceId::new("ranges.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(
        result.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFC0004" && diagnostic.severity() == Severity::Warning
        })
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Unsupported && outcome.subject() == "services.web.ports[0]"
    }));
    Ok(())
}

#[test]
fn retains_start_interval_for_target_specific_loss_reporting() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web\n",
        "    healthcheck:\n",
        "      test: [\"CMD\", \"true\"]\n",
        "      start_interval: 2s\n",
    );
    let compose_source_id = ComposeSourceId::new(75);
    let project = merged_project([(compose_source_id, "health.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("health")?)?
        .with_source_id(compose_source_id, SourceId::new("health.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let healthcheck = result
        .application()
        .and_then(|application| application.services().first())
        .and_then(|service| service.value().healthcheck())
        .ok_or("health check expected")?;
    assert_eq!(
        healthcheck.value().start_interval().map(|value| value.value().as_str()),
        Some("2s")
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Exact
            && outcome.subject() == "services.web.healthcheck.start_interval"
            && !outcome.origins().is_empty()
    }));
    Ok(())
}

#[test]
fn imports_an_explicitly_disabled_image_health_check() -> Result<(), Box<dyn Error>> {
    for (source_number, setting) in [(76, "disable: true"), (78, "test: [\"NONE\"]")] {
        let text = format!("services:\n  web:\n    image: example.invalid/web\n    healthcheck:\n      {setting}\n");
        let compose_source_id = ComposeSourceId::new(source_number);
        let project = merged_project([(compose_source_id, "disabled.compose.yaml", text.as_str())])?;
        let source = ComposeSource::new(project, Identifier::new("disabled")?)?
            .with_source_id(compose_source_id, SourceId::new("disabled.compose.yaml")?);

        let result = ComposeImporter::new()?.import(&source);
        assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
        assert_eq!(
            result
                .application()
                .and_then(|application| application.services().first())
                .and_then(|service| service.value().healthcheck())
                .and_then(|healthcheck| healthcheck.value().disabled())
                .map(boxferry_model::Sourced::value),
            Some(&true)
        );
    }
    Ok(())
}

#[test]
fn reports_invalid_health_scalars_without_erasing_the_service() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web\n",
        "    healthcheck:\n",
        "      test: [\"CMD\", \"true\"]\n",
        "      retries: many\n",
    );
    let compose_source_id = ComposeSourceId::new(77);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("invalid-health.compose.yaml", "."),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    assert!(!merged.is_valid(), "invalid retry count should remain diagnostic");
    let project = merged.project().ok_or("merged project expected")?.clone();
    let source = ComposeSource::new(project, Identifier::new("invalid-health")?)?
        .with_source_id(compose_source_id, SourceId::new("invalid-health.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.application().is_some());
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFC0005" && diagnostic.severity() == Severity::Error })
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.healthcheck.retries" && outcome.kind() == ConversionKind::Invalid
    }));
    Ok(())
}

#[test]
fn retains_invalid_released_settings_without_claiming_exact_source_conversion() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n  web:\n    image: example.invalid/web\n    hostname: web.example\n    uts: host\n",
        "    cap_add: ['']\n    tmpfs: ['${TMPFS}']\n    sysctls: ['not-an-assignment']\n",
        "    ulimits:\n      nofile:\n        soft: '${SOFT}'\n        hard: 1024\n",
        "    devices:\n      - target: /dev/fuse\n    stop_signal: ''\n",
    );
    let source_id = ComposeSourceId::new(99);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("invalid-settings.compose.yaml", "."),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("invalid-settings")?,
    )?
    .with_source_id(source_id, SourceId::new("invalid-settings.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.application().is_some());
    for subject in [
        "services.web.hostname",
        "services.web.cap_add",
        "services.web.tmpfs",
        "services.web.sysctls",
        "services.web.ulimits",
        "services.web.devices",
        "services.web.stop_signal",
    ] {
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == subject && outcome.kind() != ConversionKind::Exact),
            "missing non-exact {subject}: {:#?}",
            result.outcomes()
        );
    }
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0005")
    );
    Ok(())
}

fn processed_project(
    base: &str,
    overlay: &str,
    request: &ProfileRequest,
) -> Result<(MergedProject, ProfileSelection), Box<dyn Error>> {
    let project = merged_project([
        (ComposeSourceId::new(COMPOSE_SOURCE_ID), "compose.yaml", base),
        (
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            "compose.override.yaml",
            overlay,
        ),
    ])?;
    let selection = select_profiles(&project, request);
    Ok((project, selection))
}

#[test]
fn imports_local_volume_driver_fields_labels_and_runtime_name() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(197);
    let project = merged_project([(
        id,
        "volumes.compose.yaml",
        concat!(
            "services:\n  web:\n    image: example.invalid/web:1\n",
            "volumes:\n  data:\n    name: platform-data\n    driver: local\n",
            "    driver_opts: {type: none, device: /srv/data, o: bind}\n",
            "    labels: {com.example.owner: operations}\n",
        ),
    )])?;
    let source = ComposeSource::new(project, Identifier::new("volumes")?)?
        .with_source_id(id, SourceId::new("volumes.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    let volume = result.application().ok_or("application expected")?.volumes()[0].value();
    assert_eq!(volume.name().as_str(), "data");
    assert_eq!(
        volume.runtime_name().map(|value| value.value().expose()),
        Some("platform-data")
    );
    assert_eq!(volume.driver().map(|value| value.value().expose()), Some("local"));
    assert_eq!(volume.volume_type().map(|value| value.value().expose()), Some("none"));
    assert_eq!(volume.device().map(|value| value.value().expose()), Some("/srv/data"));
    assert_eq!(volume.options().map(|value| value.value().expose()), Some("bind"));
    assert_eq!(volume.labels().map(<[_]>::len), Some(1));
    assert_eq!(
        volume.labels().map(|labels| labels[0].value().value().expose()),
        Some("operations")
    );
    assert!(
        volume
            .labels()
            .is_some_and(|labels| !labels[0].value().value().is_sensitive())
    );
    Ok(())
}

#[test]
fn reports_nonlocal_and_malformed_volume_configuration() -> Result<(), Box<dyn Error>> {
    let id = ComposeSourceId::new(198);
    let project = merged_project([(
        id,
        "invalid-volumes.compose.yaml",
        concat!(
            "services:\n  web:\n    image: example.invalid/web:1\nvolumes:\n",
            "  plugin-data:\n    driver: rexray\n    driver_opts: {type: none}\n",
            "  local-data:\n    driver: local\n    driver_opts: {device: '', unknown: value}\n",
        ),
    )])?;
    let source = ComposeSource::new(project, Identifier::new("invalid-volumes")?)?
        .with_source_id(id, SourceId::new("invalid-volumes.compose.yaml")?);
    let result = ComposeImporter::new()?.import(&source);
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.plugin-data.driver_opts" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.local-data.driver_opts[0]" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.local-data.driver_opts[1]" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

fn merged_project<'a>(
    inputs: impl IntoIterator<Item = (ComposeSourceId, &'a str, &'a str)>,
) -> Result<MergedProject, Box<dyn Error>> {
    let loaded = LoadedProject::load(inputs.into_iter().map(|(source_id, name, text)| {
        DocumentInput::new(
            source_id,
            DocumentOrigin::new(name, "fixtures/adapter-contract/compose-import-core"),
            text,
        )
    }))?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    Ok(merged.project().ok_or("merged project expected")?.clone())
}

fn fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-contract/compose-import-core")
        .join(name);
    Ok(fs::read_to_string(path)?)
}

const fn config_and_secret_compose() -> &'static str {
    concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    configs:\n",
        "      - app-config\n",
        "      - source: external-config\n",
        "        target: /etc/example.conf\n",
        "        uid: \"101\"\n",
        "        gid: \"102\"\n",
        "        mode: \"0440\"\n",
        "    secrets:\n",
        "      - app-secret\n",
        "      - source: external-secret\n",
        "        target: api-token\n",
        "        uid: \"201\"\n",
        "        gid: \"202\"\n",
        "        mode: \"0400\"\n",
        "configs:\n",
        "  app-config:\n",
        "    content: |\n",
        "      debug=true\n",
        "  external-config:\n",
        "    external: true\n",
        "    name: runtime-config\n",
        "secrets:\n",
        "  app-secret:\n",
        "    environment: APP_SECRET\n",
        "  external-secret:\n",
        "    external: true\n",
        "    name: runtime-secret\n",
    )
}

fn assert_core_healthcheck(service: &Service) -> Result<(), Box<dyn Error>> {
    let healthcheck = service.healthcheck().ok_or("health check expected")?.value();
    assert!(matches!(
        healthcheck.command().map(boxferry_model::Sourced::value),
        Some(HealthcheckCommand::Shell(value))
            if value.expose() == "curl --fail http://127.0.0.1/health || exit 1"
    ));
    assert_eq!(healthcheck.interval().map(|value| value.value().as_str()), Some("45s"));
    assert_eq!(
        healthcheck
            .interval()
            .map(|value| {
                value
                    .origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
    assert_eq!(healthcheck.timeout().map(|value| value.value().as_str()), Some("5s"));
    assert_eq!(healthcheck.retries().map(|value| value.value().as_str()), Some("3"));
    assert_eq!(
        healthcheck.start_period().map(|value| value.value().as_str()),
        Some("10s")
    );
    Ok(())
}

fn assert_core_runtime_name(service: &Service) {
    let runtime_name = service.runtime_name();
    assert_eq!(runtime_name.map(|name| name.value().expose()), Some("ferry-web"));
    assert_eq!(
        runtime_name
            .map(|name| {
                name.origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
}

fn assert_core_restart_policy(service: &Service) {
    assert!(matches!(
        service.restart_policy().map(boxferry_model::Sourced::value),
        Some(RestartPolicy::OnFailure {
            maximum_retries: Some(maximum_retries),
        }) if maximum_retries.get() == 3
    ));
    assert_eq!(
        service
            .restart_policy()
            .map(|restart| {
                restart
                    .origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
}

fn assert_core_execution_context(service: &Service) {
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(service.group().map(|value| value.value().expose()), Some("1002"));
    assert_eq!(
        service.user_namespace().map(|value| value.value().expose()),
        Some("keep-id")
    );
    assert_eq!(
        service
            .supplementary_groups()
            .iter()
            .map(|value| value.value().expose())
            .collect::<Vec<_>>(),
        ["audio", "44"]
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
