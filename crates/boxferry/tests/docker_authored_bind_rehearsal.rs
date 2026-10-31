//! Authored bind intent through the production Compose importer, neutral model and Docker exporter.
//!
//! The complete Nextcloud graph reaches every reviewed profile without fallback. Supabase is
//! explicitly BIND/MOUNT-ONLY after full production import: every service retains its image,
//! original runtime identity, all ordered mounts and their provenance, plus volume and network
//! definitions. Networks are declaration-only: aliases/endpoints/attachments are not projected.
//! Service labels, environment, commands, health, ports and dependencies are not projected.
//! Full Supabase retains public aliases but remains a protected-health-command negative control.
//! Nextcloud uses the real scenario overlay and explicit public example environment file; Supabase
//! uses its canonical conformance document and caller-classified public synthetic interpolation.
//! Generated complete bytes and Nextcloud typed dependency decisions are inspected independently. This
//! does not supply the private CLI dependency-sidecar serialization or establish CLI readiness.
//! Full Supabase application/Python topology qualification is pending separately; projected output
//! cannot satisfy its full application admission record. No Supabase dependency claim is made.
//! Additional external-network controls independently author None/false/true constraints beside
//! both retained fixture bind inventories; they do not change canonical application topology.
//! Nextcloud's original networks remain application-owned; Supabase's canonical external edge
//! remains unconstrained. These controls are generated offline evidence, not runtime isolation.
//! Request inventories are closed: Nextcloud has 13 creates and exactly one edge/frontend
//! secondary connect; projected Supabase has 15 creates and no connect. External controls add
//! prerequisites only, never requests. Handwritten mutation controls are not native acceptance.
//! All artifacts are inert: no daemon, host bind-path inspection, `SELinux` enforcement, deployment,
//! readiness execution, or full application qualification is claimed.

#![cfg(all(feature = "compose", feature = "docker"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    path::PathBuf,
};

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
    Application, ComposeImporter, ComposeSource, ConversionKind, DockerEnvironmentPolicy, DockerExporter, DockerOutput,
    HealthcheckCommand, Identifier, ImportAdapter, ImportResult, LossPolicy, MountSource, Network, PlatformVersion,
    ProtectedString, Provenance, ProvenanceKind, ResourceOwnership, SelinuxRelabel, Service,
    ServiceDependencyCondition, SourceId, Sourced, TargetProfile, convert_imported,
};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PREFIX: &str = "bind-rehearsal";
const RUN: &str = "authored-bind-offline";
const ROOT: &str = "/authored-bind-rehearsal-offline";
const SECRET: &str = "bind-rehearsal-protected-canary-not-a-credential";
const NEXTCLOUD_BASE: &str = include_str!("../../../fixtures/scenarios/nextcloud-application/compose.yaml");
const NEXTCLOUD_OVERLAY: &str =
    include_str!("../../../fixtures/scenarios/nextcloud-application/compose.acceptance.yaml");
const NEXTCLOUD_ENV: &str = include_str!("../../../fixtures/scenarios/nextcloud-application/nextcloud.env.example");
const SUPABASE_PATH: &str = "fixtures/conformance/supabase-application/compose.yaml";
const SUPABASE: &str = include_str!("../../../fixtures/conformance/supabase-application/compose.yaml");
const CONTROL_NETWORK: &str = "authored-external-network-constraint-control";

#[derive(Clone, Copy)]
struct ExternalNetworkControl {
    internal: Option<bool>,
}

impl ExternalNetworkControl {
    fn add_to(self, application: &mut Application) -> TestResult {
        // Independently authored neutral control, not an override of either canonical Compose
        // topology. Preserve every imported resource and bind, including original provenance.
        let origin = Provenance::source(SourceId::new(CONTROL_NETWORK)?);
        let mut network = Network::new(Identifier::new(CONTROL_NETWORK)?, ResourceOwnership::External);
        network.set_driver(Sourced::from_source(ProtectedString::plain("bridge"), origin.clone()));
        if let Some(internal) = self.internal {
            network.set_internal(Sourced::from_source(internal, origin.clone()));
        }
        application.add_network(Sourced::from_source(network, origin))?;
        Ok(())
    }
}

const IMAGE_ROLES: &[(&str, &str)] = &[
    ("auth", "BF_AUTH_IMAGE"),
    ("db", "BF_DB_IMAGE"),
    ("functions", "BF_FUNCTIONS_IMAGE"),
    ("imgproxy", "BF_IMGPROXY_IMAGE"),
    ("kong", "BF_KONG_IMAGE"),
    ("meta", "BF_META_IMAGE"),
    ("realtime", "BF_REALTIME_IMAGE"),
    ("rest", "BF_REST_IMAGE"),
    ("storage", "BF_STORAGE_IMAGE"),
    ("studio", "BF_STUDIO_IMAGE"),
    ("supavisor", "BF_SUPAVISOR_IMAGE"),
];

#[derive(Clone, Copy)]
enum Fixture {
    Nextcloud,
    Supabase,
}

struct BindExpectation {
    service: &'static str,
    identity: String,
    index: usize,
    source: String,
    target: &'static str,
    relabel: SelinuxRelabel,
    source_id: &'static str,
}

impl Fixture {
    fn service_names(self) -> &'static [&'static str] {
        match self {
            Self::Nextcloud => &[
                "db",
                "cache",
                "init",
                "app",
                "frontend",
                "cron",
                "edge-proxy",
                "second-app",
            ],
            Self::Supabase => &[
                "db",
                "auth",
                "rest",
                "realtime",
                "imgproxy",
                "storage",
                "meta",
                "supavisor",
                "functions",
                "studio",
                "kong",
            ],
        }
    }

    fn identity(self, service: &str) -> String {
        match self {
            Self::Nextcloud => service.to_owned(),
            Self::Supabase => format!("{PREFIX}-supabase-{service}"),
        }
    }

    fn documents(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Self::Nextcloud => vec![
                ("compose.yaml", NEXTCLOUD_BASE),
                ("compose.acceptance.yaml", NEXTCLOUD_OVERLAY),
            ],
            Self::Supabase => vec![(SUPABASE_PATH, SUPABASE)],
        }
    }

    fn binds(self) -> Vec<BindExpectation> {
        // Independent authored expectations, including original positions among named volumes.
        let rows = match self {
            Self::Nextcloud => vec![
                (
                    "edge-proxy",
                    0,
                    "/srv/boxferry-fixture/nextcloud-application/edge-proxy.conf".to_owned(),
                    "/etc/nginx/conf.d/edge.conf",
                ),
                (
                    "frontend",
                    1,
                    "/srv/boxferry-fixture/nextcloud-application/frontend.conf".to_owned(),
                    "/etc/nginx/conf.d/nextcloud.conf",
                ),
                (
                    "init",
                    1,
                    "/srv/boxferry-fixture/nextcloud-application/nextcloud.config.php".to_owned(),
                    "/opt/boxferry/config.php",
                ),
            ],
            Self::Supabase => vec![
                (
                    "db",
                    1,
                    format!("{ROOT}/db-init.sql"),
                    "/docker-entrypoint-initdb.d/zzzzzzzzzzzz-boxferry.sql",
                ),
                ("functions", 0, format!("{ROOT}/functions"), "/home/deno/functions"),
                ("studio", 0, ROOT.to_owned(), "/boxferry-fixture"),
                ("kong", 0, format!("{ROOT}/kong.yml"), "/etc/kong/kong.yml"),
            ],
        };
        rows.into_iter()
            .map(|(service, index, source, target)| BindExpectation {
                service,
                identity: self.identity(service),
                index,
                source,
                target,
                relabel: match self {
                    Self::Nextcloud => SelinuxRelabel::Private,
                    Self::Supabase => SelinuxRelabel::Shared,
                },
                source_id: match self {
                    Self::Nextcloud => "compose.acceptance.yaml",
                    Self::Supabase => SUPABASE_PATH,
                },
            })
            .collect()
    }

    fn edges(self) -> Vec<(&'static str, &'static str, ServiceDependencyCondition)> {
        use ServiceDependencyCondition::Started;
        match self {
            Self::Nextcloud => vec![
                ("init", "db", Started),
                ("init", "cache", Started),
                ("app", "init", Started),
                ("frontend", "app", Started),
                ("cron", "app", Started),
                ("edge-proxy", "frontend", Started),
                ("edge-proxy", "second-app", Started),
            ],
            // The explicit projection intentionally has no dependency choreography.
            Self::Supabase => vec![],
        }
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn require(condition: bool, message: &'static str) -> TestResult {
    if condition { Ok(()) } else { Err(message.into()) }
}

fn import_fixture(fixture: Fixture) -> TestResult<ImportResult> {
    let documents = fixture.documents();
    let origin = match fixture {
        Fixture::Nextcloud => repository_root().join("fixtures/scenarios/nextcloud-application"),
        Fixture::Supabase => PathBuf::from(ROOT),
    };
    let inputs = documents
        .iter()
        .enumerate()
        .map(|(index, (path, text))| {
            Ok(DocumentInput::new(
                ComposeSourceId::new(u32::try_from(index + 1)?),
                DocumentOrigin::new(*path, &origin),
                *text,
            ))
        })
        .collect::<TestResult<Vec<_>>>()?;
    let loaded = LoadedProject::load(inputs)?;
    let mut environment = MapEnvironment::new();
    match fixture {
        Fixture::Nextcloud => {
            for line in NEXTCLOUD_ENV
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                let (name, value) = line.split_once('=').ok_or("example environment assignment malformed")?;
                let _ = environment.insert_sensitive(name, value);
            }
        }
        Fixture::Supabase => {
            // Explicit embedded authorization: names, labels, aliases and paths are public synthetic
            // inputs. Secret-valued interpolation remains protected, unlike public identities.
            for (name, value) in [("BF_PREFIX", PREFIX), ("BF_RUN", RUN), ("BF_FIXTURE_ROOT", ROOT)] {
                let _ = environment.insert(name, value);
            }
            for (role, name) in IMAGE_ROLES {
                let _ = environment.insert(*name, format!("registry.invalid/authored-bind/{role}:offline"));
            }
            for name in [
                "BF_DB_PASSWORD",
                "BF_ANON_KEY",
                "BF_JWT_SECRET",
                "BF_POOLER_SECRET",
                "BF_REALTIME_DB_KEY",
                "BF_REALTIME_SECRET",
                "BF_SERVICE_KEY",
            ] {
                let _ = environment.insert_sensitive(name, SECRET);
            }
        }
    }
    let interpolated = loaded.interpolate(&environment);
    require(
        interpolated.is_valid(),
        "explicit authored fixture interpolation failed",
    )?;
    let merged = merge_project(&loaded, Some(&interpolated));
    require(merged.is_valid(), "authored fixture merge failed")?;
    let mut source = ComposeSource::new(
        merged.project().ok_or("authored project missing")?.clone(),
        Identifier::new(match fixture {
            Fixture::Nextcloud => "nextcloud-application",
            Fixture::Supabase => "supabase",
        })?,
    )?;
    for (index, (path, _)) in documents.iter().enumerate() {
        source = source.with_source_id(ComposeSourceId::new(u32::try_from(index + 1)?), SourceId::new(*path)?);
    }
    Ok(ComposeImporter::new()?.import(&source))
}

fn assert_origins(origins: &[Provenance], fixture: Fixture, source_id: Option<&str>) -> TestResult {
    require(!origins.is_empty(), "authored field provenance missing")?;
    let documents = fixture.documents();
    // A merge may retain corroborating base-document origins as well as its selected overlay.
    if let Some(expected) = source_id {
        require(
            origins.iter().any(|origin| origin.source_id().as_str() == expected),
            "authored bind overlay provenance missing",
        )?;
    }
    for origin in origins {
        require(
            origin.kind() == ProvenanceKind::SourceDocument,
            "authored field provenance kind differs",
        )?;
        let (_, text) = documents
            .iter()
            .find(|(path, _)| *path == origin.source_id().as_str())
            .ok_or("authored provenance source differs")?;
        let span = origin.span().ok_or("authored provenance span missing")?;
        require(
            span.start() < span.end() && text.get(span.start()..span.end()).is_some(),
            "authored provenance span invalid",
        )?;
    }
    Ok(())
}

fn assert_neutral(application: &Application, fixture: Fixture) -> TestResult {
    let actual: BTreeSet<_> = application
        .services()
        .iter()
        .map(|service| service.value().name().as_str())
        .collect();
    require(
        actual == fixture.service_names().iter().copied().collect(),
        "full authored service inventory differs",
    )?;
    for expected in fixture.binds() {
        let service = application
            .services()
            .iter()
            .find(|service| service.value().name().as_str() == expected.service)
            .ok_or("authored bind service missing")?;
        let mount = service
            .value()
            .mounts()
            .get(expected.index)
            .ok_or("authored mount index missing")?;
        require(
            matches!(mount.value().source(), MountSource::HostPath(path) if path == &expected.source),
            "authored bind source differs",
        )?;
        require(
            mount.value().target() == expected.target
                && mount.value().read_only()
                && mount.value().selinux_relabel() == Some(expected.relabel),
            "authored bind intent differs",
        )?;
        assert_origins(mount.origins(), fixture, Some(expected.source_id))?;
        require(
            mount.origins().iter().any(|origin| {
                fixture.documents().iter().any(|(path, text)| {
                    *path == origin.source_id().as_str()
                        && origin
                            .span()
                            .and_then(|span| text.get(span.start()..span.end()))
                            .is_some_and(|text| text.contains(expected.target))
                })
            }),
            "authored bind target source evidence differs",
        )?;
        if expected.index == 1 {
            require(
                service
                    .value()
                    .mounts()
                    .first()
                    .is_some_and(|mount| matches!(mount.value().source(), MountSource::Volume(_))),
                "named-volume sibling was projected away",
            )?;
        }
    }
    Ok(())
}

fn target(profile: &NativeTargetProfile) -> TestResult<TargetProfile> {
    let version = match profile.identity().build() {
        EngineBuild::Upstream => PlatformVersion::new(29, 8, 1),
        EngineBuild::DebianPackage(_) => PlatformVersion::new(20, 10, 5),
    };
    Ok(TargetProfile::new("docker", version, Some(version))?)
}

fn hex(bytes: &[u8]) -> String {
    let alphabet = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                char::from(alphabet[usize::from(byte >> 4)]),
                char::from(alphabet[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

fn expected_context(profile: &NativeTargetProfile) -> TestResult<Value> {
    let (build, release, advertised, acquisition, rendering) = match profile.identity().build() {
        EngineBuild::Upstream => (json!({"kind": "upstream"}), "29.8.1", "1.56", "1.49", "1.56"),
        EngineBuild::DebianPackage(_) => (
            json!({"kind": "debian_package", "revision": "20.10.5+dfsg1-1+deb11u2"}),
            "20.10.5+dfsg1",
            "1.41",
            "1.41",
            "1.41",
        ),
    };
    let mode = match profile.mode() {
        DaemonMode::Rootful => "rootful",
        DaemonMode::Rootless => "rootless",
        DaemonMode::Unknown => return Err("reviewed daemon mode missing".into()),
    };
    let evidence = match (profile.identity().build(), profile.mode()) {
        (EngineBuild::DebianPackage(_), DaemonMode::Rootful) => {
            "9f26a185660badfbd6777bbf3a520f03e8fb48a777f8724bff506af5d28ba910"
        }
        (EngineBuild::DebianPackage(_), DaemonMode::Rootless) => {
            "1843873d6cae5b91d0ae3917f8238491239089905747c193e7f46027f17d891c"
        }
        (EngineBuild::Upstream, DaemonMode::Rootful) => {
            "b0825b797f052257f706b0102c5c720e7fcbbaef0e585869bb6d23d2e756fb94"
        }
        (EngineBuild::Upstream, DaemonMode::Rootless) => {
            "e6d27662271bcce957d25fe83f027eafa0264f65787d4be9b8314eb157c9ec74"
        }
        (_, DaemonMode::Unknown) => return Err("reviewed evidence lane missing".into()),
    };
    let identity = profile.identity();
    let api =
        |version: boxferry::docker::docker_lens::version::ApiVersion| format!("{}.{}", version.major, version.minor);
    require(
        identity.release().as_str() == release
            && api(identity.advertised_api_version()) == advertised
            && api(identity.acquisition_api_version()) == acquisition
            && api(identity.rendering_api_version()) == rendering
            && hex(profile.evidence_key().as_sha256_bytes()) == evidence,
        "resolved profile differs from independently reviewed cohort literals",
    )?;
    // Reviewed CommandClear source cohort 40119803977 (DockerLens ADR 0037): independent
    // literals, not operational pins or final-head native/consumer qualification.
    // Acquisition is not inferred from advertised or rendering API. Neither resolver nor
    // artifact context supplies the oracle; replacing this cohort requires independent review.
    Ok(json!({
        "kind": "target", "build": build, "engine_release": release,
        "advertised_api_version": advertised, "acquisition_api_version": acquisition,
        "rendering_api_version": rendering, "daemon_mode": mode,
        "evidence_sha256": evidence,
    }))
}

fn fields(value: &Value, expected: &[&str]) -> TestResult {
    let object = value.as_object().ok_or("closed artifact object missing")?;
    require(
        object.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected.iter().copied().collect(),
        "closed artifact field inventory differs",
    )
}

fn decimal(value: &Value) -> TestResult<&str> {
    let text = value.as_str().ok_or("opaque reference must be a decimal string")?;
    let number = text.parse::<u64>().map_err(|_| "opaque reference exceeds its bound")?;
    require(number.to_string() == text, "opaque reference decimal is not canonical")?;
    Ok(text)
}

fn bind_row(expected: &BindExpectation) -> Value {
    json!({"kind": "bind_source", "identity": expected.identity, "mount_index": expected.index.to_string(),
        "source": expected.source, "target": expected.target, "read_only": true,
        "relabel": if expected.relabel == SelinuxRelabel::Private { "private" } else { "shared" },
        "source_conditions": ["exists", "type_reviewed", "contents_reviewed", "ownership_reviewed", "permissions_reviewed"],
        "selinux_effect": "unverified",
        "selinux_conditions": ["daemon_selinux_enabled", "container_mount_label_present", "policy_filesystem_support", "relabel_authority"]})
}

fn expected_volume_mounts(fixture: Fixture, identity: &str) -> Value {
    let row = match fixture {
        Fixture::Nextcloud => match identity {
            "db" => Some(("db-data".to_owned(), "/var/lib/postgresql/data", false)),
            "cache" => Some(("redis-data".to_owned(), "/data", false)),
            "init" | "app" | "cron" => Some(("nextcloud-data".to_owned(), "/var/www/html", false)),
            "frontend" => Some(("nextcloud-data".to_owned(), "/var/www/html", true)),
            _ => None,
        },
        Fixture::Supabase => {
            if identity == format!("{PREFIX}-supabase-db") {
                Some((format!("{PREFIX}-supabase-pgdata"), "/var/lib/postgresql/data", false))
            } else if identity == format!("{PREFIX}-supabase-functions") {
                Some((format!("{PREFIX}-supabase-deno-cache"), "/root/.cache/deno", false))
            } else if identity == format!("{PREFIX}-supabase-storage") {
                Some((format!("{PREFIX}-supabase-storage"), "/var/lib/storage", false))
            } else if identity == format!("{PREFIX}-supabase-imgproxy") {
                // Canonical Compose authors storage:/var/lib/storage:ro, distinct from
                // the storage service's read-write attachment to this same named volume.
                Some((format!("{PREFIX}-supabase-storage"), "/var/lib/storage", true))
            } else {
                None
            }
        }
    };
    row.map_or_else(|| json!([]), |(source, target, read_only)| json!([{"Type": "volume", "Source": source, "Target": target, "ReadOnly": read_only}]))
}

fn assert_network_declarations(created: &BTreeSet<String>, fixture: Fixture) -> TestResult {
    let expected: BTreeSet<_> = match fixture {
        Fixture::Nextcloud => ["backend".to_owned(), "edge".to_owned()].into_iter().collect(),
        Fixture::Supabase => [format!("{PREFIX}-supabase-backend")].into_iter().collect(),
    };
    require(
        *created == expected,
        "network declarations differ or an external edge was created",
    )
}

fn assert_volume_declarations(created: &BTreeSet<String>, fixture: Fixture) -> TestResult {
    let volume_names: BTreeSet<_> = match fixture {
        Fixture::Nextcloud => ["db-data", "redis-data", "nextcloud-data"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        Fixture::Supabase => ["deno-cache", "pgdata", "storage"]
            .into_iter()
            .map(|name| format!("{PREFIX}-supabase-{name}"))
            .collect(),
    };
    require(
        *created == volume_names,
        "original volume declaration inventory differs",
    )
}

fn assert_network_request(body: &Value, fixture: Fixture) -> TestResult {
    let expected = match fixture {
        Fixture::Nextcloud if body["Name"] == "backend" => {
            json!({"Name": "backend", "Driver": "bridge", "Internal": true})
        }
        Fixture::Nextcloud => json!({"Name": "edge", "Driver": "bridge"}),
        Fixture::Supabase => json!({"Name": format!("{PREFIX}-supabase-backend"), "Driver": "bridge", "Internal": true,
            "Labels": {"io.boxferry.live-run": RUN, "io.boxferry.application": format!("{PREFIX}-supabase")}}),
    };
    require(*body == expected, "original network declaration fields differ")
}

fn assert_artifact(artifact: &Value, context: &Value, fixture: Fixture) -> TestResult {
    assert_artifact_with_external_internal(artifact, context, fixture, None)
}

fn assert_artifact_with_external_internal(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    expected_internal: Option<bool>,
) -> TestResult {
    require(
        expected_internal.is_none() || matches!(fixture, Fixture::Supabase),
        "isolation expectation references an undeclared external network",
    )?;
    fields(artifact, &["schema_version", "context", "requests", "prerequisites"])?;
    require(
        artifact["schema_version"] == json!(if expected_internal.is_some() { 3 } else { 2 })
            && artifact["context"] == *context,
        "conditional bind artifact schema or target context differs",
    )?;
    let requests = artifact["requests"].as_array().ok_or("native request list missing")?;
    let rows = artifact["prerequisites"]
        .as_array()
        .ok_or("native prerequisite list missing")?;
    require(
        requests.len()
            == match fixture {
                // Eight containers, three volumes, two networks, one secondary connect.
                Fixture::Nextcloud => 14,
                // Eleven projected containers, three volumes and one owned network.
                Fixture::Supabase => 15,
            }
            && rows.len() <= 256,
        "authored request count differs or prerequisite inventory exceeds its bound",
    )?;
    let api = format!(
        "/v{}/",
        context["rendering_api_version"]
            .as_str()
            .ok_or("selected rendering API missing")?
    );
    let mut containers = BTreeMap::new();
    let mut created_networks = BTreeSet::new();
    let mut created_volumes = BTreeSet::new();
    let mut secondary_connected = false;
    for request in requests {
        fields(request, &["method", "path", "body"])?;
        require(
            request["method"] == "POST" && request["body"].is_object(),
            "native request envelope differs",
        )?;
        let path = request["path"]
            .as_str()
            .and_then(|path| path.strip_prefix(&api))
            .ok_or("native request rendering API differs")?;
        if let Some(identity) = path.strip_prefix("containers/create?name=") {
            require(
                containers.insert(identity.to_owned(), &request["body"]).is_none(),
                "duplicate container create identity",
            )?;
        } else if path == "networks/create" {
            assert_network_request(&request["body"], fixture)?;
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("created network identity missing")?;
            require(created_networks.insert(name.to_owned()), "network declaration repeated")?;
        } else if path == "volumes/create" {
            let name = request["body"]["Name"]
                .as_str()
                .ok_or("created volume identity missing")?;
            require(created_volumes.insert(name.to_owned()), "volume declaration repeated")?;
        } else if matches!(fixture, Fixture::Nextcloud) && path == "networks/edge/connect" {
            // The authored frontend lists backend first, edge second. No aliases,
            // endpoint options, other container or other network is independently expected.
            require(!secondary_connected, "authored secondary connect is duplicated")?;
            require(
                request["body"] == json!({"Container": "frontend", "EndpointConfig": {}}),
                "authored secondary connect body differs",
            )?;
            secondary_connected = true;
        } else {
            return Err("native request endpoint is not independently authored".into());
        }
    }
    require(
        secondary_connected == matches!(fixture, Fixture::Nextcloud),
        "authored secondary connect inventory differs",
    )?;
    let expected_names: BTreeSet<_> = fixture
        .service_names()
        .iter()
        .map(|service| fixture.identity(service))
        .collect();
    require(
        containers.keys().cloned().collect::<BTreeSet<_>>() == expected_names,
        "retained generated container inventory differs",
    )?;
    let expected_binds = fixture.binds();
    assert_network_declarations(&created_networks, fixture)?;
    assert_volume_declarations(&created_volumes, fixture)?;
    assert_prerequisites(rows, &expected_binds, fixture, expected_internal)?;
    for (identity, body) in containers {
        assert_container_mounts(&identity, body, fixture, &expected_binds)?;
    }
    Ok(())
}

fn assert_container_mounts(
    identity: &str,
    body: &Value,
    fixture: Fixture,
    expected_binds: &[BindExpectation],
) -> TestResult {
    let expected: Vec<_> = expected_binds
        .iter()
        .filter(|bind| bind.identity == identity)
        .map(|bind| {
            format!(
                "{}:{}:ro,{}",
                bind.source,
                bind.target,
                if bind.relabel == SelinuxRelabel::Private {
                    "Z"
                } else {
                    "z"
                }
            )
        })
        .collect();
    let actual = body["HostConfig"].get("Binds").cloned().unwrap_or_else(|| json!([]));
    require(
        actual == json!(expected),
        "generated literal Binds differ from authored intent",
    )?;
    require(
        body["HostConfig"].get("Mounts").cloned().unwrap_or_else(|| json!([]))
            == expected_volume_mounts(fixture, identity),
        "structured mounts shadow a bind or differ from original named-volume siblings",
    )?;
    if matches!(fixture, Fixture::Supabase) {
        let role = identity
            .strip_prefix(&format!("{PREFIX}-supabase-"))
            .ok_or("projected runtime identity differs")?;
        require(
            body["Image"] == format!("registry.invalid/authored-bind/{role}:offline"),
            "projected original image differs",
        )?;
        let allowed = match (
            expected_binds.iter().any(|bind| bind.identity == identity),
            expected_volume_mounts(fixture, identity) != json!([]),
        ) {
            (true, true) => vec!["Binds", "Mounts"],
            (true, false) => vec!["Binds"],
            (false, true) => vec!["Mounts"],
            (false, false) => vec![],
        };
        if allowed.is_empty() && body.get("HostConfig").is_none() {
            // Only genuinely mount-free projected services may omit the native host object.
            fields(body, &["Image"])?;
        } else {
            fields(body, &["Image", "HostConfig"])?;
            fields(&body["HostConfig"], &allowed)?;
        }
    }
    Ok(())
}

#[test]
fn mount_free_projected_service_allows_absent_or_empty_host_config_but_no_extra_fields() -> TestResult {
    let fixture = Fixture::Supabase;
    let identity = format!("{PREFIX}-supabase-auth");
    let binds = fixture.binds();
    let mut body = json!({"Image": "registry.invalid/authored-bind/auth:offline"});
    assert_container_mounts(&identity, &body, fixture, &binds)?;
    body["HostConfig"] = json!({});
    assert_container_mounts(&identity, &body, fixture, &binds)?;
    body["Env"] = json!(["UNEXPECTED=value"]);
    require(
        assert_container_mounts(&identity, &body, fixture, &binds).is_err(),
        "mount-free projected service accepted excluded fields",
    )?;
    body.as_object_mut()
        .ok_or("synthetic container body missing")?
        .remove("Env");
    body["HostConfig"]["Mounts"] =
        json!([{"Type": "volume", "Source": "unexpected", "Target": "/unexpected", "ReadOnly": false}]);
    require(
        assert_container_mounts(&identity, &body, fixture, &binds).is_err(),
        "mount-free projected service accepted an extra mount",
    )
}

fn handwritten_supabase_bind_control(context: &Value, expected_internal: Option<bool>) -> Value {
    // Independent native-consumer data only; never a generated/admitted-profile positive.
    let fixture = Fixture::Supabase;
    let binds = fixture.binds();
    let mut requests = vec![json!({"method":"POST", "path":"/v1.56/networks/create", "body":{
        "Name":format!("{PREFIX}-supabase-backend"), "Driver":"bridge", "Internal":true,
        "Labels":{"io.boxferry.live-run":RUN,"io.boxferry.application":format!("{PREFIX}-supabase")}
    }})];
    for role in ["deno-cache", "pgdata", "storage"] {
        requests.push(
            json!({"method":"POST","path":"/v1.56/volumes/create","body":{"Name":format!("{PREFIX}-supabase-{role}")}}),
        );
    }
    for role in fixture.service_names() {
        let identity = format!("{PREFIX}-supabase-{role}");
        let mut body = json!({"Image":format!("registry.invalid/authored-bind/{role}:offline"),"HostConfig":{}});
        let named = expected_volume_mounts(fixture, &identity);
        if named != json!([]) {
            body["HostConfig"]["Mounts"] = named;
        }
        let literals = binds
            .iter()
            .filter(|bind| bind.identity == identity)
            .map(|bind| format!("{}:{}:ro,z", bind.source, bind.target))
            .collect::<Vec<_>>();
        if !literals.is_empty() {
            body["HostConfig"]["Binds"] = json!(literals);
        }
        requests.push(json!({"method":"POST","path":format!("/v1.56/containers/create?name={identity}"),"body":body}));
    }
    let mut external = json!({"kind":"network","reference":"900","identity":format!("{PREFIX}-supabase-edge"),"expected_driver":"bridge"});
    if let Some(internal) = expected_internal {
        external["expected_internal"] = json!(internal);
    }
    let mut prerequisites = vec![external];
    for (reference, bind) in binds.iter().enumerate() {
        prerequisites.push(json!({"kind":"bind_source","reference":reference.to_string(),"identity":bind.identity,
            "mount_index":bind.index.to_string(),"source":bind.source,"target":bind.target,"read_only":true,"relabel":"shared",
            "source_conditions":["exists","type_reviewed","contents_reviewed","ownership_reviewed","permissions_reviewed"],
            "selinux_effect":"unverified","selinux_conditions":["daemon_selinux_enabled","container_mount_label_present","policy_filesystem_support","relabel_authority"]}));
    }
    json!({"schema_version":if expected_internal.is_some() {3} else {2},"context":context,"requests":requests,"prerequisites":prerequisites})
}

fn handwritten_nextcloud_bind_control(context: &Value) -> Value {
    // Authored parser control only: neither generated output nor a native-admission oracle.
    let fixture = Fixture::Nextcloud;
    let binds = fixture.binds();
    let mut requests = vec![
        json!({"method":"POST", "path":"/v1.56/networks/create",
            "body":{"Name":"backend", "Driver":"bridge", "Internal":true}}),
        json!({"method":"POST", "path":"/v1.56/networks/create",
            "body":{"Name":"edge", "Driver":"bridge"}}),
    ];
    for volume in ["db-data", "redis-data", "nextcloud-data"] {
        requests.push(json!({"method":"POST", "path":"/v1.56/volumes/create", "body":{"Name":volume}}));
    }
    for identity in fixture.service_names() {
        let mut body = json!({"Image":format!("registry.invalid/authored-bind/{identity}:offline"), "HostConfig":{}});
        let named = expected_volume_mounts(fixture, identity);
        if named != json!([]) {
            body["HostConfig"]["Mounts"] = named;
        }
        let literals = binds
            .iter()
            .filter(|bind| bind.identity == *identity)
            .map(|bind| format!("{}:{}:ro,Z", bind.source, bind.target))
            .collect::<Vec<_>>();
        if !literals.is_empty() {
            body["HostConfig"]["Binds"] = json!(literals);
        }
        requests
            .push(json!({"method":"POST", "path":format!("/v1.56/containers/create?name={identity}"), "body":body}));
    }
    requests.push(json!({"method":"POST", "path":"/v1.56/networks/edge/connect",
        "body":{"Container":"frontend", "EndpointConfig":{}}}));
    let prerequisites: Vec<_> = binds
        .iter()
        .enumerate()
        .map(|(index, bind)| {
            let mut row = bind_row(bind);
            row["reference"] = json!(index.to_string());
            row
        })
        .collect();
    json!({"schema_version":2, "context":context, "requests":requests, "prerequisites":prerequisites})
}

fn assert_closed_request_mutations(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    control: Option<ExternalNetworkControl>,
) -> TestResult {
    // Every base/control is authored here; expectations never come from generated requests.
    assert_artifact_with_control(artifact, context, fixture, control)?;
    let reject = |changed: &Value, reason| {
        require(
            assert_artifact_with_control(changed, context, fixture, control).is_err(),
            reason,
        )
    };
    assert_unknown_request_endpoint_mutations(artifact, context, fixture, control)?;
    match fixture {
        Fixture::Nextcloud => {
            // Independent fixed position in the handwritten control: 2 + 3 + 8 creates.
            // Production artifact ordering is not used to obtain this expectation.
            const CONNECT: usize = 13;
            let expected_connect = json!({"method":"POST", "path":"/v1.56/networks/edge/connect",
                "body":{"Container":"frontend", "EndpointConfig":{}}});
            require(
                artifact["requests"][CONNECT] == expected_connect,
                "handwritten secondary connect differs from the independent scenario inventory",
            )?;
            let mut missing = artifact.clone();
            missing["requests"]
                .as_array_mut()
                .ok_or("handwritten requests missing")?
                .remove(CONNECT);
            reject(&missing, "closed consumer accepted a missing secondary connect")?;
            let mut duplicate = artifact.clone();
            duplicate["requests"]
                .as_array_mut()
                .ok_or("handwritten requests missing")?
                .push(expected_connect.clone());
            reject(&duplicate, "closed consumer accepted a duplicate secondary connect")?;
            duplicate["requests"]
                .as_array_mut()
                .ok_or("handwritten requests missing")?
                .remove(0);
            require(
                assert_artifact_with_control(&duplicate, context, fixture, control)
                    .err()
                    .is_some_and(|error| error.to_string() == "authored secondary connect is duplicated"),
                "same-count duplicate did not fail at the secondary-connect inventory",
            )?;
            for network in ["backend", "unreviewed-edge", CONTROL_NETWORK] {
                let mut wrong_network = artifact.clone();
                wrong_network["requests"][CONNECT]["path"] = json!(format!("/v1.56/networks/{network}/connect"));
                reject(&wrong_network, "closed consumer accepted another network connection")?;
            }
            for body in [
                json!({"Container":"app", "EndpointConfig":{}}),
                json!({"Container":null, "EndpointConfig":{}}),
                json!({"EndpointConfig":{}}),
                json!({"Container":"frontend"}),
                json!({"Container":"frontend", "EndpointConfig":null}),
                json!({"Container":"frontend", "EndpointConfig":false}),
                json!({"Container":"frontend", "EndpointConfig":[]}),
                json!({"Container":"frontend", "EndpointConfig":"{}"}),
                json!({"Container":"frontend", "EndpointConfig":{"Aliases":[]}}),
                json!({"Container":"frontend", "EndpointConfig":{"IPAMConfig":{}}}),
                json!({"Container":"frontend", "EndpointConfig":{}, "extra":true}),
            ] {
                let mut altered = artifact.clone();
                altered["requests"][CONNECT]["body"] = body;
                reject(&altered, "closed consumer accepted an altered secondary-connect body")?;
            }
        }
        Fixture::Supabase => {
            for network in ["edge".to_owned(), format!("{PREFIX}-supabase-edge")] {
                let connect = json!({"method":"POST", "path":format!("/v1.56/networks/{network}/connect"),
                    "body":{"Container":format!("{PREFIX}-supabase-kong"), "EndpointConfig":{}}});
                let mut appended = artifact.clone();
                appended["requests"]
                    .as_array_mut()
                    .ok_or("handwritten requests missing")?
                    .push(connect.clone());
                reject(&appended, "projected Supabase accepted an appended connection")?;
                let mut replaced = artifact.clone();
                replaced["requests"][0] = connect;
                reject(&replaced, "projected Supabase accepted a same-count connection")?;
            }
        }
    }
    assert_external_network_create_mutations(artifact, context, fixture, control)
}

fn assert_unknown_request_endpoint_mutations(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    control: Option<ExternalNetworkControl>,
) -> TestResult {
    for endpoint in [
        "system/prune",
        "images/prune",
        "volumes/prune",
        "containers/frontend/start",
        "networks/edge/disconnect",
    ] {
        let unknown = json!({"method":"POST", "path":format!("/v1.56/{endpoint}"), "body":{}});
        let mut appended = artifact.clone();
        appended["requests"]
            .as_array_mut()
            .ok_or("handwritten requests missing")?
            .push(unknown.clone());
        require(
            assert_artifact_with_control(&appended, context, fixture, control).is_err(),
            "closed consumer accepted an appended unknown endpoint",
        )?;
        let mut replaced = artifact.clone();
        replaced["requests"][0] = unknown;
        require(
            assert_artifact_with_control(&replaced, context, fixture, control)
                .err()
                .is_some_and(|error| error.to_string() == "native request endpoint is not independently authored"),
            "same-count unknown replacement did not fail at the closed endpoint boundary",
        )?;
    }
    Ok(())
}

fn assert_external_network_create_mutations(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    control: Option<ExternalNetworkControl>,
) -> TestResult {
    if control.is_some() {
        let create = json!({"method":"POST", "path":"/v1.56/networks/create",
            "body":{"Name":CONTROL_NETWORK, "Driver":"bridge"}});
        let mut appended = artifact.clone();
        appended["requests"]
            .as_array_mut()
            .ok_or("handwritten requests missing")?
            .push(create.clone());
        require(
            assert_artifact_with_control(&appended, context, fixture, control).is_err(),
            "declaration-only external control created a network",
        )?;
        let mut replaced = artifact.clone();
        replaced["requests"][0] = create;
        require(
            assert_artifact_with_control(&replaced, context, fixture, control).is_err(),
            "external control replaced an authored network create",
        )?;
    }
    Ok(())
}

#[test]
fn handwritten_bind_consumers_close_every_request_endpoint_and_secondary_connect() -> TestResult {
    let context = json!({"kind":"target", "rendering_api_version":"1.56"});
    assert_closed_request_mutations(
        &handwritten_nextcloud_bind_control(&context),
        &context,
        Fixture::Nextcloud,
        None,
    )?;
    assert_closed_request_mutations(
        &handwritten_supabase_bind_control(&context, None),
        &context,
        Fixture::Supabase,
        None,
    )
}

#[test]
fn declaration_only_external_controls_preserve_closed_fixture_request_inventories() -> TestResult {
    let context = json!({"kind":"target", "rendering_api_version":"1.56"});
    for fixture in [Fixture::Nextcloud, Fixture::Supabase] {
        for internal in [None, Some(false), Some(true)] {
            let control = ExternalNetworkControl { internal };
            let mut artifact = match fixture {
                Fixture::Nextcloud => handwritten_nextcloud_bind_control(&context),
                Fixture::Supabase => handwritten_supabase_bind_control(&context, None),
            };
            artifact["schema_version"] = json!(if internal.is_some() { 3 } else { 2 });
            let mut row = json!({"kind":"network", "reference":"901", "identity":CONTROL_NETWORK,
                "expected_driver":"bridge"});
            if let Some(internal) = internal {
                row["expected_internal"] = json!(internal);
            }
            artifact["prerequisites"]
                .as_array_mut()
                .ok_or("handwritten prerequisites missing")?
                .push(row);
            assert_closed_request_mutations(&artifact, &context, fixture, Some(control))?;
        }
    }
    Ok(())
}

#[test]
fn handwritten_schema3_external_constraints_keep_all_bind_obligations_without_native_admission() -> TestResult {
    let context = json!({"kind":"target","rendering_api_version":"1.56"});
    for expected in [None, Some(false), Some(true)] {
        let artifact = handwritten_supabase_bind_control(&context, expected);
        assert_artifact_with_external_internal(&artifact, &context, Fixture::Supabase, expected)?;
        let mut reordered = artifact.clone();
        reordered["prerequisites"]
            .as_array_mut()
            .ok_or("handwritten prerequisite rows")?
            .reverse();
        assert_artifact_with_external_internal(&reordered, &context, Fixture::Supabase, expected)?;
        for value in [json!(null), json!(0), json!("false"), json!(!expected.unwrap_or(false))] {
            let mut changed = artifact.clone();
            changed["schema_version"] = json!(3);
            changed["prerequisites"][0]["expected_internal"] = value;
            require(
                assert_artifact_with_external_internal(&changed, &context, Fixture::Supabase, expected).is_err(),
                "unreviewed external boolean accepted",
            )?;
        }
        for (field, value) in [
            ("source_conditions", json!([])),
            ("selinux_effect", json!("verified")),
            ("mount_index", json!("99")),
            ("reference", json!("900")),
        ] {
            let mut changed = artifact.clone();
            changed["prerequisites"][1][field] = value;
            require(
                assert_artifact_with_external_internal(&changed, &context, Fixture::Supabase, expected).is_err(),
                "schema-3 control dropped a bind obligation",
            )?;
        }
        if expected.is_some() {
            for schema in [1, 2] {
                let mut changed = artifact.clone();
                changed["schema_version"] = json!(schema);
                require(
                    assert_artifact_with_external_internal(&changed, &context, Fixture::Supabase, expected).is_err(),
                    "explicit isolation accepted schema downgrade",
                )?;
            }
        }
    }
    Ok(())
}

fn assert_prerequisites(
    rows: &[Value],
    expected_binds: &[BindExpectation],
    fixture: Fixture,
    expected_internal: Option<bool>,
) -> TestResult {
    let mut seen = BTreeSet::new();
    let mut groups = BTreeMap::new();
    let mut owners = BTreeMap::new();
    let mut external_references = BTreeSet::new();
    for row in rows {
        if row["kind"] != "bind_source" {
            let mut expected_fields = vec!["kind", "reference", "identity", "expected_driver"];
            if expected_internal.is_some() {
                expected_fields.push("expected_internal");
            }
            fields(row, &expected_fields)?;
            require(
                row.get("expected_internal") == expected_internal.map(|value| json!(value)).as_ref(),
                "external isolation prerequisite differs from the authored expectation",
            )?;
            require(
                matches!(fixture, Fixture::Supabase)
                    && row["kind"] == "network"
                    && row["identity"] == format!("{PREFIX}-supabase-edge")
                    && row["expected_driver"] == "bridge",
                "fixture external prerequisite inventory differs",
            )?;
            require(
                external_references.insert(decimal(&row["reference"])?),
                "external references repeat",
            )?;
            continue;
        }
        fields(
            row,
            &[
                "kind",
                "reference",
                "identity",
                "mount_index",
                "source",
                "target",
                "read_only",
                "relabel",
                "source_conditions",
                "selinux_effect",
                "selinux_conditions",
            ],
        )?;
        let reference = decimal(&row["reference"])?;
        let index = decimal(&row["mount_index"])?;
        let identity = row["identity"].as_str().ok_or("bind prerequisite identity missing")?;
        let expected = expected_binds
            .iter()
            .find(|expected| expected.identity == identity && expected.index.to_string() == index)
            .ok_or("bind prerequisite is not independently authored")?;
        require(seen.insert((identity, index)), "duplicate bind prerequisite")?;
        let mut intent = row.clone();
        intent
            .as_object_mut()
            .ok_or("bind prerequisite object missing")?
            .remove("reference");
        require(
            intent == bind_row(expected),
            "bind prerequisite intent or obligations differ",
        )?;
        require(
            groups.insert(identity, reference).is_none_or(|old| old == reference)
                && owners.insert(reference, identity).is_none_or(|old| old == identity),
            "opaque bind reference groups differ",
        )?;
    }
    require(
        seen.len() == expected_binds.len()
            && external_references.len() == usize::from(matches!(fixture, Fixture::Supabase))
            && external_references.is_disjoint(&owners.keys().copied().collect()),
        "bind prerequisite bijection or reference separation differs",
    )?;
    Ok(())
}

fn assert_dependencies(output: &DockerOutput, fixture: Fixture) -> TestResult {
    let expected = fixture.edges();
    require(
        output.dependency_decisions().len() == expected.len(),
        "typed dependency inventory differs",
    )?;
    let mut seen = BTreeSet::new();
    for decision in output.dependency_decisions() {
        require(
            seen.insert((decision.service(), decision.dependency())),
            "typed dependency repeats",
        )?;
        let (_, _, condition) = expected
            .iter()
            .find(|(service, dependency, _)| *service == decision.service() && *dependency == decision.dependency())
            .ok_or("typed dependency is not independently authored")?;
        require(
            decision.service_identity() == fixture.identity(decision.service()).as_bytes()
                && decision.dependency_identity() == fixture.identity(decision.dependency()).as_bytes()
                && !decision.sensitive_identity(),
            "typed dependency identities differ",
        )?;
        require(
            decision.condition() == Some(condition)
                && decision.required()
                    == match fixture {
                        Fixture::Nextcloud => Some(true),
                        Fixture::Supabase => None,
                    }
                && decision.restart().is_none(),
            "typed dependency choreography differs",
        )?;
        assert_origins(decision.reference_origins(), fixture, None)?;
        assert_origins(decision.condition_origins(), fixture, None)?;
        match fixture {
            Fixture::Nextcloud => assert_origins(decision.required_origins(), fixture, None)?,
            Fixture::Supabase => require(
                decision.required_origins().is_empty(),
                "default required flag gained authored provenance",
            )?,
        }
        require(
            decision.restart_origins().is_empty(),
            "default restart flag gained authored provenance",
        )?;
    }
    Ok(())
}

fn assert_private_text(text: &str, fixture: Fixture) -> TestResult {
    require(
        !text.contains(SECRET) && !text.contains(ROOT) && !text.contains("/srv/boxferry-fixture/nextcloud-application"),
        "protected value or bind path leaked into presentation",
    )?;
    if matches!(fixture, Fixture::Nextcloud) {
        for line in NEXTCLOUD_ENV.lines().filter(|line| !line.starts_with('#')) {
            if let Some((_, value)) = line.split_once('=') {
                require(
                    !text.contains(value),
                    "public example credential canary leaked into presentation",
                )?;
            }
        }
    }
    Ok(())
}

fn supabase_bind_mount_only_projection(original: &Application) -> TestResult<Application> {
    // Authorized scope, never automatic recovery from a failed full-graph export. Keep original
    // import outcomes/diagnostics in the caller. Networks are declaration-only; no service alias
    // is declassified and no protected command or environment value is promoted into output.
    let mut projected = Application::new(Identifier::new("supabase-bind-mount-only-with-resource-declarations")?);
    for volume in original.volumes() {
        projected.add_volume(volume.clone())?;
    }
    for network in original.networks() {
        projected.add_network(network.clone())?;
    }
    for original_service in original.services() {
        let source = original_service.value();
        let mut service = Service::new(source.name().clone());
        service.set_image(source.image().ok_or("original Supabase image missing")?.clone());
        service.set_runtime_name(
            source
                .runtime_name()
                .ok_or("original Supabase runtime identity missing")?
                .clone(),
        );
        for mount in source.mounts() {
            service.add_mount(mount.clone());
        }
        require(
            service.image() == source.image()
                && service.runtime_name() == source.runtime_name()
                && service.mounts() == source.mounts(),
            "explicit bind/mount-only projection changed retained source values or provenance",
        )?;
        require(
            service.environment().is_empty()
                && service.dependencies().is_empty()
                && service.networks().is_empty()
                && service.healthcheck().is_none(),
            "bind/mount-only projection retained excluded service configuration",
        )?;
        let mut service = Sourced::generated(service);
        for origin in original_service.origins() {
            service.add_origin(origin.clone());
        }
        projected.add_service(service)?;
    }
    require(
        projected.volumes() == original.volumes() && projected.networks() == original.networks(),
        "bind/mount-only projection changed original resource declarations",
    )?;
    assert_neutral(&projected, Fixture::Supabase)?;
    Ok(projected)
}

fn generated_fixture(fixture: Fixture, profile: &NativeTargetProfile) -> TestResult<Vec<u8>> {
    generated_fixture_with_control(fixture, profile, None)
}

fn import_bind_fixture_with_control(
    fixture: Fixture,
    control: Option<ExternalNetworkControl>,
) -> TestResult<ImportResult> {
    let imported = import_fixture(fixture)?;
    assert_neutral(
        imported.application().ok_or("full authored application missing")?,
        fixture,
    )?;
    let imported = match fixture {
        Fixture::Nextcloud => imported,
        Fixture::Supabase => {
            let (application, outcomes, diagnostics) = imported.into_parts();
            let application = application.ok_or("full Supabase application missing")?;
            ImportResult::new(
                Some(supabase_bind_mount_only_projection(&application)?),
                outcomes,
                diagnostics,
            )
        }
    };
    if let Some(control) = control {
        let (application, outcomes, diagnostics) = imported.into_parts();
        let mut application = application.ok_or("control application missing")?;
        control.add_to(&mut application)?;
        assert_neutral(&application, fixture)?;
        Ok(ImportResult::new(Some(application), outcomes, diagnostics))
    } else {
        Ok(imported)
    }
}

fn generated_fixture_with_control(
    fixture: Fixture,
    profile: &NativeTargetProfile,
    control: Option<ExternalNetworkControl>,
) -> TestResult<Vec<u8>> {
    let imported = import_bind_fixture_with_control(fixture, control)?;
    let exporter =
        DockerExporter::new(profile.clone())?.with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral);
    let result = convert_imported(imported, &exporter, &target(profile)?, LossPolicy::AllowApproximate)?;
    if let Some(control) = control {
        let subject = format!("networks.{CONTROL_NETWORK}.internal");
        let outcomes: Vec<_> = result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect();
        if control.internal.is_some() {
            let [outcome] = outcomes.as_slice() else {
                return Err("explicit external control lost its exact isolation outcome".into());
            };
            require(
                outcome.kind() == ConversionKind::Exact
                    && outcome.diagnostic().is_none()
                    && outcome
                        .origins()
                        .contains(&Provenance::source(SourceId::new(CONTROL_NETWORK)?)),
                "external control isolation outcome or authored provenance differs",
            )?;
        } else {
            require(outcomes.is_empty(), "unconstrained control gained an isolation outcome")?;
        }
    }
    assert_private_text(&format!("{:?} {:?}", result.outcomes(), result.diagnostics()), fixture)?;
    if result.is_blocked() {
        let losses: Vec<_> = result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() != ConversionKind::Exact)
            .map(|outcome| format!("{}:{:?}:{:?}", outcome.subject(), outcome.kind(), outcome.diagnostic()))
            .collect();
        let scope = match fixture {
            Fixture::Nextcloud => "full Nextcloud graph",
            Fixture::Supabase => "explicit Supabase bind/mount-only projection with resource declarations",
        };
        return Err(format!("{scope} blocked, without fallback: {}", losses.join(", ")).into());
    }
    require(
        result
            .outcomes()
            .iter()
            .all(|outcome| matches!(outcome.kind(), ConversionKind::Exact | ConversionKind::Approximate)),
        "unsupported authored configuration reached output",
    )?;
    let output = result.output().ok_or("authorized full graph output missing")?;
    require(
        output.native_profile() == profile
            && output.evidence().record_key() == profile.evidence_key()
            && output.network_prerequisites().len()
                == usize::from(matches!(fixture, Fixture::Supabase)) + usize::from(control.is_some())
            && output.volume_prerequisites().is_empty(),
        "selected scope native profile or external prerequisite inventory differs",
    )?;
    if matches!(fixture, Fixture::Supabase) {
        require(
            output.network_prerequisites().iter().any(|row| {
                row.identity() == format!("{PREFIX}-supabase-edge").as_bytes() && row.expected_internal.is_none()
            }),
            "projected declaration lost exact external edge prerequisite",
        )?;
    }
    assert_private_text(&format!("{output:?} {:?}", output.dependency_decisions()), fixture)?;
    assert_dependencies(output, fixture)?;
    let bytes = output
        .complete_bytes()
        .map_err(|_| "complete native artifact unavailable")?;
    require(
        !bytes.is_empty() && bytes.len() <= 1_048_576,
        "complete native artifact exceeds its bound",
    )?;
    let artifact: Value = serde_json::from_slice(&bytes)?;
    assert_artifact_with_control(&artifact, &expected_context(profile)?, fixture, control)?;
    if matches!(fixture, Fixture::Nextcloud) {
        require(
            artifact["requests"]
                .as_array()
                .is_some_and(|requests| requests.iter().any(|request| request["body"].get("Env").is_some())),
            "explicit literal environment authorization was not exercised",
        )?;
    }
    if matches!(fixture, Fixture::Supabase) {
        require(
            !std::str::from_utf8(&bytes)?.contains(SECRET),
            "bind/mount-only projection leaked excluded environment values",
        )?;
    }
    Ok(bytes)
}

#[test]
fn full_nextcloud_and_explicit_supabase_bind_mount_only_projection_retain_binds_for_all_profiles() -> TestResult {
    let profiles: Vec<_> = TargetCapabilityCatalog::reviewed().profiles().cloned().collect();
    require(profiles.len() == 4, "four reviewed candidate profiles required")?;
    let mut lanes = BTreeSet::new();
    for profile in profiles {
        let context = expected_context(&profile)?;
        let lane = format!(
            "{}-{}",
            if matches!(profile.identity().build(), EngineBuild::Upstream) {
                "upstream"
            } else {
                "debian"
            },
            context["daemon_mode"].as_str().ok_or("profile daemon mode missing")?
        );
        require(lanes.insert(lane), "reviewed profile lane repeats")?;
        let _ = generated_fixture(Fixture::Nextcloud, &profile)?;
        let _ = generated_fixture(Fixture::Supabase, &profile)?;
    }
    Ok(())
}

fn assert_artifact_with_control(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    control: Option<ExternalNetworkControl>,
) -> TestResult {
    let Some(control) = control else {
        return assert_artifact(artifact, context, fixture);
    };
    fields(artifact, &["schema_version", "context", "requests", "prerequisites"])?;
    require(
        artifact["schema_version"] == json!(if control.internal.is_some() { 3 } else { 2 })
            && artifact["context"] == *context,
        "generated network-control schema or independent target context differs",
    )?;
    let mut fixture_artifact = artifact.clone();
    let rows = fixture_artifact["prerequisites"]
        .as_array_mut()
        .ok_or("generated control prerequisites missing")?;
    let index = rows
        .iter()
        .position(|row| row["identity"] == CONTROL_NETWORK)
        .ok_or("generated external network control missing")?;
    let row = rows.remove(index);
    let mut expected_fields = vec!["kind", "reference", "identity", "expected_driver"];
    if control.internal.is_some() {
        expected_fields.push("expected_internal");
    }
    fields(&row, &expected_fields)?;
    require(
        row["kind"] == "network"
            && row["expected_driver"] == "bridge"
            && row.get("expected_internal") == control.internal.map(|value| json!(value)).as_ref(),
        "generated external network control differs from independently authored intent",
    )?;
    let reference = decimal(&row["reference"])?;
    for other in rows.iter() {
        require(
            decimal(&other["reference"])? != reference,
            "external control reference overlaps a fixture resource",
        )?;
    }
    // Test-only composition: after independently validating the control and original schema,
    // apply the existing closed schema-2 consumer to the unchanged fixture requests and rows.
    // This clone is never emitted, passed to the planner, or used as a generated positive oracle.
    fixture_artifact["schema_version"] = json!(2);
    assert_artifact(&fixture_artifact, context, fixture)
}

fn assert_control_bind_obligations(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    control: ExternalNetworkControl,
) -> TestResult {
    let rows = artifact["prerequisites"]
        .as_array()
        .ok_or("control prerequisites missing")?;
    for (bind_index, row) in rows.iter().enumerate().filter(|(_, row)| row["kind"] == "bind_source") {
        for field in [
            "reference",
            "identity",
            "mount_index",
            "source",
            "target",
            "read_only",
            "relabel",
            "source_conditions",
            "selinux_effect",
            "selinux_conditions",
        ] {
            let mut altered = artifact.clone();
            altered["prerequisites"][bind_index]
                .as_object_mut()
                .ok_or("control bind row missing")?
                .remove(field);
            require(
                assert_artifact_with_control(&altered, context, fixture, Some(control)).is_err(),
                "generated network control accepted an omitted bind obligation",
            )?;
        }
        require(row["read_only"] == true, "authored read-only bind differs")?;
    }
    Ok(())
}

#[test]
fn generated_external_constraint_controls_retain_actual_fixture_binds_on_all_four_profiles() -> TestResult {
    let profiles: Vec<_> = TargetCapabilityCatalog::reviewed().profiles().cloned().collect();
    require(profiles.len() == 4, "four sealed network-control profiles required")?;
    let mut lanes = BTreeSet::new();
    for profile in profiles {
        let context = expected_context(&profile)?;
        require(
            lanes.insert(format!("{}:{}", context["build"], context["daemon_mode"])),
            "sealed network-control profile lane repeats",
        )?;
        for fixture in [Fixture::Nextcloud, Fixture::Supabase] {
            for internal in [None, Some(false), Some(true)] {
                let control = ExternalNetworkControl { internal };
                let bytes = generated_fixture_with_control(fixture, &profile, Some(control))?;
                let artifact: Value = serde_json::from_slice(&bytes)?;
                let rows = artifact["prerequisites"]
                    .as_array()
                    .ok_or("control prerequisites missing")?;
                let index = rows
                    .iter()
                    .position(|row| row["identity"] == CONTROL_NETWORK)
                    .ok_or("external control row missing")?;
                for value in [json!(null), json!(0), json!("false"), json!(!internal.unwrap_or(false))] {
                    let mut altered = artifact.clone();
                    altered["prerequisites"][index]["expected_internal"] = value;
                    require(
                        assert_artifact_with_control(&altered, &context, fixture, Some(control)).is_err(),
                        "generated control accepted a substituted external constraint",
                    )?;
                }
                let mut external_fields = vec!["kind", "reference", "identity", "expected_driver"];
                if internal.is_some() {
                    external_fields.push("expected_internal");
                }
                for field in external_fields {
                    let mut altered = artifact.clone();
                    altered["prerequisites"][index]
                        .as_object_mut()
                        .ok_or("external control row missing")?
                        .remove(field);
                    require(
                        assert_artifact_with_control(&altered, &context, fixture, Some(control)).is_err(),
                        "generated control accepted an omitted external-network field",
                    )?;
                }
                let mut omitted = artifact.clone();
                omitted["prerequisites"]
                    .as_array_mut()
                    .ok_or("control prerequisites missing")?
                    .remove(index);
                require(
                    assert_artifact_with_control(&omitted, &context, fixture, Some(control)).is_err(),
                    "generated control accepted an omitted external-network prerequisite",
                )?;
                let mut reordered = artifact.clone();
                reordered["prerequisites"]
                    .as_array_mut()
                    .ok_or("control prerequisites missing")?
                    .reverse();
                assert_artifact_with_control(&reordered, &context, fixture, Some(control))?;
                let mut renamed = artifact.clone();
                let mut identities = BTreeMap::new();
                for row in renamed["prerequisites"]
                    .as_array_mut()
                    .ok_or("control prerequisites missing")?
                {
                    let identity = row["identity"].as_str().ok_or("control identity missing")?.to_owned();
                    let next = u64::try_from(identities.len())?;
                    let reference = identities
                        .entry(identity)
                        .or_insert_with(|| (u64::MAX - next).to_string());
                    row["reference"] = json!(reference);
                }
                assert_artifact_with_control(&renamed, &context, fixture, Some(control))?;
                for api in [
                    "advertised_api_version",
                    "acquisition_api_version",
                    "rendering_api_version",
                ] {
                    let mut altered = artifact.clone();
                    altered["context"][api] = json!("1.00");
                    require(
                        assert_artifact_with_control(&altered, &context, fixture, Some(control)).is_err(),
                        "generated control accepted an altered target API dimension",
                    )?;
                }
                assert_control_bind_obligations(&artifact, &context, fixture, control)?;
                for schema in [1, if internal.is_some() { 2 } else { 3 }] {
                    let mut altered = artifact.clone();
                    altered["schema_version"] = json!(schema);
                    require(
                        assert_artifact_with_control(&altered, &context, fixture, Some(control)).is_err(),
                        "generated control accepted a substituted schema",
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[test]
fn default_environment_withholding_and_mismatched_target_version_fail_closed() -> TestResult {
    let fixture = Fixture::Nextcloud;
    for profile in TargetCapabilityCatalog::reviewed().profiles() {
        let result = convert_imported(
            import_fixture(fixture)?,
            &DockerExporter::new(profile.clone())?,
            &target(profile)?,
            LossPolicy::AllowApproximate,
        )?;
        require(
            result.is_blocked() && result.output().is_none(),
            "default environment withholding must block non-partial authorization",
        )?;
        require(
            result.outcomes().iter().any(|outcome| {
                outcome.kind() == ConversionKind::Unsupported
                    && outcome.subject().starts_with("services.db.environment.")
            }),
            "default environment withholding lost its structured outcome",
        )?;
        assert_private_text(&format!("{:?} {:?}", result.outcomes(), result.diagnostics()), fixture)?;
        let wrong = TargetProfile::new(
            "docker",
            PlatformVersion::new(1, 0, 0),
            Some(PlatformVersion::new(1, 0, 0)),
        )?;
        let exporter =
            DockerExporter::new(profile.clone())?.with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral);
        let result = convert_imported(import_fixture(fixture)?, &exporter, &wrong, LossPolicy::AllowPartial)?;
        require(
            result.is_blocked() && result.output().is_none(),
            "mismatched explicit target version must not export",
        )?;
    }
    Ok(())
}

#[test]
fn full_supabase_graph_keeps_public_aliases_and_external_edge_but_refuses_protected_health_command() -> TestResult {
    // Literal aliases and explicitly public identity interpolation remain public under ADR 0073.
    // IncludeLiteral authorizes environment output only; the auth-key-bearing health command
    // stays protected and cannot enter the native review artifact.
    let imported = import_fixture(Fixture::Supabase)?;
    let application = imported.application().ok_or("full Supabase application missing")?;
    assert_full_supabase_privacy_neutral(application)?;
    for profile in TargetCapabilityCatalog::reviewed().profiles() {
        let exporter =
            DockerExporter::new(profile.clone())?.with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral);
        let result = convert_imported(
            imported.clone(),
            &exporter,
            &target(profile)?,
            LossPolicy::AllowApproximate,
        )?;
        require(
            result.is_blocked() && result.output().is_none(),
            "full Supabase privacy refusal must remain blocked",
        )?;
        require(
            !result.outcomes().iter().any(|outcome| {
                outcome.kind() == ConversionKind::Unsupported && outcome.subject().contains(".aliases.")
            }),
            "public literal aliases must not acquire an unsupported privacy refusal",
        )?;
        require(
            result.outcomes().iter().any(|outcome| {
                outcome.kind() == ConversionKind::Unsupported
                    && outcome.subject() == "services.realtime.healthcheck.command"
            }),
            "full Supabase protected health-command refusal missing",
        )?;
        assert_private_text(
            &format!("{:?} {:?}", result.outcomes(), result.diagnostics()),
            Fixture::Supabase,
        )?;
    }
    Ok(())
}

fn assert_full_supabase_privacy_neutral(application: &Application) -> TestResult {
    assert_neutral(application, Fixture::Supabase)?;
    let network_names: BTreeSet<_> = application
        .networks()
        .iter()
        .map(|row| row.value().name().as_str())
        .collect();
    require(
        network_names == ["backend", "edge"].into_iter().collect(),
        "full Supabase network inventory differs",
    )?;
    let edge = application
        .networks()
        .iter()
        .find(|row| row.value().name().as_str() == "edge")
        .ok_or("external edge missing")?;
    require(
        edge.value().ownership() == ResourceOwnership::External
            && edge
                .value()
                .runtime_name()
                .is_some_and(|name| name.value().expose() == format!("{PREFIX}-supabase-edge")),
        "full Supabase external edge intent differs",
    )?;
    require(
        application.services().len() == Fixture::Supabase.service_names().len()
            && application
                .services()
                .iter()
                .map(|service| service.value().name().as_str())
                .collect::<BTreeSet<_>>()
                == Fixture::Supabase.service_names().iter().copied().collect(),
        "full Supabase service inventory differs",
    )?;
    for service in application.services() {
        let name = service.value().name().as_str();
        require(
            service.value().networks().len() == if name == "kong" { 2 } else { 1 },
            "full Supabase attachment inventory differs",
        )?;
        for (index, attachment) in service.value().networks().iter().enumerate() {
            let expected: &[&str] = match (name, index) {
                ("realtime", 0) => &["realtime-dev.supabase-realtime", "realtime"],
                ("kong", 1) => &["supabase"],
                (_, 0) => &[name],
                _ => return Err("unexpected authored Supabase attachment".into()),
            };
            require(
                attachment.value().network().as_str() == if index == 1 { "edge" } else { "backend" }
                    && attachment
                        .value()
                        .aliases()
                        .iter()
                        .map(String::as_str)
                        .eq(expected.iter().copied())
                    && attachment.value().alias_sensitivities().len() == attachment.value().aliases().len()
                    && attachment
                        .value()
                        .alias_sensitivities()
                        .iter()
                        .all(|sensitive| !*sensitive),
                "full Supabase public alias inventory or per-scalar classification differs",
            )?;
        }
    }
    let realtime = application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == "realtime")
        .ok_or("realtime service missing")?;
    let command = realtime
        .value()
        .healthcheck()
        .and_then(|health| health.value().command())
        .ok_or("realtime health command missing")?;
    let protected = match command.value() {
        HealthcheckCommand::Shell(text) => text.is_sensitive() && text.expose().contains(SECRET),
        HealthcheckCommand::Exec(arguments) => arguments
            .iter()
            .any(|argument| argument.is_sensitive() && argument.expose().contains(SECRET)),
        _ => false,
    };
    require(
        protected,
        "realtime interpolated auth key lost health-command protection",
    )?;
    Ok(())
}

#[test]
fn independent_consumer_rejects_bind_and_context_changes_but_accepts_opaque_group_renaming() -> TestResult {
    let catalogue = TargetCapabilityCatalog::reviewed();
    let profile = catalogue
        .profiles()
        .find(|profile| {
            matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootless
        })
        .ok_or("upstream-rootless reviewed profile missing")?;
    for fixture in [Fixture::Nextcloud, Fixture::Supabase] {
        assert_bind_mutations(fixture, profile)?;
    }
    Ok(())
}

fn assert_bind_mutations(fixture: Fixture, profile: &NativeTargetProfile) -> TestResult {
    let bytes = generated_fixture(fixture, profile)?;
    let context = expected_context(profile)?;
    let artifact: Value = serde_json::from_slice(&bytes)?;
    let selected = fixture
        .binds()
        .into_iter()
        .find(|bind| bind.index == 1)
        .ok_or("independently authored index-one bind missing")?;
    for (field, value) in [
        ("mount_index", json!("0")),
        ("source", json!("/wrong-source")),
        ("read_only", json!(false)),
        (
            "relabel",
            json!(if selected.relabel == SelinuxRelabel::Private {
                "shared"
            } else {
                "private"
            }),
        ),
        ("selinux_effect", json!("verified")),
        ("source_conditions", json!([])),
        ("selinux_conditions", json!([])),
        ("reference", json!("01")),
    ] {
        let mut altered = artifact.clone();
        let rows = altered["prerequisites"].as_array_mut().ok_or("prerequisites missing")?;
        let row = rows
            .iter_mut()
            .find(|row| row["identity"] == selected.identity)
            .ok_or("authored bind prerequisite missing")?;
        row[field] = value;
        require(
            assert_artifact(&altered, &context, fixture).is_err(),
            "independent bind consumer accepted altered intent",
        )?;
    }
    let mut altered = artifact.clone();
    altered["schema_version"] = json!(1);
    require(
        assert_artifact(&altered, &context, fixture).is_err(),
        "bind consumer accepted schema downgrade",
    )?;
    altered = artifact.clone();
    altered["context"]["rendering_api_version"] = json!("1.00");
    require(
        assert_artifact(&altered, &context, fixture).is_err(),
        "bind consumer accepted context drift",
    )?;
    assert_mount_shadows_rejected(&artifact, &context, fixture, &selected)?;
    let mut renamed = artifact;
    let mut identities = BTreeMap::new();
    for row in renamed["prerequisites"].as_array_mut().ok_or("prerequisites missing")? {
        let identity = row["identity"].as_str().ok_or("bind identity missing")?.to_owned();
        let next = identities.len();
        let next = u64::try_from(next)?;
        let reference = identities
            .entry(identity)
            .or_insert_with(|| (u64::MAX - next).to_string());
        row["reference"] = json!(reference);
    }
    assert_artifact(&renamed, &context, fixture)
}

fn assert_mount_shadows_rejected(
    artifact: &Value,
    context: &Value,
    fixture: Fixture,
    selected: &BindExpectation,
) -> TestResult {
    for kind in ["bind", "volume", "literal_duplicate"] {
        let mut altered = artifact.clone();
        let request = altered["requests"]
            .as_array_mut()
            .ok_or("requests missing")?
            .iter_mut()
            .find(|request| {
                request["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(&format!("containers/create?name={}", selected.identity)))
            })
            .ok_or("authored bind request missing")?;
        if kind == "literal_duplicate" {
            let binds = request["body"]["HostConfig"]["Binds"]
                .as_array_mut()
                .ok_or("literal Binds missing")?;
            let duplicate = binds.first().ok_or("literal bind missing")?.clone();
            binds.push(duplicate);
        } else {
            let mut mounts = request["body"]["HostConfig"]["Mounts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            mounts.push(json!({"Type": kind, "Source": "shadow-source", "Target": selected.target, "ReadOnly": false}));
            request["body"]["HostConfig"]["Mounts"] = json!(mounts);
        }
        require(
            assert_artifact(&altered, context, fixture).is_err(),
            "bind consumer accepted structured shadow or duplicate target",
        )?;
    }
    Ok(())
}
