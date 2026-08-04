//! Runtime-independent effective-state observations.

use std::{error::Error, fmt};

use boxferry_model::{Identifier, ImageReference, Mount, NetworkAttachment, Port, ProtectedString, SourceId};

/// Container implementation from which a snapshot was obtained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuntimeImplementation {
    /// Docker Engine or a compatible Docker API implementation.
    Docker,
    /// Podman or the Podman service API.
    Podman,
    /// Another explicitly named runtime implementation.
    Other(Identifier),
}

impl RuntimeImplementation {
    /// Returns the stable implementation name used in diagnostics.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Other(name) => name.as_str(),
        }
    }
}

/// Effective command arguments observed from a container or image.
///
/// Runtime inspection cannot recover whether the original definition used shell or exec syntax.
/// Arguments are therefore retained as an effective exec vector and are sensitive by default.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EffectiveCommand {
    /// Effective argument vector, redacted from debug output.
    Exec(Vec<ProtectedString>),
    /// The runtime reported an explicitly empty command.
    Empty,
}

impl EffectiveCommand {
    /// Creates an effective argument vector whose values are sensitive by default.
    #[must_use]
    pub fn exec<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Exec(
            arguments
                .into_iter()
                .map(|argument| ProtectedString::sensitive(argument.into()))
                .collect(),
        )
    }

    /// Returns the effective argument vector, or `None` for an explicit empty command.
    #[must_use]
    pub fn arguments(&self) -> Option<&[ProtectedString]> {
        match self {
            Self::Exec(arguments) => Some(arguments),
            Self::Empty => None,
        }
    }
}

/// One effective runtime environment value.
///
/// Values from inspection are sensitive by default because environment data frequently contains
/// credentials. Callers must explicitly expose the value when mapping or rendering authorized
/// output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeEnvironmentVariable {
    name: Identifier,
    value: ProtectedString,
}

impl RuntimeEnvironmentVariable {
    /// Creates a sensitive effective environment value.
    #[must_use]
    pub fn new(name: Identifier, value: impl Into<String>) -> Self {
        Self {
            name,
            value: ProtectedString::sensitive(value),
        }
    }

    /// Returns the environment-variable name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the protected effective value.
    #[must_use]
    pub const fn value(&self) -> &ProtectedString {
        &self.value
    }
}

/// Optional runtime creation-command evidence.
///
/// Arguments are sensitive by default and never override contradictory effective inspection
/// fields. The evidence can contribute provenance without being present at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreationEvidence {
    source_id: SourceId,
    arguments: Vec<ProtectedString>,
}

impl CreationEvidence {
    /// Creates optional evidence with sensitive command arguments.
    #[must_use]
    pub fn new<I, S>(source_id: SourceId, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            source_id,
            arguments: arguments
                .into_iter()
                .map(|argument| ProtectedString::sensitive(argument.into()))
                .collect(),
        }
    }

    /// Returns the stable, caller-redacted evidence identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Returns the protected creation arguments.
    #[must_use]
    pub fn arguments(&self) -> &[ProtectedString] {
        &self.arguments
    }
}

/// Effective image configuration used to classify container overrides.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageObservation {
    source_id: SourceId,
    command: Option<EffectiveCommand>,
    environment: Option<Vec<RuntimeEnvironmentVariable>>,
    user: Option<ProtectedString>,
    working_directory: Option<ProtectedString>,
}

impl ImageObservation {
    /// Creates an image observation with no assumed fields.
    #[must_use]
    pub const fn new(source_id: SourceId) -> Self {
        Self {
            source_id,
            command: None,
            environment: None,
            user: None,
            working_directory: None,
        }
    }

    /// Returns the stable, caller-redacted image identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Records the image's effective command default.
    pub fn set_command(&mut self, command: EffectiveCommand) {
        self.command = Some(command);
    }

    /// Returns the observed command default, if the adapter supplied it.
    #[must_use]
    pub const fn command(&self) -> Option<&EffectiveCommand> {
        self.command.as_ref()
    }

    /// Records the complete ordered image environment defaults, including an empty collection.
    pub fn set_environment(&mut self, environment: Vec<RuntimeEnvironmentVariable>) {
        self.environment = Some(environment);
    }

    /// Returns the observed environment defaults, if the adapter supplied them.
    #[must_use]
    pub fn environment(&self) -> Option<&[RuntimeEnvironmentVariable]> {
        self.environment.as_deref()
    }

    /// Records a non-empty image user default as sensitive runtime data.
    pub fn set_user(&mut self, user: impl Into<String>) {
        self.user = Some(ProtectedString::sensitive(user));
    }

    /// Returns the observed image user default.
    #[must_use]
    pub const fn user(&self) -> Option<&ProtectedString> {
        self.user.as_ref()
    }

    /// Records a non-empty image working-directory default as sensitive runtime data.
    pub fn set_working_directory(&mut self, working_directory: impl Into<String>) {
        self.working_directory = Some(ProtectedString::sensitive(working_directory));
    }

    /// Returns the observed image working-directory default.
    #[must_use]
    pub const fn working_directory(&self) -> Option<&ProtectedString> {
        self.working_directory.as_ref()
    }
}

/// Effective state and relationships of one runtime container.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerObservation {
    source_id: SourceId,
    name: Identifier,
    image: Option<ImageReference>,
    image_source_id: Option<SourceId>,
    command: Option<EffectiveCommand>,
    environment: Option<Vec<RuntimeEnvironmentVariable>>,
    user: Option<ProtectedString>,
    working_directory: Option<ProtectedString>,
    read_only_root_filesystem: Option<bool>,
    ports: Vec<Port>,
    mounts: Vec<Mount>,
    networks: Vec<NetworkAttachment>,
    pod_source_id: Option<SourceId>,
    creation_evidence: Option<CreationEvidence>,
}

impl ContainerObservation {
    /// Creates an empty container observation for an explicitly named resource.
    #[must_use]
    pub const fn new(source_id: SourceId, name: Identifier) -> Self {
        Self {
            source_id,
            name,
            image: None,
            image_source_id: None,
            command: None,
            environment: None,
            user: None,
            working_directory: None,
            read_only_root_filesystem: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            networks: Vec::new(),
            pod_source_id: None,
            creation_evidence: None,
        }
    }

    /// Returns the stable, caller-redacted container identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Returns the neutral service name selected by the runtime adapter.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Records the effective image reference and optional linked image observation.
    pub fn set_image(&mut self, image: ImageReference, image_source_id: Option<SourceId>) {
        self.image = Some(image);
        self.image_source_id = image_source_id;
    }

    /// Returns the effective image reference.
    #[must_use]
    pub const fn image(&self) -> Option<&ImageReference> {
        self.image.as_ref()
    }

    /// Returns the linked image-observation identity used for override reconstruction.
    #[must_use]
    pub const fn image_source_id(&self) -> Option<&SourceId> {
        self.image_source_id.as_ref()
    }

    /// Records the effective container command.
    pub fn set_command(&mut self, command: EffectiveCommand) {
        self.command = Some(command);
    }

    /// Returns the effective command, if the adapter supplied it.
    #[must_use]
    pub const fn command(&self) -> Option<&EffectiveCommand> {
        self.command.as_ref()
    }

    /// Records the complete ordered effective environment, including an empty collection.
    pub fn set_environment(&mut self, environment: Vec<RuntimeEnvironmentVariable>) {
        self.environment = Some(environment);
    }

    /// Returns the effective environment, if the adapter supplied it.
    #[must_use]
    pub fn environment(&self) -> Option<&[RuntimeEnvironmentVariable]> {
        self.environment.as_deref()
    }

    /// Records a non-empty effective container user as sensitive runtime data.
    pub fn set_user(&mut self, user: impl Into<String>) {
        self.user = Some(ProtectedString::sensitive(user));
    }

    /// Returns the effective container user, if the adapter supplied one.
    #[must_use]
    pub const fn user(&self) -> Option<&ProtectedString> {
        self.user.as_ref()
    }

    /// Records a non-empty effective container working directory as sensitive runtime data.
    pub fn set_working_directory(&mut self, working_directory: impl Into<String>) {
        self.working_directory = Some(ProtectedString::sensitive(working_directory));
    }

    /// Returns the effective container working directory, if supplied.
    #[must_use]
    pub const fn working_directory(&self) -> Option<&ProtectedString> {
        self.working_directory.as_ref()
    }

    /// Records the effective read-only-root-filesystem choice.
    pub fn set_read_only_root_filesystem(&mut self, read_only: bool) {
        self.read_only_root_filesystem = Some(read_only);
    }

    /// Returns the effective read-only-root-filesystem choice, if supplied.
    #[must_use]
    pub const fn read_only_root_filesystem(&self) -> Option<bool> {
        self.read_only_root_filesystem
    }

    /// Appends one observed published port.
    pub fn add_port(&mut self, port: Port) {
        self.ports.push(port);
    }

    /// Returns observed ports in runtime response order.
    #[must_use]
    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    /// Appends one observed storage relationship.
    pub fn add_mount(&mut self, mount: Mount) {
        self.mounts.push(mount);
    }

    /// Returns observed mounts in runtime response order.
    #[must_use]
    pub fn mounts(&self) -> &[Mount] {
        &self.mounts
    }

    /// Appends one observed network relationship with all aliases in runtime response order.
    pub fn add_network(&mut self, network: NetworkAttachment) {
        self.networks.push(network);
    }

    /// Returns observed network relationships in runtime response order.
    #[must_use]
    pub fn networks(&self) -> &[NetworkAttachment] {
        &self.networks
    }

    /// Records the containing Podman pod observation, if any.
    pub fn set_pod_source_id(&mut self, source_id: SourceId) {
        self.pod_source_id = Some(source_id);
    }

    /// Returns the containing pod identity.
    #[must_use]
    pub const fn pod_source_id(&self) -> Option<&SourceId> {
        self.pod_source_id.as_ref()
    }

    /// Attaches optional creation-command evidence without changing effective values.
    pub fn set_creation_evidence(&mut self, evidence: CreationEvidence) {
        self.creation_evidence = Some(evidence);
    }

    /// Returns optional creation-command evidence.
    #[must_use]
    pub const fn creation_evidence(&self) -> Option<&CreationEvidence> {
        self.creation_evidence.as_ref()
    }
}

/// One inspected runtime network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkObservation {
    source_id: SourceId,
    name: Identifier,
}

impl NetworkObservation {
    /// Creates a runtime network observation.
    #[must_use]
    pub const fn new(source_id: SourceId, name: Identifier) -> Self {
        Self { source_id, name }
    }

    /// Returns the stable, caller-redacted network identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Returns the neutral resource name selected by the runtime adapter.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }
}

/// One inspected runtime volume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeObservation {
    source_id: SourceId,
    name: Identifier,
}

impl VolumeObservation {
    /// Creates a runtime volume observation.
    #[must_use]
    pub const fn new(source_id: SourceId, name: Identifier) -> Self {
        Self { source_id, name }
    }

    /// Returns the stable, caller-redacted volume identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Returns the neutral resource name selected by the runtime adapter.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }
}

/// One inspected Podman pod and its ordered container relationships.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodObservation {
    source_id: SourceId,
    name: Identifier,
    members: Vec<SourceId>,
    creation_evidence: Option<CreationEvidence>,
}

impl PodObservation {
    /// Creates an empty pod observation.
    #[must_use]
    pub const fn new(source_id: SourceId, name: Identifier) -> Self {
        Self {
            source_id,
            name,
            members: Vec::new(),
            creation_evidence: None,
        }
    }

    /// Returns the stable, caller-redacted pod identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Returns the pod name selected by the runtime adapter.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Appends one member container identity in runtime response order.
    pub fn add_member(&mut self, source_id: SourceId) {
        self.members.push(source_id);
    }

    /// Returns member container identities in runtime response order.
    #[must_use]
    pub fn members(&self) -> &[SourceId] {
        &self.members
    }

    /// Attaches optional creation-command evidence without changing effective values.
    pub fn set_creation_evidence(&mut self, evidence: CreationEvidence) {
        self.creation_evidence = Some(evidence);
    }

    /// Returns optional pod creation-command evidence.
    #[must_use]
    pub const fn creation_evidence(&self) -> Option<&CreationEvidence> {
        self.creation_evidence.as_ref()
    }
}

/// A complete caller-selected set of related runtime observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSnapshot {
    application_name: Identifier,
    implementation: RuntimeImplementation,
    containers: Vec<ContainerObservation>,
    images: Vec<ImageObservation>,
    networks: Vec<NetworkObservation>,
    volumes: Vec<VolumeObservation>,
    pods: Vec<PodObservation>,
}

impl RuntimeSnapshot {
    /// Creates an empty snapshot without reading a runtime or ambient state.
    #[must_use]
    pub const fn new(application_name: Identifier, implementation: RuntimeImplementation) -> Self {
        Self {
            application_name,
            implementation,
            containers: Vec::new(),
            images: Vec::new(),
            networks: Vec::new(),
            volumes: Vec::new(),
            pods: Vec::new(),
        }
    }

    /// Returns the neutral application name selected by the caller.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns the runtime implementation that produced this snapshot.
    #[must_use]
    pub const fn implementation(&self) -> &RuntimeImplementation {
        &self.implementation
    }

    /// Adds a uniquely identified container observation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSnapshotError`] for duplicate source identities or container names.
    pub fn add_container(&mut self, container: ContainerObservation) -> Result<(), RuntimeSnapshotError> {
        self.ensure_source_unique(container.source_id())?;
        Self::ensure_name_unique(
            "container",
            container.name(),
            self.containers.iter().map(ContainerObservation::name),
        )?;
        self.containers.push(container);
        Ok(())
    }

    /// Returns containers in caller-selected discovery order.
    #[must_use]
    pub fn containers(&self) -> &[ContainerObservation] {
        &self.containers
    }

    /// Adds a uniquely identified image observation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSnapshotError`] for a duplicate source identity.
    pub fn add_image(&mut self, image: ImageObservation) -> Result<(), RuntimeSnapshotError> {
        self.ensure_source_unique(image.source_id())?;
        self.images.push(image);
        Ok(())
    }

    /// Returns images in caller-selected discovery order.
    #[must_use]
    pub fn images(&self) -> &[ImageObservation] {
        &self.images
    }

    /// Adds a uniquely identified and named network observation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSnapshotError`] for duplicate source identities or network names.
    pub fn add_network(&mut self, network: NetworkObservation) -> Result<(), RuntimeSnapshotError> {
        self.ensure_source_unique(network.source_id())?;
        Self::ensure_name_unique(
            "network",
            network.name(),
            self.networks.iter().map(NetworkObservation::name),
        )?;
        self.networks.push(network);
        Ok(())
    }

    /// Returns networks in caller-selected discovery order.
    #[must_use]
    pub fn networks(&self) -> &[NetworkObservation] {
        &self.networks
    }

    /// Adds a uniquely identified and named volume observation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSnapshotError`] for duplicate source identities or volume names.
    pub fn add_volume(&mut self, volume: VolumeObservation) -> Result<(), RuntimeSnapshotError> {
        self.ensure_source_unique(volume.source_id())?;
        Self::ensure_name_unique(
            "volume",
            volume.name(),
            self.volumes.iter().map(VolumeObservation::name),
        )?;
        self.volumes.push(volume);
        Ok(())
    }

    /// Returns volumes in caller-selected discovery order.
    #[must_use]
    pub fn volumes(&self) -> &[VolumeObservation] {
        &self.volumes
    }

    /// Adds a uniquely identified and named pod observation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeSnapshotError`] for duplicate source identities or pod names.
    pub fn add_pod(&mut self, pod: PodObservation) -> Result<(), RuntimeSnapshotError> {
        self.ensure_source_unique(pod.source_id())?;
        Self::ensure_name_unique("pod", pod.name(), self.pods.iter().map(PodObservation::name))?;
        self.pods.push(pod);
        Ok(())
    }

    /// Returns pods in caller-selected discovery order.
    #[must_use]
    pub fn pods(&self) -> &[PodObservation] {
        &self.pods
    }

    fn ensure_source_unique(&self, source_id: &SourceId) -> Result<(), RuntimeSnapshotError> {
        if self.source_ids().any(|candidate| candidate == source_id) {
            return Err(RuntimeSnapshotError::DuplicateSourceIdentity {
                source_id: source_id.clone(),
            });
        }
        Ok(())
    }

    fn ensure_name_unique<'a>(
        kind: &'static str,
        name: &Identifier,
        existing: impl Iterator<Item = &'a Identifier>,
    ) -> Result<(), RuntimeSnapshotError> {
        if existing.into_iter().any(|candidate| candidate == name) {
            return Err(RuntimeSnapshotError::DuplicateResource {
                kind,
                name: name.as_str().to_owned(),
            });
        }
        Ok(())
    }

    fn source_ids(&self) -> impl Iterator<Item = &SourceId> {
        self.containers
            .iter()
            .map(ContainerObservation::source_id)
            .chain(self.images.iter().map(ImageObservation::source_id))
            .chain(self.networks.iter().map(NetworkObservation::source_id))
            .chain(self.volumes.iter().map(VolumeObservation::source_id))
            .chain(self.pods.iter().map(PodObservation::source_id))
    }
}

/// Invalid runtime snapshot structure.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuntimeSnapshotError {
    /// Two top-level observations used the same caller-selected source identity.
    DuplicateSourceIdentity {
        /// Duplicate stable, caller-redacted identity.
        source_id: SourceId,
    },
    /// Two resources of one kind used the same neutral name.
    DuplicateResource {
        /// Runtime resource kind.
        kind: &'static str,
        /// Duplicate neutral name.
        name: String,
    },
}

impl fmt::Display for RuntimeSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSourceIdentity { source_id } => {
                write!(formatter, "duplicate runtime source identity `{}`", source_id.as_str())
            }
            Self::DuplicateResource { kind, name } => write!(formatter, "duplicate runtime {kind} `{name}`"),
        }
    }
}

impl Error for RuntimeSnapshotError {}

#[cfg(test)]
mod tests {
    use boxferry_model::{Identifier, SourceId};

    use super::{
        ContainerObservation, CreationEvidence, EffectiveCommand, RuntimeEnvironmentVariable, RuntimeImplementation,
        RuntimeSnapshot,
    };

    #[test]
    fn inspected_values_are_redacted_by_default() -> Result<(), String> {
        let command = EffectiveCommand::exec(["server", "--password=never-print-this"]);
        let environment = RuntimeEnvironmentVariable::new(id("PASSWORD")?, "never-print-this");
        let mut container = ContainerObservation::new(source("runtime:podman:container:web")?, id("web")?);
        container.set_user("never-print-this");
        container.set_working_directory("/never-print-this");
        let evidence = CreationEvidence::new(
            source("runtime:podman:create:web")?,
            ["--env", "PASSWORD=never-print-this"],
        );

        for debug in [
            format!("{command:?}"),
            format!("{environment:?}"),
            format!("{container:?}"),
            format!("{evidence:?}"),
        ] {
            assert!(!debug.contains("never-print-this"));
            assert!(debug.contains("[REDACTED]"));
        }
        Ok(())
    }

    #[test]
    fn snapshot_rejects_ambiguous_source_identities_and_names() -> Result<(), String> {
        let mut snapshot = RuntimeSnapshot::new(id("example")?, RuntimeImplementation::Podman);
        snapshot
            .add_container(ContainerObservation::new(source("runtime:container:web")?, id("web")?))
            .map_err(|error| error.to_string())?;

        let duplicate_source = snapshot
            .add_container(ContainerObservation::new(
                source("runtime:container:web")?,
                id("worker")?,
            ))
            .err()
            .ok_or("duplicate source must fail")?;
        assert!(duplicate_source.to_string().contains("source identity"));

        let duplicate_name = snapshot
            .add_container(ContainerObservation::new(
                source("runtime:container:web-2")?,
                id("web")?,
            ))
            .err()
            .ok_or("duplicate name must fail")?;
        assert!(duplicate_name.to_string().contains("container `web`"));
        Ok(())
    }

    fn id(value: &str) -> Result<Identifier, String> {
        Identifier::new(value).map_err(|error| error.to_string())
    }

    fn source(value: &str) -> Result<SourceId, String> {
        SourceId::new(value).map_err(|error| error.to_string())
    }
}
