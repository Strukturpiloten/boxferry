//! Contract and opt-in live conformance tests for exact Podman runtime images.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use boxferry_engine::{PlatformVersion, Severity};
use boxferry_model::{HealthcheckCommand, Identifier, RestartPolicy};
use boxferry_podman::{
    MAXIMUM_PODMAN_VERSION, MINIMUM_PODMAN_VERSION, PodmanImporter, PodmanInspectDocuments, PodmanInspectSource,
};
use boxferry_runtime::OverrideReconstruction;
use serde::Deserialize;

const EXPECTED_MINOR_LINES: &[(u64, u64)] = &[(5, 4), (5, 5), (5, 6), (5, 7), (5, 8)];
const OFFICIAL_IMAGE_PREFIX: &str = "quay.io/podman/stable:v";
const OFFICIAL_IMAGE_SUFFIX: &str = "-immutable@sha256:";

#[derive(Debug, Deserialize)]
struct RuntimeMatrix {
    schema: u64,
    current_upstream: String,
    support: Support,
    installed_current: InstalledCurrent,
    provenance: Provenance,
    runtime: Vec<RuntimeLane>,
    gap: Vec<EvidenceGap>,
}

#[derive(Debug, Deserialize)]
struct Support {
    minimum: String,
    reviewed_maximum: String,
    runtime_image_maximum: String,
}

#[derive(Debug, Deserialize)]
struct InstalledCurrent {
    version: String,
    executable_environment: String,
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
}

#[derive(Debug, Deserialize)]
struct EvidenceGap {
    version: String,
    kind: String,
    detail: String,
}

#[test]
fn runtime_matrix_is_finite_exact_and_digest_pinned() -> Result<(), String> {
    let matrix = load_matrix()?;

    if matrix.schema != 1 {
        return Err(format!("unsupported Podman runtime matrix schema {}", matrix.schema));
    }
    if version(&matrix.support.minimum)? != MINIMUM_PODMAN_VERSION {
        return Err("runtime matrix minimum differs from the decoder minimum".to_owned());
    }
    if version(&matrix.support.reviewed_maximum)? != MAXIMUM_PODMAN_VERSION {
        return Err("runtime matrix reviewed maximum differs from the decoder maximum".to_owned());
    }
    if version(matrix.current_upstream.trim_start_matches('v'))? != MAXIMUM_PODMAN_VERSION {
        return Err("upstream release signal differs from the reviewed decoder maximum".to_owned());
    }
    if version(&matrix.installed_current.version)? != MAXIMUM_PODMAN_VERSION
        || matrix.installed_current.executable_environment != "BOXFERRY_CURRENT_PODMAN"
    {
        return Err(
            "installed-current lane differs from the reviewed decoder maximum or environment contract".to_owned(),
        );
    }
    if matrix.provenance.registry != "quay.io/podman/stable"
        || !matrix.provenance.registry_tags_url.starts_with("https://")
        || !matrix.provenance.release_url.ends_with(&matrix.current_upstream)
        || matrix.provenance.checked.is_empty()
    {
        return Err("runtime matrix provenance is incomplete".to_owned());
    }

    let mut observed_minor_lines = Vec::new();
    let mut previous = None;
    for lane in &matrix.runtime {
        let lane_version = version(&lane.version)?;
        if let Some(previous) = previous {
            if lane_version <= previous {
                return Err("runtime lanes must be unique and sorted by version".to_owned());
            }
        }
        previous = Some(lane_version);
        observed_minor_lines.push((lane_version.major(), lane_version.minor()));
        validate_image_reference(lane)?;
    }
    if observed_minor_lines != EXPECTED_MINOR_LINES {
        return Err(format!(
            "runtime lanes must cover each available supported minor once: {EXPECTED_MINOR_LINES:?}"
        ));
    }
    if version(&matrix.support.runtime_image_maximum)?
        != matrix
            .runtime
            .last()
            .map(|lane| version(&lane.version))
            .transpose()?
            .ok_or("runtime matrix contains no executable lanes")?
    {
        return Err("runtime image maximum differs from the last executable lane".to_owned());
    }

    let current_gaps = matrix
        .gap
        .iter()
        .filter(|gap| version(&gap.version).ok() == Some(MAXIMUM_PODMAN_VERSION))
        .collect::<Vec<_>>();
    if current_gaps.len() != 1
        || current_gaps[0].kind != "runtime-image-unavailable"
        || current_gaps[0].detail.is_empty()
    {
        return Err("the exact current-patch runtime evidence gap must remain explicit".to_owned());
    }
    Ok(())
}

#[test]
#[ignore = "requires an explicitly selected outer container engine and privileged nested Podman"]
fn available_podman_minor_lines_decode_live_inspection() -> Result<(), String> {
    let engine = env::var_os("BOXFERRY_CONTAINER_ENGINE").ok_or(
        "set BOXFERRY_CONTAINER_ENGINE to an explicit Docker or Podman executable before running live conformance",
    )?;
    let requested_version = env::var("BOXFERRY_PODMAN_RUNTIME_VERSION").ok();
    let matrix = load_matrix()?;
    let lanes = matrix
        .runtime
        .iter()
        .filter(|lane| {
            requested_version
                .as_ref()
                .is_none_or(|requested| requested == &lane.version)
        })
        .collect::<Vec<_>>();
    if lanes.is_empty() {
        return Err(format!(
            "no executable runtime lane matches {}",
            requested_version.as_deref().unwrap_or("the matrix")
        ));
    }

    for lane in lanes {
        decode_live_lane(&engine, lane)?;
    }
    Ok(())
}

#[test]
#[ignore = "creates uniquely named resources in an explicitly selected installed Podman runtime"]
fn installed_current_podman_decodes_live_inspection() -> Result<(), String> {
    let executable = env::var_os("BOXFERRY_CURRENT_PODMAN").ok_or(
        "set BOXFERRY_CURRENT_PODMAN to an explicit Podman executable before running current-host conformance",
    )?;
    let scratch = ScratchDirectory::new()?;
    let resource_prefix = scratch.resource_prefix()?;
    let current = MAXIMUM_PODMAN_VERSION.to_string();
    run_installed_conformance(
        &executable,
        &current,
        &resource_prefix,
        &conformance_script()?,
        scratch.path(),
    )?;
    validate_live_evidence(&current, &resource_prefix, scratch.path())
}

fn validate_image_reference(lane: &RuntimeLane) -> Result<(), String> {
    let expected_prefix = format!("{OFFICIAL_IMAGE_PREFIX}{}{}", lane.version, OFFICIAL_IMAGE_SUFFIX);
    let digest = lane
        .image
        .strip_prefix(&expected_prefix)
        .ok_or_else(|| format!("Podman {} image tag does not exactly match its lane", lane.version))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Podman {} image lacks a full SHA-256 digest", lane.version));
    }
    Ok(())
}

fn decode_live_lane(engine: &std::ffi::OsStr, lane: &RuntimeLane) -> Result<(), String> {
    let scratch = ScratchDirectory::new()?;
    let script = conformance_script()?;
    run_conformance_container(engine, lane, &script, scratch.path())?;
    validate_live_evidence(&lane.version, "boxferry-conformance", scratch.path())
}

fn validate_live_evidence(version_value: &str, resource_prefix: &str, root: &Path) -> Result<(), String> {
    let actual_version = read(root, "version.txt")?;
    if actual_version.trim() != version_value {
        return Err(format!(
            "Podman image version {} differs from expected {}",
            actual_version.trim(),
            version_value
        ));
    }
    let source = PodmanInspectSource::new(
        Identifier::new(resource_prefix).map_err(|error| error.to_string())?,
        version(version_value)?,
        PodmanInspectDocuments::new(
            read(root, "containers.json")?,
            read(root, "images.json")?,
            read(root, "networks.json")?,
            read(root, "volumes.json")?,
            read(root, "pods.json")?,
        ),
    );
    validate_live_decode(version_value, resource_prefix, &source)
}

fn run_conformance_container(
    engine: &std::ffi::OsStr,
    lane: &RuntimeLane,
    script: &Path,
    output_directory: &Path,
) -> Result<(), String> {
    let script_mount = format!("{}:/boxferry/conformance.sh:ro", script.display());
    let output_mount = format!("{}:/boxferry/evidence", output_directory.display());
    let mut command = Command::new(engine);
    command
        .args([
            "run",
            "--rm",
            "--privileged",
            "--pull=missing",
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
            "/bin/bash",
            &lane.image,
            "/boxferry/conformance.sh",
            &lane.version,
            "/boxferry/evidence",
        ])
        .output()
        .map_err(|error| format!("failed to start the outer container engine: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Podman {} conformance container failed with {:?}: {}",
            lane.version,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn run_installed_conformance(
    executable: &std::ffi::OsStr,
    expected_version: &str,
    resource_prefix: &str,
    script: &Path,
    output_directory: &Path,
) -> Result<(), String> {
    let output = Command::new("/bin/bash")
        .arg(script)
        .arg(expected_version)
        .arg(output_directory)
        .arg(resource_prefix)
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to start installed Podman conformance: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "installed Podman {expected_version} conformance failed with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn validate_live_decode(
    version_value: &str,
    resource_prefix: &str,
    source: &PodmanInspectSource,
) -> Result<(), String> {
    let importer =
        PodmanImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let result = importer.decode(source);
    let snapshot = result.snapshot().ok_or_else(|| {
        let diagnostics = result
            .diagnostics()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        format!("Podman {version_value} live inspection did not decode: {diagnostics}")
    })?;
    if result
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity() == Severity::Error)
    {
        return Err(format!(
            "Podman {version_value} live inspection produced an error diagnostic"
        ));
    }
    let expected_container = format!("{resource_prefix}-web");
    let expected_network = format!("{resource_prefix}-network");
    let expected_volume = format!("{resource_prefix}-data");
    let expected_pod = format!("{resource_prefix}-pod");
    let container = snapshot
        .containers()
        .iter()
        .find(|container| container.name().as_str() == expected_container)
        .ok_or_else(|| format!("Podman {version_value} live inspection lost the test container"))?;
    let healthcheck = container
        .healthcheck()
        .ok_or_else(|| format!("Podman {version_value} live inspection lost the regular health check"))?;
    if container.user().map(boxferry_model::ProtectedString::expose) != Some("1001:1002")
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
        || !has_label(
            snapshot.images().first().and_then(|image| image.labels()),
            "com.example.image",
            "runtime-matrix",
        )
        || !snapshot
            .networks()
            .iter()
            .any(|network| network.name().as_str() == expected_network)
        || !snapshot
            .volumes()
            .iter()
            .any(|volume| volume.name().as_str() == expected_volume)
        || !snapshot.pods().iter().any(|pod| pod.name().as_str() == expected_pod)
    {
        return Err(format!(
            "Podman {version_value} live inspection lost test-owned effective state or a resource"
        ));
    }
    Ok(())
}

fn conformance_script() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/runtime/podman-inspect-conformance.sh")
        .canonicalize()
        .map_err(|error| format!("cannot locate conformance script: {error}"))
}

fn has_label(labels: Option<&[boxferry_runtime::RuntimeMetadataLabel]>, name: &str, value: &str) -> bool {
    labels.is_some_and(|labels| {
        labels
            .iter()
            .any(|label| label.name().as_str() == name && label.value().expose() == value)
    })
}

fn load_matrix() -> Result<RuntimeMatrix, String> {
    toml::from_str(include_str!("../../../tools/podman-runtime-matrix.toml"))
        .map_err(|error| format!("invalid Podman runtime matrix: {error}"))
}

fn version(value: &str) -> Result<PlatformVersion, String> {
    value
        .parse()
        .map_err(|error| format!("invalid Podman version {value}: {error}"))
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
            let path = env::temp_dir().join(format!("boxferry-podman-conformance-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("cannot create conformance directory: {error}")),
            }
        }
        Err("cannot allocate a unique conformance directory".to_owned())
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn resource_prefix(&self) -> Result<String, String> {
        self.0
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_owned)
            .ok_or_else(|| "conformance directory has no portable resource prefix".to_owned())
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
