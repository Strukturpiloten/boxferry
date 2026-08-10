//! Golden Compose-to-Quadlet conversion through the public facade.

#![cfg(all(feature = "compose", feature = "quadlet"))]

use std::{error::Error, fs, path::PathBuf};

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::{
    ComposeImporter, ComposeSource, ConversionKind, Identifier, LossPolicy, PlatformVersion, QuadletExporter,
    QuadletGroupingPolicy, SourceId, TargetProfile, convert,
};

const BASE_SOURCE_ID: u32 = 91;
const OVERRIDE_SOURCE_ID: u32 = 92;
const POD_SOURCE_ID: u32 = 93;
const DEPENDENCY_SOURCE_ID: u32 = 94;
const SECRET_SOURCE_ID: u32 = 95;
const ENVIRONMENT_FILE_SOURCE_ID: u32 = 96;
const SECURITY_OPTION_SOURCE_ID: u32 = 99;

#[test]
fn public_compose_to_quadlet_route_emits_released_container_settings() -> Result<(), Box<dyn Error>> {
    let text = concat!(
        "services:\n  web:\n    image: example.invalid/web:1\n    hostname: web.example\n",
        "    pids_limit: '00042'\n    shm_size: 64m\n    cap_drop: [NET_RAW]\n",
        "    cap_add: [SYS_PTRACE]\n    tmpfs: [/run:mode=1777]\n",
        "    sysctls: { net.ipv4.ip_forward: '1' }\n",
        "    ulimits:\n      nofile:\n        soft: 1024\n        hard: 4096\n",
        "    devices: [/dev/fuse:/dev/fuse:rwm]\n    stop_signal: SIGTERM\n",
    );
    let source_id = ComposeSourceId::new(97);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("settings.compose.yaml", "settings.compose.yaml"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged project expected")?.clone();
    let source = ComposeSource::new(project, Identifier::new("settings")?)?
        .with_source_id(source_id, SourceId::new("settings.compose.yaml")?)
        .with_profile_selection(select_profiles(
            merged.project().ok_or("merged project expected")?,
            &ProfileRequest::new(),
        ));
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;
    let result = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    let text = result
        .output()
        .and_then(|output| output.file("web.container"))
        .map(boxferry::QuadletFile::text)
        .ok_or("exact public output expected")?;
    for line in [
        "HostName=web.example",
        "PidsLimit=00042",
        "ShmSize=64m",
        "DropCapability=NET_RAW",
        "AddCapability=SYS_PTRACE",
        "Tmpfs=/run:mode=1777",
        "Sysctl=net.ipv4.ip_forward=1",
        "Ulimit=nofile=1024:4096",
        "AddDevice=/dev/fuse:/dev/fuse:rwm",
        "StopSignal=SIGTERM",
    ] {
        assert!(text.contains(line), "missing {line} in {text}");
    }
    Ok(())
}

#[test]
fn public_compose_to_quadlet_route_emits_ordered_dns_keys() -> Result<(), Box<dyn Error>> {
    let source_id = ComposeSourceId::new(98);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("dns.compose.yaml", "dns.compose.yaml"),
        concat!(
            "services:\n  web:\n    image: example.invalid/web:1\n",
            "    dns: [1.1.1.1, 8.8.8.8]\n",
            "    dns_opt: [ndots:5, none]\n",
            "    dns_search: [example.test, example.internal]\n",
        ),
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged project expected")?.clone();
    let source = ComposeSource::new(project, Identifier::new("dns")?)?
        .with_source_id(source_id, SourceId::new("dns.compose.yaml")?)
        .with_profile_selection(select_profiles(
            merged.project().ok_or("merged project expected")?,
            &ProfileRequest::new(),
        ));
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;
    let result = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    let text = result
        .output()
        .and_then(|output| output.file("web.container"))
        .map(boxferry::QuadletFile::text)
        .ok_or("DNS Quadlet output expected")?;
    let expected = [
        "DNS=1.1.1.1",
        "DNS=8.8.8.8",
        "DNSOption=ndots:5",
        "DNSOption=none",
        "DNSSearch=example.test",
        "DNSSearch=example.internal",
    ];
    let mut previous = 0;
    for line in expected {
        let index = text[previous..]
            .find(line)
            .ok_or_else(|| format!("missing {line} in {text}"))?
            + previous;
        assert!(index >= previous);
        previous = index + line.len();
    }
    Ok(())
}

#[test]
fn public_compose_to_quadlet_route_emits_all_released_security_keys_in_order() -> Result<(), Box<dyn Error>> {
    let source = security_option_source()?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 8, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let result = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    let output = result.output().ok_or("partial security-option output expected")?;
    assert!(!format!("{output:?}").contains("secret-seccomp.json"));
    assert_eq!(
        security_option_lines(output.file("web.container").map(boxferry::QuadletFile::text)),
        [
            "AppArmor=profile-a",
            "NoNewPrivileges=false",
            "SeccompProfile=secret-seccomp.json",
            "SecurityLabelDisable=true",
            "SecurityLabelFileType=container_file_t",
            "SecurityLabelLevel=s0:c1,c2",
            "SecurityLabelNested=true",
            "SecurityLabelType=container_t",
            "Mask=/proc/acpi",
            "Unmask=/proc/acpi",
            "Mask=/proc/acpi",
        ]
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_options" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn public_compose_to_quadlet_route_requires_apparmor_floor_without_losing_other_candidates()
-> Result<(), Box<dyn Error>> {
    let source = security_option_source()?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 7, 1),
        Some(PlatformVersion::new(5, 8, 0)),
    )?;

    let strict = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(strict.is_blocked());
    assert!(strict.output().is_none());
    assert!(strict.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.security_options[0]" && outcome.kind() == ConversionKind::Unsupported
    }));

    let partial = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let output = partial.output().ok_or("partial security-option output expected")?;
    assert_eq!(
        security_option_lines(output.file("web.container").map(boxferry::QuadletFile::text)),
        [
            "NoNewPrivileges=false",
            "SeccompProfile=secret-seccomp.json",
            "SecurityLabelDisable=true",
            "SecurityLabelFileType=container_file_t",
            "SecurityLabelLevel=s0:c1,c2",
            "SecurityLabelNested=true",
            "SecurityLabelType=container_t",
            "Mask=/proc/acpi",
            "Unmask=/proc/acpi",
            "Mask=/proc/acpi",
        ]
    );
    assert!(
        partial
            .outcomes()
            .iter()
            .filter(|outcome| {
                outcome.subject().starts_with("services.web.security_options[")
                    && outcome.subject() != "services.web.security_options[0]"
            })
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(())
}

#[test]
fn converts_the_golden_project_with_explicit_partial_authorization() -> Result<(), Box<dyn Error>> {
    let directory = fixture_directory();
    let base = fixture_text("compose.yaml")?;
    let override_text = fixture_text("compose.override.yaml")?;
    let base_id = ComposeSourceId::new(BASE_SOURCE_ID);
    let override_id = ComposeSourceId::new(OVERRIDE_SOURCE_ID);
    let loaded = LoadedProject::load([
        DocumentInput::new(
            base_id,
            DocumentOrigin::new("compose.yaml", directory.display().to_string()),
            base,
        ),
        DocumentInput::new(
            override_id,
            DocumentOrigin::new("compose.override.yaml", directory.display().to_string()),
            override_text,
        ),
    ])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let project = merged.project().ok_or("merged project expected")?.clone();
    let selection = select_profiles(&project, &ProfileRequest::new());
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(base_id, SourceId::new("compose.yaml")?)
        .with_source_id(override_id, SourceId::new("compose.override.yaml")?)
        .with_profile_selection(selection);
    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let strict = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(strict.is_blocked());
    assert!(strict.output().is_none());

    let partial = convert(&importer, &source, &exporter, &target, LossPolicy::AllowPartial)?;
    let output = partial.output().ok_or("partial output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["frontend.network", "data.volume", "web.container"]
    );
    for name in ["frontend.network", "data.volume", "web.container"] {
        assert_eq!(
            output.file(name).map(boxferry::QuadletFile::text),
            Some(fixture_text(name)?.as_str()),
            "{name} differs from its reviewed golden output"
        );
    }
    assert!(output.file("worker.container").is_none());
    assert!(output.document_set().is_valid());
    assert_eq!(output.document_set().graph().edges().len(), 2);

    let restart_outcome = partial
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.web.restart_policy")
        .ok_or("restart-policy outcome expected")?;
    assert!(restart_outcome.kind() == ConversionKind::Exact && !restart_outcome.origins().is_empty());

    let host_mapping_outcomes = partial
        .outcomes()
        .iter()
        .filter(|outcome| outcome.subject().contains("extra_hosts") || outcome.subject().contains("host_mappings"))
        .collect::<Vec<_>>();
    assert_eq!(host_mapping_outcomes.len(), 6);
    assert!(
        host_mapping_outcomes
            .iter()
            .all(|outcome| { outcome.kind() == ConversionKind::Exact && !outcome.origins().is_empty() })
    );

    let diagnostics = partial
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0003")
        .map(|diagnostic| {
            let subject = diagnostic
                .fields()
                .iter()
                .find(|field| field.name() == "subject")
                .map_or("missing-subject", |field| field.value().expose());
            format!("{} {subject}", diagnostic.code().as_str())
        })
        .collect::<Vec<_>>();
    let expected = fixture_text("expected-diagnostics.txt")?
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics, expected);

    let unsupported = partial
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .collect::<Vec<_>>();
    assert_eq!(unsupported.len(), 6);
    assert!(unsupported.iter().all(|outcome| !outcome.origins().is_empty()));
    Ok(())
}

#[test]
fn converts_a_compatible_project_into_an_explicitly_authorized_pod() -> Result<(), Box<dyn Error>> {
    let directory = pod_fixture_directory();
    let compose = pod_fixture_text("compose.yaml")?;
    let compose_id = ComposeSourceId::new(POD_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_id,
        DocumentOrigin::new("compose.yaml", directory.display().to_string()),
        compose,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let project = merged.project().ok_or("merged project expected")?.clone();
    let selection = select_profiles(&project, &ProfileRequest::new());
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(compose_id, SourceId::new("compose.yaml")?)
        .with_profile_selection(selection);
    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?.with_grouping_policy(QuadletGroupingPolicy::SinglePod);
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let strict = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(strict.is_blocked());
    let approximate = convert(&importer, &source, &exporter, &target, LossPolicy::AllowApproximate)?;
    let output = approximate.output().ok_or("approximate pod output expected")?;
    for name in ["frontend.network", "ferry-pod.pod", "web.container", "worker.container"] {
        assert_eq!(
            output.file(name).map(boxferry::QuadletFile::text),
            Some(pod_fixture_text(name)?.as_str()),
            "{name} differs from its reviewed golden output"
        );
    }
    assert_eq!(output.document_set().graph().edges().len(), 3);
    let pod_host_mapping = approximate
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.pod.host_mappings[0]")
        .ok_or("pod host-mapping outcome expected")?;
    assert_eq!(pod_host_mapping.kind(), ConversionKind::Exact);
    assert_eq!(pod_host_mapping.origins().len(), 2);
    let pod_user_namespace = approximate
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.pod.user_namespace")
        .ok_or("pod user-namespace outcome expected")?;
    assert_eq!(pod_user_namespace.kind(), ConversionKind::Exact);
    assert_eq!(pod_user_namespace.origins().len(), 2);
    let grouping = approximate
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.grouping")
        .ok_or("grouping outcome expected")?;
    assert_eq!(grouping.kind(), ConversionKind::Approximate);
    assert!(!grouping.origins().is_empty());
    assert_eq!(
        approximate
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0007")
            .count(),
        1
    );
    let dependency = approximate
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.worker.dependencies[0]")
        .ok_or("pod-grouped dependency outcome expected")?;
    assert_eq!(dependency.kind(), ConversionKind::Exact);
    assert!(!dependency.origins().is_empty());
    Ok(())
}

#[test]
fn converts_required_optional_and_healthy_dependencies_exactly() -> Result<(), Box<dyn Error>> {
    let directory = dependency_fixture_directory();
    let compose = dependency_fixture_text("compose.yaml")?;
    let compose_id = ComposeSourceId::new(DEPENDENCY_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_id,
        DocumentOrigin::new("compose.yaml", directory.display().to_string()),
        compose,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("fallback")?,
    )?
    .with_source_id(compose_id, SourceId::new("compose.yaml")?);
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let result = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    let output = result.output().ok_or("exact dependency output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["database.container", "cache.container", "web.container"]
    );
    for name in ["database.container", "cache.container", "web.container"] {
        assert_eq!(
            output.file(name).map(boxferry::QuadletFile::text),
            Some(dependency_fixture_text(name)?.as_str()),
            "{name} differs from its reviewed dependency output"
        );
    }
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.database.readiness"
            && outcome.kind() == ConversionKind::Exact
            && !outcome.origins().is_empty()
    }));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.dependencies[1]"
            && outcome.kind() == ConversionKind::Exact
            && outcome.origins().len() >= 2
    }));
    Ok(())
}

#[test]
fn converts_external_secrets_and_reports_materialization_actions() -> Result<(), Box<dyn Error>> {
    let directory = secret_fixture_directory();
    let compose = secret_fixture_text("compose.yaml")?;
    let compose_id = ComposeSourceId::new(SECRET_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_id,
        DocumentOrigin::new("compose.yaml", directory.display().to_string()),
        compose,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("fallback")?,
    )?
    .with_source_id(compose_id, SourceId::new("compose.yaml")?);
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let strict = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(strict.is_blocked());

    let partial = convert(
        &ComposeImporter::new()?,
        &source,
        &QuadletExporter::new()?,
        &target,
        LossPolicy::AllowPartial,
    )?;
    let output = partial.output().ok_or("partial secret output expected")?;
    assert_eq!(output.files().len(), 1);
    assert_eq!(
        output.file("web.container").map(boxferry::QuadletFile::text),
        Some(secret_fixture_text("web.container")?.as_str())
    );

    let diagnostics = partial
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0003")
        .map(|diagnostic| {
            let subject = diagnostic
                .fields()
                .iter()
                .find(|field| field.name() == "subject")
                .map_or("missing-subject", |field| field.value().expose());
            format!("{} {subject}", diagnostic.code().as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        diagnostics,
        secret_fixture_text("expected-diagnostics.txt")?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
    let unsupported = partial
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .collect::<Vec<_>>();
    assert_eq!(unsupported.len(), 4);
    assert!(unsupported.iter().all(|outcome| !outcome.origins().is_empty()));
    for subject in [
        "services.web.secrets[0]",
        "services.web.secrets[1]",
        "services.web.secrets[2]",
    ] {
        assert!(
            partial
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Exact })
        );
    }
    Ok(())
}

#[test]
fn converts_compose_environment_files_without_implicitly_reading_them() -> Result<(), Box<dyn Error>> {
    let compose = concat!(
        "services:\n",
        "  web:\n",
        "    image: example.invalid/web:1\n",
        "    environment:\n",
        "      APP_ENV: production\n",
        "    env_file:\n",
        "      - ./base.env\n",
        "      - path: config/raw.env\n",
        "        required: true\n",
        "        format: raw\n",
    );
    let compose_id = ComposeSourceId::new(ENVIRONMENT_FILE_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_id,
        DocumentOrigin::new("compose.yaml", "/srv/project"),
        compose,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let source = ComposeSource::new(
        merged.project().ok_or("merged project expected")?.clone(),
        Identifier::new("environment-files")?,
    )?
    .with_source_id(compose_id, SourceId::new("compose.yaml")?);
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;
    let exporter = QuadletExporter::new()?.with_relative_host_path_root("/srv/project")?;

    let strict = convert(
        &ComposeImporter::new()?,
        &source,
        &exporter,
        &target,
        LossPolicy::ExactOnly,
    )?;
    assert!(strict.is_blocked());
    let approximate = convert(
        &ComposeImporter::new()?,
        &source,
        &exporter,
        &target,
        LossPolicy::AllowApproximate,
    )?;
    assert_eq!(
        approximate
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry::QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Environment=APP_ENV=production\n",
            "EnvironmentFile=/srv/project/base.env\n",
            "EnvironmentFile=/srv/project/config/raw.env\n",
        ))
    );
    assert_eq!(
        approximate
            .outcomes()
            .iter()
            .filter(|outcome| {
                outcome.kind() == ConversionKind::Approximate
                    && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFQ0010")
            })
            .count(),
        2
    );
    Ok(())
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-core")
}

fn pod_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-pod")
}

fn dependency_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-dependencies")
}

fn secret_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-secrets")
}

fn fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(fixture_directory().join(name))?)
}

fn pod_fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(pod_fixture_directory().join(name))?)
}

fn dependency_fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(dependency_fixture_directory().join(name))?)
}

fn secret_fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(secret_fixture_directory().join(name))?)
}

fn security_option_source() -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(SECURITY_OPTION_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("security.compose.yaml", "security.compose.yaml"),
        concat!(
            "services:\n  web:\n    image: example.invalid/web:1\n    security_opt:\n",
            "      - apparmor=profile-a\n      - no-new-privileges:false\n",
            "      - seccomp=secret-seccomp.json\n      - label:disable\n",
            "      - label:filetype:container_file_t\n      - label:level:s0:c1,c2\n",
            "      - label:nested\n      - label:type:container_t\n",
            "      - mask=/proc/acpi\n      - unmask=/proc/acpi\n      - mask=/proc/acpi\n",
        ),
    )])?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged project expected")?.clone();
    Ok(ComposeSource::new(project, Identifier::new("security")?)?
        .with_source_id(source_id, SourceId::new("security.compose.yaml")?)
        .with_profile_selection(select_profiles(
            merged.project().ok_or("merged project expected")?,
            &ProfileRequest::new(),
        )))
}

fn security_option_lines(text: Option<&str>) -> Vec<&str> {
    text.unwrap_or_default()
        .lines()
        .filter(|line| {
            [
                "AppArmor=",
                "NoNewPrivileges=",
                "SeccompProfile=",
                "SecurityLabel",
                "Mask=",
                "Unmask=",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        })
        .collect()
}
