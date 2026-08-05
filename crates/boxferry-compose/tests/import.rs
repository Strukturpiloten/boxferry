//! Public adapter behavior backed by the repository fixture contract.

use std::{error::Error, fs, path::PathBuf};

use boxferry_compose::{ComposeImporter, ComposeSource};
use boxferry_engine::{ConversionKind, ImportAdapter, Severity};
use boxferry_model::{
    Command, ConfigMaterial, EnvironmentValue, HealthcheckCommand, HostAddressKind, Identifier, MountSource, Protocol,
    ResourceGrantSyntax, ResourceOwnership, SecretMaterial, SelinuxRelabel, Service, ServiceDependencyCondition,
    SourceId,
};
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
    assert_eq!(
        web.image().map(|image| image.value().as_str()),
        Some("registry.example:5000/team/web:1.3@sha256:fedcba")
    );
    assert!(matches!(
        web.command().map(boxferry_model::Sourced::value),
        Some(Command::Exec(values))
            if values
                .iter()
                .map(boxferry_model::ProtectedString::expose)
                .eq(["php", "-v"])
    ));
    assert_core_execution_context(web);
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
