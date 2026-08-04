//! Explicit read-only acquisition of selected Docker resources.

use std::{
    collections::BTreeSet,
    error::Error,
    ffi::OsStr,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use boxferry_model::{Identifier, ProtectedString};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    DockerApiVersion, DockerInspectDocuments, DockerInspectSource, MAXIMUM_DOCKER_API_VERSION,
    MINIMUM_DOCKER_API_VERSION,
    native::{ContainerInspect, ImageInspect, NetworkInspect, VolumeInspect},
};

/// Docker resource family accepted by the read-only inspector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerResourceKind {
    /// Existing containers.
    Container,
    /// Locally available images.
    Image,
    /// Existing Docker networks.
    Network,
    /// Existing Docker volumes.
    Volume,
}

impl DockerResourceKind {
    const fn subcommand(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Image => "image",
            Self::Network => "network",
            Self::Volume => "volume",
        }
    }
}

/// Explicit policy controlling which relationships acquisition may follow.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerExpansionPolicy {
    /// Inspect only selectors supplied in [`DockerResourceSelection`].
    #[default]
    ExplicitOnly,
    /// Add images, networks, and named volumes referenced by selected containers.
    ContainerResources,
}

/// Invalid explicit Docker resource selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerSelectionError {
    kind: DockerResourceKind,
}

impl DockerSelectionError {
    /// Returns the family containing the invalid selector.
    #[must_use]
    pub const fn kind(&self) -> DockerResourceKind {
        self.kind
    }
}

impl fmt::Display for DockerSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Docker {:?} selector must be non-empty and contain no NUL byte",
            self.kind
        )
    }
}

impl Error for DockerSelectionError {}

/// Caller-selected names or IDs for every supported inspect resource family.
///
/// Selectors are protected in debug output. An empty family becomes `[]` without invoking the
/// executor, so acquisition cannot accidentally enumerate ambient daemon resources.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct DockerResourceSelection {
    containers: Vec<ProtectedString>,
    images: Vec<ProtectedString>,
    networks: Vec<ProtectedString>,
    volumes: Vec<ProtectedString>,
}

impl DockerResourceSelection {
    /// Creates an empty explicit selection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            containers: Vec::new(),
            images: Vec::new(),
            networks: Vec::new(),
            volumes: Vec::new(),
        }
    }

    /// Adds a container name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`DockerSelectionError`] for an empty selector or a NUL byte.
    pub fn add_container(&mut self, selector: impl Into<String>) -> Result<(), DockerSelectionError> {
        add_selector(&mut self.containers, DockerResourceKind::Container, selector)
    }

    /// Adds an image name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`DockerSelectionError`] for an empty selector or a NUL byte.
    pub fn add_image(&mut self, selector: impl Into<String>) -> Result<(), DockerSelectionError> {
        add_selector(&mut self.images, DockerResourceKind::Image, selector)
    }

    /// Adds a network name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`DockerSelectionError`] for an empty selector or a NUL byte.
    pub fn add_network(&mut self, selector: impl Into<String>) -> Result<(), DockerSelectionError> {
        add_selector(&mut self.networks, DockerResourceKind::Network, selector)
    }

    /// Adds a volume name or ID.
    ///
    /// # Errors
    ///
    /// Returns [`DockerSelectionError`] for an empty selector or a NUL byte.
    pub fn add_volume(&mut self, selector: impl Into<String>) -> Result<(), DockerSelectionError> {
        add_selector(&mut self.volumes, DockerResourceKind::Volume, selector)
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

    fn for_kind(&self, kind: DockerResourceKind) -> &[ProtectedString] {
        match kind {
            DockerResourceKind::Container => &self.containers,
            DockerResourceKind::Image => &self.images,
            DockerResourceKind::Network => &self.networks,
            DockerResourceKind::Volume => &self.volumes,
        }
    }
}

impl fmt::Debug for DockerResourceSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerResourceSelection")
            .field("containers", &self.containers.len())
            .field("images", &self.images.len())
            .field("networks", &self.networks.len())
            .field("volumes", &self.volumes.len())
            .finish()
    }
}

fn add_selector(
    destination: &mut Vec<ProtectedString>,
    kind: DockerResourceKind,
    selector: impl Into<String>,
) -> Result<(), DockerSelectionError> {
    let selector = selector.into();
    if selector.is_empty() || selector.contains('\0') {
        return Err(DockerSelectionError { kind });
    }
    destination.push(ProtectedString::sensitive(selector));
    Ok(())
}

/// Closed read-only Docker inspect command passed to an executor.
///
/// Callers cannot add arbitrary flags or alternate subcommands. The daemon endpoint and Engine
/// API version are mandatory, and selectors are separated from options with `--`.
#[derive(Clone, Eq, PartialEq)]
pub struct DockerInspectCommand {
    executable: PathBuf,
    endpoint: ProtectedString,
    api_version: DockerApiVersion,
    kind: DockerResourceKind,
    selectors: Vec<ProtectedString>,
}

impl DockerInspectCommand {
    fn new(
        executable: PathBuf,
        endpoint: ProtectedString,
        api_version: DockerApiVersion,
        kind: DockerResourceKind,
        selectors: Vec<ProtectedString>,
    ) -> Self {
        Self {
            executable,
            endpoint,
            api_version,
            kind,
            selectors,
        }
    }

    /// Returns the explicit Docker executable path or name.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Returns the protected explicit daemon endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &ProtectedString {
        &self.endpoint
    }

    /// Returns the exact Engine API version forced for the command.
    #[must_use]
    pub const fn api_version(&self) -> DockerApiVersion {
        self.api_version
    }

    /// Returns the fixed resource family.
    #[must_use]
    pub const fn kind(&self) -> DockerResourceKind {
        self.kind
    }

    /// Returns protected selectors in caller order.
    #[must_use]
    pub fn selectors(&self) -> &[ProtectedString] {
        &self.selectors
    }
}

impl fmt::Debug for DockerInspectCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerInspectCommand")
            .field("executable", &self.executable)
            .field("endpoint", &self.endpoint)
            .field("api_version", &self.api_version)
            .field("kind", &self.kind)
            .field("selector_count", &self.selectors.len())
            .finish()
    }
}

/// Sensitive successful stdout returned by a Docker command executor.
#[derive(Clone, Eq, PartialEq)]
pub struct DockerCommandOutput {
    stdout: ProtectedString,
}

impl DockerCommandOutput {
    /// Creates protected stdout, primarily for replaceable executors and tests.
    #[must_use]
    pub fn new(stdout: impl Into<String>) -> Self {
        Self {
            stdout: ProtectedString::sensitive(stdout),
        }
    }

    /// Returns protected stdout. Acquisition exposes it only while building decoder input.
    #[must_use]
    pub const fn stdout(&self) -> &ProtectedString {
        &self.stdout
    }
}

impl fmt::Debug for DockerCommandOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerCommandOutput")
            .field("stdout", &self.stdout)
            .finish()
    }
}

/// Failure while constructing or executing a read-only Docker inspection.
#[non_exhaustive]
pub enum DockerAcquisitionError {
    /// The explicit executable path or name was empty.
    InvalidExecutable,
    /// The explicit daemon endpoint was empty or contained a NUL byte.
    InvalidEndpoint,
    /// The requested Engine API version is outside the reviewed acquisition range.
    UnsupportedApiVersion {
        /// Requested exact Engine API version.
        version: DockerApiVersion,
        /// Inclusive reviewed floor.
        minimum: DockerApiVersion,
        /// Inclusive reviewed ceiling.
        maximum: DockerApiVersion,
    },
    /// An isolated empty Docker CLI configuration directory could not be prepared.
    ClientConfiguration {
        /// Resource family whose command could not be isolated.
        kind: DockerResourceKind,
        /// Underlying temporary-directory failure.
        source: io::Error,
    },
    /// The operating system could not start the fixed inspect command.
    Spawn {
        /// Resource family being inspected.
        kind: DockerResourceKind,
        /// Underlying process-spawn failure.
        source: io::Error,
    },
    /// Docker returned a non-zero process status.
    CommandFailed {
        /// Resource family being inspected.
        kind: DockerResourceKind,
        /// Portable numeric exit status, when supplied by the platform.
        status: Option<i32>,
        /// Protected stderr for explicitly authorized troubleshooting.
        stderr: ProtectedString,
    },
    /// Docker stdout was not UTF-8 JSON text.
    NonUtf8Output {
        /// Resource family being inspected.
        kind: DockerResourceKind,
    },
    /// An inspect response needed for relationship expansion was not valid JSON for its family.
    InvalidInspectOutput {
        /// Resource family whose response could not be interpreted.
        kind: DockerResourceKind,
        /// One-based line reported by the JSON decoder.
        line: usize,
        /// One-based column reported by the JSON decoder.
        column: usize,
    },
}

impl DockerAcquisitionError {
    /// Creates a process-spawn failure for a replaceable executor.
    #[must_use]
    pub const fn spawn(kind: DockerResourceKind, source: io::Error) -> Self {
        Self::Spawn { kind, source }
    }

    /// Creates a failed-command result while protecting stderr by default.
    #[must_use]
    pub fn command_failed(kind: DockerResourceKind, status: Option<i32>, stderr: impl Into<String>) -> Self {
        Self::CommandFailed {
            kind,
            status,
            stderr: ProtectedString::sensitive(stderr),
        }
    }

    /// Creates an output-encoding failure for a replaceable executor.
    #[must_use]
    pub const fn non_utf8_output(kind: DockerResourceKind) -> Self {
        Self::NonUtf8Output { kind }
    }
}

impl fmt::Debug for DockerAcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutable => formatter.write_str("DockerAcquisitionError::InvalidExecutable"),
            Self::InvalidEndpoint => formatter.write_str("DockerAcquisitionError::InvalidEndpoint"),
            Self::UnsupportedApiVersion {
                version,
                minimum,
                maximum,
            } => formatter
                .debug_struct("DockerAcquisitionError::UnsupportedApiVersion")
                .field("version", version)
                .field("minimum", minimum)
                .field("maximum", maximum)
                .finish(),
            Self::ClientConfiguration { kind, .. } => formatter
                .debug_struct("DockerAcquisitionError::ClientConfiguration")
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::Spawn { kind, .. } => formatter
                .debug_struct("DockerAcquisitionError::Spawn")
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::CommandFailed {
                kind, status, stderr, ..
            } => formatter
                .debug_struct("DockerAcquisitionError::CommandFailed")
                .field("kind", kind)
                .field("status", status)
                .field("stderr", stderr)
                .finish(),
            Self::NonUtf8Output { kind } => formatter
                .debug_struct("DockerAcquisitionError::NonUtf8Output")
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::InvalidInspectOutput { kind, line, column } => formatter
                .debug_struct("DockerAcquisitionError::InvalidInspectOutput")
                .field("kind", kind)
                .field("line", line)
                .field("column", column)
                .finish(),
        }
    }
}

impl fmt::Display for DockerAcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExecutable => formatter.write_str("Docker executable must not be empty"),
            Self::InvalidEndpoint => {
                formatter.write_str("Docker daemon endpoint must be non-empty and contain no NUL byte")
            }
            Self::UnsupportedApiVersion {
                version,
                minimum,
                maximum,
            } => write!(
                formatter,
                "Docker Engine API version {version} is outside the reviewed acquisition range {minimum} through {maximum}"
            ),
            Self::ClientConfiguration { kind, .. } => {
                write!(formatter, "could not isolate Docker {kind:?} client configuration")
            }
            Self::Spawn { kind, .. } => write!(formatter, "could not start read-only Docker {kind:?} inspection"),
            Self::CommandFailed { kind, status, .. } => {
                write!(
                    formatter,
                    "read-only Docker {kind:?} inspection failed with status {status:?}"
                )
            }
            Self::NonUtf8Output { kind } => {
                write!(
                    formatter,
                    "read-only Docker {kind:?} inspection returned non-UTF-8 output"
                )
            }
            Self::InvalidInspectOutput { kind, line, column } => write!(
                formatter,
                "read-only Docker {kind:?} inspection returned invalid expansion data at line {line}, column {column}"
            ),
        }
    }
}

impl Error for DockerAcquisitionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::ClientConfiguration { source, .. } => Some(source),
            Self::InvalidExecutable
            | Self::InvalidEndpoint
            | Self::UnsupportedApiVersion { .. }
            | Self::CommandFailed { .. }
            | Self::NonUtf8Output { .. }
            | Self::InvalidInspectOutput { .. } => None,
        }
    }
}

/// Replaceable execution boundary for fixed read-only Docker inspect commands.
pub trait DockerCommandExecutor {
    /// Executes one closed command and returns protected JSON stdout.
    ///
    /// # Errors
    ///
    /// Returns [`DockerAcquisitionError`] for process, status, or output-encoding failures.
    fn execute(&self, command: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError>;
}

/// Standard-library process executor for fixed Docker inspect commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessDockerCommandExecutor;

impl ProcessDockerCommandExecutor {
    /// Creates a stateless process executor.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DockerCommandExecutor for ProcessDockerCommandExecutor {
    fn execute(&self, request: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError> {
        let client_config =
            TemporaryClientConfig::new().map_err(|source| DockerAcquisitionError::ClientConfiguration {
                kind: request.kind(),
                source,
            })?;
        let mut command = Command::new(request.executable());
        command
            .arg("--config")
            .arg(client_config.path())
            .arg("--host")
            .arg(request.endpoint().expose())
            .arg(request.kind().subcommand())
            .arg("inspect")
            .arg("--")
            .args(request.selectors().iter().map(ProtectedString::expose))
            .env("DOCKER_API_VERSION", request.api_version().to_string())
            .env_remove("DOCKER_HOST")
            .env_remove("DOCKER_CONTEXT")
            .env_remove("DOCKER_CUSTOM_HEADERS")
            .env_remove("DOCKER_TLS")
            .env_remove("DOCKER_TLS_VERIFY")
            .env_remove("DOCKER_CERT_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command.output().map_err(|source| DockerAcquisitionError::Spawn {
            kind: request.kind(),
            source,
        })?;
        if !output.status.success() {
            return Err(DockerAcquisitionError::CommandFailed {
                kind: request.kind(),
                status: output.status.code(),
                stderr: ProtectedString::sensitive(String::from_utf8_lossy(&output.stderr).into_owned()),
            });
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| DockerAcquisitionError::NonUtf8Output { kind: request.kind() })?;
        Ok(DockerCommandOutput::new(stdout))
    }
}

struct TemporaryClientConfig(PathBuf);

impl TemporaryClientConfig {
    fn new() -> Result<Self, io::Error> {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("boxferry-docker-config-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate an isolated Docker client configuration directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryClientConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Read-only inspector with explicit execution, endpoint, and API-version boundaries.
#[derive(Clone)]
pub struct DockerInspector<E> {
    executor: E,
    executable: PathBuf,
    endpoint: ProtectedString,
    api_version: DockerApiVersion,
}

impl<E> DockerInspector<E> {
    /// Creates an inspector without searching for or invoking Docker.
    ///
    /// # Errors
    ///
    /// Returns [`DockerAcquisitionError::InvalidExecutable`] for an empty executable,
    /// [`DockerAcquisitionError::InvalidEndpoint`] for an empty endpoint or NUL byte, or
    /// [`DockerAcquisitionError::UnsupportedApiVersion`] when the requested API version is
    /// outside `BoxFerry`'s supported range.
    pub fn new(
        executor: E,
        executable: impl Into<PathBuf>,
        endpoint: impl Into<String>,
        api_version: DockerApiVersion,
    ) -> Result<Self, DockerAcquisitionError> {
        let executable = executable.into();
        if executable.as_os_str() == OsStr::new("") {
            return Err(DockerAcquisitionError::InvalidExecutable);
        }
        let endpoint = endpoint.into();
        if endpoint.is_empty() || endpoint.contains('\0') {
            return Err(DockerAcquisitionError::InvalidEndpoint);
        }
        if api_version < MINIMUM_DOCKER_API_VERSION || api_version > MAXIMUM_DOCKER_API_VERSION {
            return Err(DockerAcquisitionError::UnsupportedApiVersion {
                version: api_version,
                minimum: MINIMUM_DOCKER_API_VERSION,
                maximum: MAXIMUM_DOCKER_API_VERSION,
            });
        }
        Ok(Self {
            executor,
            executable,
            endpoint: ProtectedString::sensitive(endpoint),
            api_version,
        })
    }

    /// Returns the exact Engine API version forced for acquired documents.
    #[must_use]
    pub const fn api_version(&self) -> DockerApiVersion {
        self.api_version
    }

    /// Returns the explicit executable path or name without resolving `PATH`.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Returns the protected explicit daemon endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &ProtectedString {
        &self.endpoint
    }
}

impl<E: DockerCommandExecutor> DockerInspector<E> {
    /// Acquires all explicitly selected resources as one decoder source.
    ///
    /// Commands run sequentially in container, image, network, and volume order. Empty families
    /// become `[]` without execution. No command enumerates ambient daemon resources.
    ///
    /// # Errors
    ///
    /// Returns the first [`DockerAcquisitionError`] from the replaceable executor.
    pub fn inspect(
        &self,
        application_name: Identifier,
        selection: &DockerResourceSelection,
    ) -> Result<DockerInspectSource, DockerAcquisitionError> {
        let containers = self.inspect_kind(DockerResourceKind::Container, selection)?;
        let images = self.inspect_kind(DockerResourceKind::Image, selection)?;
        let networks = self.inspect_kind(DockerResourceKind::Network, selection)?;
        let volumes = self.inspect_kind(DockerResourceKind::Volume, selection)?;
        Ok(DockerInspectSource::new(
            application_name,
            self.api_version,
            DockerInspectDocuments::new(containers, images, networks, volumes),
        ))
    }

    /// Acquires selected resources and finite relationships authorized by `policy`.
    ///
    /// [`DockerExpansionPolicy::ContainerResources`] follows only image IDs, network names, and
    /// named-volume references present in selected container responses. Bind paths are never
    /// interpreted as resource selectors. Exact selectors and returned resources are deduplicated
    /// in first-observed order, and no invoked command has an empty selector set.
    ///
    /// # Errors
    ///
    /// Returns the first executor failure, or [`DockerAcquisitionError::InvalidInspectOutput`] if
    /// a response required for relationship discovery or deduplication is malformed.
    pub fn inspect_with_policy(
        &self,
        application_name: Identifier,
        selection: &DockerResourceSelection,
        policy: DockerExpansionPolicy,
    ) -> Result<DockerInspectSource, DockerAcquisitionError> {
        if policy == DockerExpansionPolicy::ExplicitOnly {
            return self.inspect(application_name, selection);
        }

        let mut expanded = selection.clone();
        deduplicate_selection(&mut expanded);

        let containers = self.inspect_kind(DockerResourceKind::Container, &expanded)?;
        let containers =
            normalize_document::<ContainerInspect, _>(DockerResourceKind::Container, &containers, |container| {
                &container.id
            })?;
        let inspected_containers = parse_document::<ContainerInspect>(DockerResourceKind::Container, &containers)?;
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

        let images = self.inspect_kind(DockerResourceKind::Image, &expanded)?;
        let images = normalize_document::<ImageInspect, _>(DockerResourceKind::Image, &images, |image| &image.id)?;
        let networks = self.inspect_kind(DockerResourceKind::Network, &expanded)?;
        let networks =
            normalize_document::<NetworkInspect, _>(DockerResourceKind::Network, &networks, |network| &network.name)?;
        let volumes = self.inspect_kind(DockerResourceKind::Volume, &expanded)?;
        let volumes =
            normalize_document::<VolumeInspect, _>(DockerResourceKind::Volume, &volumes, |volume| &volume.name)?;

        Ok(DockerInspectSource::new(
            application_name,
            self.api_version,
            DockerInspectDocuments::new(containers, images, networks, volumes),
        ))
    }

    fn inspect_kind(
        &self,
        kind: DockerResourceKind,
        selection: &DockerResourceSelection,
    ) -> Result<String, DockerAcquisitionError> {
        let selectors = selection.for_kind(kind);
        if selectors.is_empty() {
            return Ok("[]".to_owned());
        }
        let request = DockerInspectCommand::new(
            self.executable.clone(),
            self.endpoint.clone(),
            self.api_version,
            kind,
            selectors.to_vec(),
        );
        self.executor
            .execute(&request)
            .map(|output| output.stdout().expose().to_owned())
    }
}

fn deduplicate_selection(selection: &mut DockerResourceSelection) {
    deduplicate_selectors(&mut selection.containers);
    deduplicate_selectors(&mut selection.images);
    deduplicate_selectors(&mut selection.networks);
    deduplicate_selectors(&mut selection.volumes);
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
    kind: DockerResourceKind,
    document: &str,
) -> Result<Vec<T>, DockerAcquisitionError> {
    serde_json::from_str(document).map_err(|error| DockerAcquisitionError::InvalidInspectOutput {
        kind,
        line: error.line(),
        column: error.column(),
    })
}

fn normalize_document<T: DeserializeOwned, F: Fn(&T) -> &str>(
    kind: DockerResourceKind,
    document: &str,
    identity: F,
) -> Result<String, DockerAcquisitionError> {
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
    serde_json::to_string(&retained).map_err(|error| DockerAcquisitionError::InvalidInspectOutput {
        kind,
        line: error.line(),
        column: error.column(),
    })
}

impl<E> fmt::Debug for DockerInspector<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerInspector")
            .field("executable", &self.executable)
            .field("endpoint", &self.endpoint)
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}
