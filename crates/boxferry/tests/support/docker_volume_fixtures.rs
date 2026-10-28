//! Shared loader/importer mechanics for offline volume-only consumer fixtures.
//!
//! These mechanics carry no independent acceptance expectations and never execute native output.

use std::error::Error;

use boxferry::compose::compose_lens::{
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};
use boxferry::docker::docker_lens::version::{
    DaemonMode, EngineBuild, TargetCapabilityCatalog, TargetProfile as NativeTargetProfile,
};
use boxferry::{
    Application, ComposeImporter, ComposeSource, ConversionKind, DockerExporter, ExportAdapter, Identifier,
    ImportAdapter, LossPolicy, PlatformVersion, SourceId, TargetProfile,
};
use sha2::{Digest, Sha256};

pub type FixtureResult<T> = Result<T, Box<dyn Error>>;

pub struct Fixture {
    pub id: &'static str,
    pub source_path: &'static str,
    pub template: &'static str,
    pub volume_count: usize,
}

pub const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "forgejo",
        source_path: "fixtures/conformance/forgejo-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/forgejo-application/compose.yaml"),
        volume_count: 2,
    },
    Fixture {
        id: "nextcloud",
        source_path: "fixtures/conformance/nextcloud-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/nextcloud-application/compose.yaml"),
        volume_count: 3,
    },
    Fixture {
        id: "paperless-ngx",
        source_path: "fixtures/conformance/paperless-ngx-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/paperless-ngx-application/compose.yaml"),
        volume_count: 6,
    },
    Fixture {
        id: "immich",
        source_path: "fixtures/conformance/immich-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/immich-application/compose.yaml"),
        volume_count: 4,
    },
    Fixture {
        id: "observability",
        source_path: "fixtures/conformance/observability-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/observability-application/compose.yaml"),
        volume_count: 5,
    },
    Fixture {
        id: "supabase",
        source_path: "fixtures/conformance/supabase-application/compose.yaml",
        template: include_str!("../../../../fixtures/conformance/supabase-application/compose.yaml"),
        volume_count: 3,
    },
];

pub fn fixture(id: &str) -> FixtureResult<&'static Fixture> {
    FIXTURES
        .iter()
        .find(|fixture| fixture.id == id)
        .ok_or_else(|| "unknown volume fixture".into())
}

pub fn synthetic_environment(prefix: &str, run: Option<&str>) -> MapEnvironment {
    // Public synthetic inputs only; no ambient environment, implicit .env, or source rewriting.
    let mut environment = MapEnvironment::new();
    let _ = environment.insert("BF_PREFIX", prefix);
    if let Some(run) = run {
        let _ = environment.insert("BF_RUN", run);
    }
    let _ = environment.insert("BF_FIXTURE_ROOT", "/volume-label-consumer-offline");
    for name in [
        "BF_POSTGRES_IMAGE",
        "BF_FORGEJO_IMAGE",
        "BF_NEXTCLOUD_IMAGE",
        "BF_NGINX_IMAGE",
        "BF_REDIS_IMAGE",
        "BF_GOTENBERG_IMAGE",
        "BF_PAPERLESS_IMAGE",
        "BF_TIKA_IMAGE",
        "BF_VALKEY_IMAGE",
        "BF_MACHINE_LEARNING_IMAGE",
        "BF_SERVER_IMAGE",
        "BF_ALLOY_IMAGE",
        "BF_GRAFANA_IMAGE",
        "BF_LOKI_IMAGE",
        "BF_PRODUCER_IMAGE",
        "BF_PROMETHEUS_IMAGE",
        "BF_AUTH_IMAGE",
        "BF_DB_IMAGE",
        "BF_FUNCTIONS_IMAGE",
        "BF_IMGPROXY_IMAGE",
        "BF_KONG_IMAGE",
        "BF_META_IMAGE",
        "BF_REALTIME_IMAGE",
        "BF_REST_IMAGE",
        "BF_STORAGE_IMAGE",
        "BF_STUDIO_IMAGE",
        "BF_SUPAVISOR_IMAGE",
    ] {
        let _ = environment.insert(name, "registry.invalid/volume-label-consumer/image:offline");
    }
    for name in [
        "BF_DB_PASSWORD",
        "BF_FORGEJO_SECRET_KEY",
        "BF_ADMIN_PASSWORD",
        "BF_REDIS_PASSWORD",
        "BF_SECRET_KEY",
        "BF_ANON_KEY",
        "BF_JWT_SECRET",
        "BF_POOLER_SECRET",
        "BF_REALTIME_DB_KEY",
        "BF_REALTIME_SECRET",
        "BF_SERVICE_KEY",
    ] {
        let _ = environment.insert(name, "synthetic-offline-not-a-credential");
    }
    environment
}

pub fn loaded_fixture(fixture: &Fixture) -> FixtureResult<(LoadedProject, usize)> {
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeSourceId::new(1),
        DocumentOrigin::new(fixture.source_path, "/volume-label-consumer-offline"),
        fixture.template,
    )])?;
    Ok((loaded, fixture.template.len()))
}

pub fn import_fixture(fixture: &Fixture, prefix: &str, run: &str) -> FixtureResult<(Application, usize)> {
    let (loaded, source_length) = loaded_fixture(fixture)?;
    let interpolated = loaded.interpolate(&synthetic_environment(prefix, Some(run)));
    if !interpolated.is_valid() {
        return Err("explicit synthetic fixture interpolation failed".into());
    }
    let merged = merge_project(&loaded, Some(&interpolated));
    if !merged.is_valid() {
        return Err("canonical fixture merge failed".into());
    }
    let project = merged.project().ok_or("merged fixture project missing")?.clone();
    let source = ComposeSource::new(project, Identifier::new(fixture.id)?)?
        .with_source_id(ComposeSourceId::new(1), SourceId::new(fixture.source_path)?);
    let imported = ComposeImporter::new()?.import(&source);
    let application = imported
        .application()
        .ok_or("fixture neutral application missing")?
        .clone();
    Ok((application, source_length))
}

pub fn volume_only_projection(application: &Application, id: &str) -> FixtureResult<Application> {
    let mut volume_only = Application::new(Identifier::new(format!("volume-only-{id}"))?);
    for volume in application.volumes() {
        volume_only.add_volume(volume.clone())?;
    }
    Ok(volume_only)
}

pub fn target(profile: &NativeTargetProfile) -> FixtureResult<TargetProfile> {
    let version = match profile.identity().build() {
        EngineBuild::Upstream => PlatformVersion::new(29, 8, 1),
        EngineBuild::DebianPackage(_) => PlatformVersion::new(20, 10, 5),
    };
    Ok(TargetProfile::new("docker", version, Some(version))?)
}

pub fn selected_profile(lane: &str) -> FixtureResult<NativeTargetProfile> {
    let (upstream, mode) = match lane {
        "debian11-rootful" => (false, DaemonMode::Rootful),
        "debian11-rootless" => (false, DaemonMode::Rootless),
        "upstream-rootful" => (true, DaemonMode::Rootful),
        "upstream-rootless" => (true, DaemonMode::Rootless),
        _ => return Err("unknown exact volume rehearsal lane".into()),
    };
    let catalogue = TargetCapabilityCatalog::reviewed();
    let mut matching = catalogue.profiles().filter(|profile| {
        matches!(profile.identity().build(), EngineBuild::Upstream) == upstream && profile.mode() == mode
    });
    let profile = matching
        .next()
        .ok_or("reviewed volume rehearsal profile missing")?
        .clone();
    if matching.next().is_some() {
        return Err("ambiguous volume rehearsal profile".into());
    }
    Ok(profile)
}

pub fn sha256(bytes: &[u8]) -> String {
    let hex = b"0123456789abcdef";
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| {
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

pub fn render_fixture(
    fixture: &Fixture,
    prefix: &str,
    run: &str,
    profile: &NativeTargetProfile,
) -> FixtureResult<Vec<u8>> {
    let (application, _) = import_fixture(fixture, prefix, run)?;
    let volume_only = volume_only_projection(&application, fixture.id)?;
    if volume_only.volumes().len() != fixture.volume_count {
        return Err("authored fixture volume inventory differs".into());
    }
    let planned = DockerExporter::new(profile.clone())?.plan(&volume_only, &target(profile)?)?;
    if !planned
        .outcomes()
        .iter()
        .all(|outcome| outcome.kind() == ConversionKind::Exact)
    {
        return Err("volume rehearsal conversion is not exact".into());
    }
    let output = planned.candidate().ok_or("labelled-volume native candidate missing")?;
    if !output.volume_prerequisites().is_empty()
        || !output.network_prerequisites().is_empty()
        || !output.dependency_decisions().is_empty()
        || output.native_profile() != profile
    {
        return Err("volume-only native candidate context or prerequisites differ".into());
    }
    let raw = output
        .complete_bytes()
        .map_err(|_| "complete volume artifact unavailable")?;
    if raw.is_empty() || raw.len() > 1_048_576 || planned.authorize(LossPolicy::ExactOnly).is_blocked() {
        return Err("volume rehearsal artifact is unavailable or exceeds its bound".into());
    }
    Ok(raw)
}
