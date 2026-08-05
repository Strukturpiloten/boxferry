//! Contract and opt-in live conformance tests for an exact nested Docker Engine.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use boxferry_docker::{
    DockerApiVersion, DockerImporter, DockerInspectDocuments, DockerInspectSource, MAXIMUM_DOCKER_API_VERSION,
    MINIMUM_DOCKER_API_VERSION,
};
use boxferry_engine::Severity;
use boxferry_model::{HealthcheckCommand, Identifier, RestartPolicy};
use boxferry_runtime::OverrideReconstruction;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RuntimeMatrix {
    schema: u64,
    current_upstream: String,
    support: Support,
    provenance: Provenance,
    runtime: RuntimeLane,
}

#[derive(Debug, Deserialize)]
struct Support {
    minimum_api: String,
    reviewed_maximum_api: String,
}

#[derive(Debug, Deserialize)]
struct Provenance {
    registry: String,
    registry_tags_url: String,
    release_url: String,
    checked: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeLane {
    version: String,
    image: String,
    api_versions: Vec<String>,
}

#[test]
fn runtime_matrix_is_finite_exact_and_digest_pinned() -> Result<(), String> {
    let matrix = load_matrix()?;

    if matrix.schema != 1 {
        return Err(format!("unsupported Docker runtime matrix schema {}", matrix.schema));
    }
    if api_version(&matrix.support.minimum_api)? != MINIMUM_DOCKER_API_VERSION
        || api_version(&matrix.support.reviewed_maximum_api)? != MAXIMUM_DOCKER_API_VERSION
    {
        return Err("Docker runtime matrix API bounds differ from decoder bounds".to_owned());
    }
    let api_versions = matrix
        .runtime
        .api_versions
        .iter()
        .map(|value| api_version(value))
        .collect::<Result<Vec<_>, _>>()?;
    if api_versions != [MINIMUM_DOCKER_API_VERSION, MAXIMUM_DOCKER_API_VERSION] {
        return Err("Docker runtime lane must exercise the reviewed API floor and ceiling".to_owned());
    }
    if matrix.current_upstream != format!("docker-v{}", matrix.runtime.version) {
        return Err("Docker upstream signal differs from the executable runtime lane".to_owned());
    }
    validate_image_reference(&matrix.runtime)?;
    if matrix.provenance.registry != "docker.io/library/docker"
        || !matrix.provenance.registry_tags_url.starts_with("https://")
        || !matrix.provenance.release_url.ends_with("#2971")
        || matrix.provenance.checked.is_empty()
    {
        return Err("Docker runtime matrix provenance is incomplete".to_owned());
    }
    Ok(())
}

#[test]
#[ignore = "requires an explicitly selected outer engine and a privileged nested Docker daemon"]
fn current_docker_engine_decodes_reviewed_api_responses() -> Result<(), String> {
    let engine = env::var_os("BOXFERRY_CONTAINER_ENGINE")
        .ok_or("set BOXFERRY_CONTAINER_ENGINE to an explicit Docker or Podman executable before live conformance")?;
    let matrix = load_matrix()?;
    let scratch = ScratchDirectory::new()?;
    let resource_prefix = scratch.resource_prefix()?;

    run_conformance_container(&engine, &matrix.runtime, &resource_prefix, scratch.path())?;
    validate_live_evidence(&matrix, &resource_prefix, scratch.path())
}

fn validate_image_reference(lane: &RuntimeLane) -> Result<(), String> {
    let expected_prefix = format!("docker.io/library/docker:{}-dind@sha256:", lane.version);
    let digest = lane
        .image
        .strip_prefix(&expected_prefix)
        .ok_or("Docker dind image tag does not exactly match its runtime version")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Docker dind image lacks a full SHA-256 digest".to_owned());
    }
    Ok(())
}

fn run_conformance_container(
    engine: &std::ffi::OsStr,
    lane: &RuntimeLane,
    resource_prefix: &str,
    output_directory: &Path,
) -> Result<(), String> {
    let script = conformance_script()?;
    let script_mount = format!("{}:/boxferry/conformance.sh:ro", script.display());
    let output_mount = format!("{}:/boxferry/evidence", output_directory.display());
    let mut command = Command::new(engine);
    command
        .args([
            "run",
            "--rm",
            "--privileged",
            "--pull=missing",
            "--env",
            "DOCKER_TLS_CERTDIR=",
            "--volume",
            &script_mount,
            "--volume",
            &output_mount,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if Path::new(engine)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| name == "podman")
    {
        command.args(["--security-opt", "label=disable"]);
    }
    let output = command
        .args([
            "--entrypoint",
            "/bin/sh",
            &lane.image,
            "/boxferry/conformance.sh",
            &lane.version,
            "/boxferry/evidence",
            resource_prefix,
        ])
        .args(&lane.api_versions)
        .output()
        .map_err(|error| format!("failed to start the outer container engine: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Docker {} conformance container failed with {:?}: {}",
            lane.version,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn validate_live_evidence(matrix: &RuntimeMatrix, resource_prefix: &str, root: &Path) -> Result<(), String> {
    let engine_version = read(root, "engine-version.txt")?;
    if engine_version.trim() != matrix.runtime.version {
        return Err(format!(
            "Docker Engine version {} differs from expected {}",
            engine_version.trim(),
            matrix.runtime.version
        ));
    }
    if api_version(read(root, "engine-api.txt")?.trim())? != MAXIMUM_DOCKER_API_VERSION
        || api_version(read(root, "engine-minimum-api.txt")?.trim())? != MINIMUM_DOCKER_API_VERSION
    {
        return Err("nested Docker Engine API range differs from the reviewed range".to_owned());
    }
    for version in &matrix.runtime.api_versions {
        validate_live_decode(
            api_version(version)?,
            resource_prefix,
            &root.join(format!("api-{version}")),
        )?;
    }
    Ok(())
}

fn validate_live_decode(version: DockerApiVersion, resource_prefix: &str, root: &Path) -> Result<(), String> {
    let source = DockerInspectSource::new(
        Identifier::new(resource_prefix).map_err(|error| error.to_string())?,
        version,
        DockerInspectDocuments::new(
            read(root, "containers.json")?,
            read(root, "images.json")?,
            read(root, "networks.json")?,
            read(root, "volumes.json")?,
        ),
    );
    let importer =
        DockerImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let result = importer.decode(&source);
    let snapshot = result.snapshot().ok_or_else(|| {
        let diagnostics = result
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        format!("Docker API {version} live inspection did not decode: {diagnostics}")
    })?;
    if result
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity() == Severity::Error)
    {
        return Err(format!(
            "Docker API {version} live inspection produced an error diagnostic"
        ));
    }
    let expected_container = format!("{resource_prefix}-web");
    let expected_network = format!("{resource_prefix}-network");
    let expected_volume = format!("{resource_prefix}-data");
    let container = snapshot
        .containers()
        .iter()
        .find(|container| container.name().as_str() == expected_container)
        .ok_or_else(|| format!("Docker API {version} inspection lost the test container"))?;
    let healthcheck = container
        .healthcheck()
        .ok_or_else(|| format!("Docker API {version} inspection lost the regular health check"))?;
    if container.image_source_id().is_none()
        || container.user().map(boxferry_model::ProtectedString::expose) != Some("1001:1002")
        || container
            .working_directory()
            .map(boxferry_model::ProtectedString::expose)
            != Some("/srv/runtime")
        || container.read_only_root_filesystem() != Some(true)
        || !matches!(
            container.restart_policy(),
            Some(RestartPolicy::OnFailure {
                maximum_retries: Some(value)
            }) if value.get() == 4
        )
        || !has_label(container.labels(), "com.example.boxferry", "runtime-matrix")
        || !has_label(container.labels(), "com.example.image", "runtime-matrix")
        || !matches!(healthcheck.command(), Some(HealthcheckCommand::Shell(command)) if command.expose() == "/bin/true")
        || healthcheck.interval().map(boxferry_model::HealthcheckDuration::as_str) != Some("30s")
        || healthcheck.timeout().map(boxferry_model::HealthcheckDuration::as_str) != Some("2s")
        || healthcheck.retries().map(boxferry_model::HealthcheckRetries::as_str) != Some("4")
        || healthcheck
            .start_period()
            .map(boxferry_model::HealthcheckDuration::as_str)
            != Some("5s")
        || healthcheck.start_interval().is_some()
        || snapshot
            .images()
            .first()
            .and_then(|image| image.user())
            .map(boxferry_model::ProtectedString::expose)
            != Some("1000:1000")
        || snapshot
            .images()
            .first()
            .and_then(|image| image.working_directory())
            .map(boxferry_model::ProtectedString::expose)
            != Some("/srv/image")
        || !has_label(
            snapshot.images().first().and_then(|image| image.labels()),
            "com.example.image",
            "runtime-matrix",
        )
        || !container.networks().iter().any(|network| {
            network.network().as_str() == expected_network && network.aliases().iter().any(|alias| alias == "web")
        })
        || !snapshot
            .networks()
            .iter()
            .any(|network| network.name().as_str() == expected_network)
        || !snapshot
            .volumes()
            .iter()
            .any(|volume| volume.name().as_str() == expected_volume)
    {
        return Err(format!(
            "Docker API {version} inspection lost test-owned effective state or a relationship"
        ));
    }
    Ok(())
}

fn conformance_script() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/runtime/docker-inspect-conformance.sh")
        .canonicalize()
        .map_err(|error| format!("cannot locate Docker conformance script: {error}"))
}

fn has_label(labels: Option<&[boxferry_runtime::RuntimeMetadataLabel]>, name: &str, value: &str) -> bool {
    labels.is_some_and(|labels| {
        labels
            .iter()
            .any(|label| label.name().as_str() == name && label.value().expose() == value)
    })
}

fn load_matrix() -> Result<RuntimeMatrix, String> {
    toml::from_str(include_str!("../../../tools/docker-runtime-matrix.toml"))
        .map_err(|error| format!("invalid Docker runtime matrix: {error}"))
}

fn api_version(value: &str) -> Result<DockerApiVersion, String> {
    value
        .parse()
        .map_err(|error| format!("invalid Docker Engine API version {value}: {error}"))
}

fn read(root: &Path, name: &str) -> Result<String, String> {
    fs::read_to_string(root.join(name)).map_err(|error| format!("cannot read live {name}: {error}"))
}

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new() -> Result<Self, String> {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!("boxferry-docker-conformance-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("cannot create Docker conformance directory: {error}")),
            }
        }
        Err("cannot allocate a unique Docker conformance directory".to_owned())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn resource_prefix(&self) -> Result<String, String> {
        self.0
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_owned)
            .ok_or_else(|| "Docker conformance directory has no portable resource prefix".to_owned())
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
