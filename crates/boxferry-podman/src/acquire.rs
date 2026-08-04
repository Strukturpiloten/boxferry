//! Explicit read-only acquisition of selected Podman resources.

use std::{
    collections::BTreeSet,
    error::Error,
    ffi::OsStr,
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use boxferry_engine::PlatformVersion;
use boxferry_model::{Identifier, ProtectedString};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    PodmanInspectDocuments, PodmanInspectSource,
    native::{ContainerInspect, ImageInspect, NetworkInspect, PodInspect, VolumeInspect},
};

/// Podman resource family accepted by the read-only inspector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PodmanResourceKind {
    /// Existing containers.
    Container,
    /// Locally available images.
    Image,
    /// Existing Podman networks.
    Network,
    /// Existing Podman volumes.
    Volume,
    /// Existing Podman pods.
    Pod,
}

/// Explicit policy controlling which relationships acquisition may follow.
///
/// Expansion is deliberately finite. It never enumerates a resource family and does not follow
/// Podman's opaque container dependency list. Every additional selector comes from an inspect
/// response for a resource the caller selected, or from a selected pod's member list.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PodmanExpansionPolicy {
    /// Inspect only the selectors supplied in [`PodmanResourceSelection`].
    #[default]
    ExplicitOnly,
    /// Add images, networks, and named volumes referenced by selected containers.
    ContainerResources,
    /// Add selected pods' member containers, then their images, networks, and named volumes.
    PodMembersAndContainerResources,
}

impl PodmanExpansionPolicy {
    const fn includes_container_resources(self) -> bool {
        matches!(self, Self::ContainerResources | Self::PodMembersAndContainerResources)
    }

    const fn includes_pod_members(self) -> bool {
        matches!(self, Self::PodMembersAndContainerResources)
    }
}

impl PodmanResourceKind {
    const fn subcommand(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Image => "image",
            Self::Network => "network",
            Self::Volume => "volume",
            Self::Pod => "pod",
        }
    }
}

/// Invalid explicit Podman resource selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodmanSelectionError {
    kind: PodmanResourceKind,
}

impl PodmanSelectionError {
    /// Returns the family containing the invalid selector.
    #[must_use]
    pub const fn kind(&self) -> PodmanResourceKind {
        self.kind
    }
}

impl fmt::Display for PodmanSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Podman {:?} selector must be non-empty and contain no NUL byte",
            self.kind
        )
    }
}

impl Error for PodmanSelectionError {}

/// Caller-selected names or IDs for every inspect resource family.
///
/// Selectors are sensitive in debug output because opaque IDs and user-chosen resource names may
/// reveal machine or application details. An empty family is acquired as `[]` without invoking a
/// command.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct PodmanResourceSelection {
    containers: Vec<ProtectedString>,
    images: Vec<ProtectedString>,
    networks: Vec<ProtectedString>,
    volumes: Vec<ProtectedString>,
    pods: Vec<ProtectedString>,
}

impl PodmanResourceSelection {
    /// Creates an empty explicit selection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            containers: Vec::new(),
            images: Vec::new(),
            networks: Vec::new(),
            volumes: Vec::new(),
            pods: Vec::new(),
        }
    }

    /// Adds a container name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanSelectionError`] for an empty selector or a NUL byte.
    pub fn add_container(&mut self, selector: impl Into<String>) -> Result<(), PodmanSelectionError> {
        add_selector(&mut self.containers, PodmanResourceKind::Container, selector)
    }

    /// Adds an image name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanSelectionError`] for an empty selector or a NUL byte.
    pub fn add_image(&mut self, selector: impl Into<String>) -> Result<(), PodmanSelectionError> {
        add_selector(&mut self.images, PodmanResourceKind::Image, selector)
    }

    /// Adds a network name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanSelectionError`] for an empty selector or a NUL byte.
    pub fn add_network(&mut self, selector: impl Into<String>) -> Result<(), PodmanSelectionError> {
        add_selector(&mut self.networks, PodmanResourceKind::Network, selector)
    }

    /// Adds a volume name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanSelectionError`] for an empty selector or a NUL byte.
    pub fn add_volume(&mut self, selector: impl Into<String>) -> Result<(), PodmanSelectionError> {
        add_selector(&mut self.volumes, PodmanResourceKind::Volume, selector)
    }

    /// Adds a pod name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanSelectionError`] for an empty selector or a NUL byte.
    pub fn add_pod(&mut self, selector: impl Into<String>) -> Result<(), PodmanSelectionError> {
        add_selector(&mut self.pods, PodmanResourceKind::Pod, selector)
    }

    /// Returns protected container selectors in caller order.
    #[must_use]
    pub fn containers(&self) -> &[ProtectedString] {
        &self.containers
    }

    /// Returns protected image selectors in caller order.
    #[must_use]
    pub fn images(&self) -> &[ProtectedString] {
        &self.images
    }

    /// Returns protected network selectors in caller order.
    #[must_use]
    pub fn networks(&self) -> &[ProtectedString] {
        &self.networks
    }

    /// Returns protected volume selectors in caller order.
    #[must_use]
    pub fn volumes(&self) -> &[ProtectedString] {
        &self.volumes
    }

    /// Returns protected pod selectors in caller order.
    #[must_use]
    pub fn pods(&self) -> &[ProtectedString] {
        &self.pods
    }

    fn for_kind(&self, kind: PodmanResourceKind) -> &[ProtectedString] {
        match kind {
            PodmanResourceKind::Container => &self.containers,
            PodmanResourceKind::Image => &self.images,
            PodmanResourceKind::Network => &self.networks,
            PodmanResourceKind::Volume => &self.volumes,
            PodmanResourceKind::Pod => &self.pods,
        }
    }
}

impl fmt::Debug for PodmanResourceSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanResourceSelection")
            .field("containers", &self.containers.len())
            .field("images", &self.images.len())
            .field("networks", &self.networks.len())
            .field("volumes", &self.volumes.len())
            .field("pods", &self.pods.len())
            .finish()
    }
}

fn add_selector(
    destination: &mut Vec<ProtectedString>,
    kind: PodmanResourceKind,
    selector: impl Into<String>,
) -> Result<(), PodmanSelectionError> {
    let selector = selector.into();
    if selector.is_empty() || selector.contains('\0') {
        return Err(PodmanSelectionError { kind });
    }
    destination.push(ProtectedString::sensitive(selector));
    Ok(())
}

/// Closed read-only Podman inspect command passed to an executor.
///
/// Callers cannot add arbitrary flags or alternate subcommands. Selectors are separated from
/// options by `--` when the process executor builds the command.
#[derive(Clone, Eq, PartialEq)]
pub struct PodmanInspectCommand {
    executable: PathBuf,
    kind: PodmanResourceKind,
    selectors: Vec<ProtectedString>,
}

impl PodmanInspectCommand {
    fn new(executable: PathBuf, kind: PodmanResourceKind, selectors: Vec<ProtectedString>) -> Self {
        Self {
            executable,
            kind,
            selectors,
        }
    }

    /// Returns the explicit Podman executable path or name.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Returns the fixed resource family.
    #[must_use]
    pub const fn kind(&self) -> PodmanResourceKind {
        self.kind
    }

    /// Returns protected selectors in caller order.
    #[must_use]
    pub fn selectors(&self) -> &[ProtectedString] {
        &self.selectors
    }
}

impl fmt::Debug for PodmanInspectCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanInspectCommand")
            .field("executable", &self.executable)
            .field("kind", &self.kind)
            .field("selector_count", &self.selectors.len())
            .finish()
    }
}

/// Sensitive successful stdout returned by a Podman command executor.
#[derive(Clone, Eq, PartialEq)]
pub struct PodmanCommandOutput {
    stdout: ProtectedString,
}

impl PodmanCommandOutput {
    /// Creates protected stdout, primarily for replaceable executors and tests.
    #[must_use]
    pub fn new(stdout: impl Into<String>) -> Self {
        Self {
            stdout: ProtectedString::sensitive(stdout),
        }
    }

    /// Returns protected stdout. Acquisition explicitly exposes it only to construct decoder input.
    #[must_use]
    pub const fn stdout(&self) -> &ProtectedString {
        &self.stdout
    }
}

impl fmt::Debug for PodmanCommandOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanCommandOutput")
            .field("stdout", &self.stdout)
            .finish()
    }
}

/// Failure while constructing or executing a read-only Podman inspection.
#[non_exhaustive]
pub enum PodmanAcquisitionError {
    /// The explicit executable path or name was empty.
    InvalidExecutable,
    /// The operating system could not start the fixed inspect command.
    Spawn {
        /// Resource family being inspected.
        kind: PodmanResourceKind,
        /// Underlying process-spawn failure.
        source: io::Error,
    },
    /// Podman returned a non-zero process status.
    CommandFailed {
        /// Resource family being inspected.
        kind: PodmanResourceKind,
        /// Portable numeric exit status, when the platform supplied one.
        status: Option<i32>,
        /// Protected stderr for explicitly authorized troubleshooting.
        stderr: ProtectedString,
    },
    /// Podman stdout was not UTF-8 JSON text.
    NonUtf8Output {
        /// Resource family being inspected.
        kind: PodmanResourceKind,
    },
    /// An inspect response needed for relationship expansion was not valid JSON for its family.
    InvalidInspectOutput {
        /// Resource family whose response could not be interpreted.
        kind: PodmanResourceKind,
        /// One-based line reported by the JSON decoder.
        line: usize,
        /// One-based column reported by the JSON decoder.
        column: usize,
    },
}

impl PodmanAcquisitionError {
    /// Creates a process-spawn failure for a replaceable executor.
    #[must_use]
    pub const fn spawn(kind: PodmanResourceKind, source: io::Error) -> Self {
        Self::Spawn { kind, source }
    }

    /// Creates a failed-command result while protecting stderr by default.
    #[must_use]
    pub fn command_failed(kind: PodmanResourceKind, status: Option<i32>, stderr: impl Into<String>) -> Self {
        Self::CommandFailed {
            kind,
            status,
            stderr: ProtectedString::sensitive(stderr),
        }
    }

    /// Creates an output-encoding failure for a replaceable executor.
    #[must_use]
    pub const fn non_utf8_output(kind: PodmanResourceKind) -> Self {
        Self::NonUtf8Output { kind }
    }
}

impl fmt::Debug for PodmanAcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutable => formatter.write_str("PodmanAcquisitionError::InvalidExecutable"),
            Self::Spawn { kind, .. } => formatter
                .debug_struct("PodmanAcquisitionError::Spawn")
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::CommandFailed {
                kind, status, stderr, ..
            } => formatter
                .debug_struct("PodmanAcquisitionError::CommandFailed")
                .field("kind", kind)
                .field("status", status)
                .field("stderr", stderr)
                .finish(),
            Self::NonUtf8Output { kind } => formatter
                .debug_struct("PodmanAcquisitionError::NonUtf8Output")
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::InvalidInspectOutput { kind, line, column } => formatter
                .debug_struct("PodmanAcquisitionError::InvalidInspectOutput")
                .field("kind", kind)
                .field("line", line)
                .field("column", column)
                .finish(),
        }
    }
}

impl fmt::Display for PodmanAcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutable => formatter.write_str("Podman executable must not be empty"),
            Self::Spawn { kind, .. } => write!(formatter, "could not start read-only Podman {kind:?} inspection"),
            Self::CommandFailed { kind, status, .. } => {
                write!(
                    formatter,
                    "read-only Podman {kind:?} inspection failed with status {status:?}"
                )
            }
            Self::NonUtf8Output { kind, .. } => {
                write!(
                    formatter,
                    "read-only Podman {kind:?} inspection returned non-UTF-8 output"
                )
            }
            Self::InvalidInspectOutput { kind, line, column } => write!(
                formatter,
                "read-only Podman {kind:?} inspection returned invalid expansion data at line {line}, column {column}"
            ),
        }
    }
}

impl Error for PodmanAcquisitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::InvalidExecutable
            | Self::CommandFailed { .. }
            | Self::NonUtf8Output { .. }
            | Self::InvalidInspectOutput { .. } => None,
        }
    }
}

/// Replaceable execution boundary for fixed read-only Podman inspect commands.
pub trait PodmanCommandExecutor {
    /// Executes one closed command and returns protected JSON stdout.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanAcquisitionError`] for process, exit-status, or output-encoding failures.
    fn execute(&self, command: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError>;
}

/// Standard-library process executor for fixed Podman inspect commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessPodmanCommandExecutor;

impl ProcessPodmanCommandExecutor {
    /// Creates a stateless process executor.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl PodmanCommandExecutor for ProcessPodmanCommandExecutor {
    fn execute(&self, request: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError> {
        let mut command = Command::new(request.executable());
        command
            .arg(request.kind().subcommand())
            .arg("inspect")
            .arg("--")
            .args(request.selectors().iter().map(ProtectedString::expose))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command.output().map_err(|source| PodmanAcquisitionError::Spawn {
            kind: request.kind(),
            source,
        })?;
        if !output.status.success() {
            return Err(PodmanAcquisitionError::CommandFailed {
                kind: request.kind(),
                status: output.status.code(),
                stderr: ProtectedString::sensitive(String::from_utf8_lossy(&output.stderr).into_owned()),
            });
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| PodmanAcquisitionError::NonUtf8Output { kind: request.kind() })?;
        Ok(PodmanCommandOutput::new(stdout))
    }
}

/// Read-only inspector using an explicit executor, executable, and producing Podman version.
#[derive(Clone)]
pub struct PodmanInspector<E> {
    executor: E,
    executable: PathBuf,
    version: PlatformVersion,
}

impl<E> PodmanInspector<E> {
    /// Creates an inspector without searching for or invoking Podman.
    ///
    /// # Errors
    ///
    /// Returns [`PodmanAcquisitionError::InvalidExecutable`] for an empty path or name.
    pub fn new(
        executor: E,
        executable: impl Into<PathBuf>,
        version: PlatformVersion,
    ) -> Result<Self, PodmanAcquisitionError> {
        let executable = executable.into();
        if executable.as_os_str() == OsStr::new("") {
            return Err(PodmanAcquisitionError::InvalidExecutable);
        }
        Ok(Self {
            executor,
            executable,
            version,
        })
    }

    /// Returns the caller-declared version that will label acquired documents.
    #[must_use]
    pub const fn version(&self) -> PlatformVersion {
        self.version
    }

    /// Returns the explicit executable path or name without resolving `PATH`.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

impl<E: PodmanCommandExecutor> PodmanInspector<E> {
    /// Acquires all explicitly selected resources as one decoder source.
    ///
    /// Commands run sequentially in container, image, network, volume, and pod order. Empty
    /// families become `[]` without execution. The method performs only Podman's documented
    /// inspect commands and never creates, changes, starts, stops, or removes a resource.
    ///
    /// # Errors
    ///
    /// Returns the first [`PodmanAcquisitionError`] from the replaceable executor.
    pub fn inspect(
        &self,
        application_name: Identifier,
        selection: &PodmanResourceSelection,
    ) -> Result<PodmanInspectSource, PodmanAcquisitionError> {
        let containers = self.inspect_kind(PodmanResourceKind::Container, selection)?;
        let images = self.inspect_kind(PodmanResourceKind::Image, selection)?;
        let networks = self.inspect_kind(PodmanResourceKind::Network, selection)?;
        let volumes = self.inspect_kind(PodmanResourceKind::Volume, selection)?;
        let pods = self.inspect_kind(PodmanResourceKind::Pod, selection)?;
        Ok(PodmanInspectSource::new(
            application_name,
            self.version,
            PodmanInspectDocuments::new(containers, images, networks, volumes, pods),
        ))
    }

    /// Acquires selected resources and the finite relationships authorized by `policy`.
    ///
    /// [`PodmanExpansionPolicy::ContainerResources`] follows only image, network, and named-volume
    /// references found on selected containers. [`PodmanExpansionPolicy::PodMembersAndContainerResources`]
    /// first adds the members of selected pods. Bind mounts do not identify a separate Podman
    /// resource and are therefore not inspected. Container-to-container dependencies and reverse
    /// container-to-pod expansion are intentionally not followed.
    ///
    /// Expanded commands run sequentially in pod, container, image, network, and volume order.
    /// Exact selectors and returned resources are deduplicated in first-observed order. Every
    /// invoked command still contains at least one selector; the inspector never lists an ambient
    /// resource family.
    ///
    /// # Errors
    ///
    /// Returns the first executor failure, or [`PodmanAcquisitionError::InvalidInspectOutput`] if
    /// a response required to discover or deduplicate relationships is malformed.
    pub fn inspect_with_policy(
        &self,
        application_name: Identifier,
        selection: &PodmanResourceSelection,
        policy: PodmanExpansionPolicy,
    ) -> Result<PodmanInspectSource, PodmanAcquisitionError> {
        if policy == PodmanExpansionPolicy::ExplicitOnly {
            return self.inspect(application_name, selection);
        }

        let mut expanded = selection.clone();
        deduplicate_selection(&mut expanded);

        let pods = self.inspect_kind(PodmanResourceKind::Pod, &expanded)?;
        let pods = normalize_document::<PodInspect, _>(PodmanResourceKind::Pod, &pods, |pod| &pod.id)?;
        if policy.includes_pod_members() {
            let inspected_pods = parse_document::<PodInspect>(PodmanResourceKind::Pod, &pods)?;
            extend_unique(
                &mut expanded.containers,
                inspected_pods
                    .into_iter()
                    .flat_map(|pod| pod.containers)
                    .map(|container| container.id),
            );
        }

        let containers = self.inspect_kind(PodmanResourceKind::Container, &expanded)?;
        let containers =
            normalize_document::<ContainerInspect, _>(PodmanResourceKind::Container, &containers, |container| {
                &container.id
            })?;
        if policy.includes_container_resources() {
            let inspected_containers = parse_document::<ContainerInspect>(PodmanResourceKind::Container, &containers)?;
            for container in inspected_containers {
                extend_unique(&mut expanded.images, [container.image]);
                if let Some(network_settings) = container.network_settings {
                    extend_unique(&mut expanded.networks, network_settings.networks.into_keys());
                }
                extend_unique(
                    &mut expanded.volumes,
                    container
                        .mounts
                        .into_iter()
                        .filter(|mount| mount.kind == "volume")
                        .map(|mount| mount.name),
                );
            }
        }

        let images = self.inspect_kind(PodmanResourceKind::Image, &expanded)?;
        let images = normalize_document::<ImageInspect, _>(PodmanResourceKind::Image, &images, |image| &image.id)?;
        let networks = self.inspect_kind(PodmanResourceKind::Network, &expanded)?;
        let networks =
            normalize_document::<NetworkInspect, _>(PodmanResourceKind::Network, &networks, |network| &network.name)?;
        let volumes = self.inspect_kind(PodmanResourceKind::Volume, &expanded)?;
        let volumes =
            normalize_document::<VolumeInspect, _>(PodmanResourceKind::Volume, &volumes, |volume| &volume.name)?;

        Ok(PodmanInspectSource::new(
            application_name,
            self.version,
            PodmanInspectDocuments::new(containers, images, networks, volumes, pods),
        ))
    }

    fn inspect_kind(
        &self,
        kind: PodmanResourceKind,
        selection: &PodmanResourceSelection,
    ) -> Result<String, PodmanAcquisitionError> {
        let selectors = selection.for_kind(kind);
        if selectors.is_empty() {
            return Ok("[]".to_owned());
        }
        let request = PodmanInspectCommand::new(self.executable.clone(), kind, selectors.to_vec());
        self.executor
            .execute(&request)
            .map(|output| output.stdout().expose().to_owned())
    }
}

fn deduplicate_selection(selection: &mut PodmanResourceSelection) {
    deduplicate_selectors(&mut selection.containers);
    deduplicate_selectors(&mut selection.images);
    deduplicate_selectors(&mut selection.networks);
    deduplicate_selectors(&mut selection.volumes);
    deduplicate_selectors(&mut selection.pods);
}

fn deduplicate_selectors(selectors: &mut Vec<ProtectedString>) {
    let mut seen = BTreeSet::new();
    selectors.retain(|selector| seen.insert(selector.expose().to_owned()));
}

fn extend_unique(values: &mut Vec<ProtectedString>, additions: impl IntoIterator<Item = String>) {
    let mut seen = values
        .iter()
        .map(|value| value.expose().to_owned())
        .collect::<BTreeSet<_>>();
    values.extend(
        additions
            .into_iter()
            .filter(|value| !value.is_empty() && seen.insert(value.clone()))
            .map(ProtectedString::sensitive),
    );
}

fn parse_document<T: DeserializeOwned>(
    kind: PodmanResourceKind,
    document: &str,
) -> Result<Vec<T>, PodmanAcquisitionError> {
    serde_json::from_str(document).map_err(|error| PodmanAcquisitionError::InvalidInspectOutput {
        kind,
        line: error.line(),
        column: error.column(),
    })
}

fn normalize_document<T: DeserializeOwned, F: Fn(&T) -> &str>(
    kind: PodmanResourceKind,
    document: &str,
    identity: F,
) -> Result<String, PodmanAcquisitionError> {
    let typed = parse_document::<T>(kind, document)?;
    let values = parse_document::<Value>(kind, document)?;
    let mut seen = BTreeSet::new();
    let retained = typed
        .iter()
        .zip(values)
        .filter_map(|(resource, value)| {
            let identity = identity(resource);
            (identity.is_empty() || seen.insert(identity.to_owned())).then_some(value)
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&retained).map_err(|error| PodmanAcquisitionError::InvalidInspectOutput {
        kind,
        line: error.line(),
        column: error.column(),
    })
}

impl<E> fmt::Debug for PodmanInspector<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanInspector")
            .field("executable", &self.executable)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
