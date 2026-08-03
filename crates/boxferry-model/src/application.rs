//! Ordered neutral application graph and resource attachments.

use std::{error::Error, fmt};

use crate::{ImageReference, ProtectedString, Sourced};

/// Error raised when constructing an invalid neutral model value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ModelError {
    /// A required value was empty.
    EmptyValue(&'static str),
    /// A value contained a NUL byte.
    ContainsNul(&'static str),
    /// A source byte range ended before it started.
    ReversedSpan {
        /// Inclusive start offset.
        start: usize,
        /// Exclusive end offset.
        end: usize,
    },
    /// An application already contains a resource of this kind and name.
    DuplicateResource {
        /// Neutral resource kind.
        kind: &'static str,
        /// Duplicated name.
        name: String,
    },
    /// An image reference had invalid component structure.
    InvalidImageReference(&'static str),
    /// A container port was zero.
    ZeroContainerPort,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue(kind) => write!(formatter, "{kind} must not be empty"),
            Self::ContainsNul(kind) => write!(formatter, "{kind} must not contain a NUL byte"),
            Self::ReversedSpan { start, end } => {
                write!(formatter, "source span end {end} is before start {start}")
            }
            Self::DuplicateResource { kind, name } => {
                write!(formatter, "duplicate {kind} `{name}`")
            }
            Self::InvalidImageReference(reason) => write!(formatter, "invalid image reference: {reason}"),
            Self::ZeroContainerPort => formatter.write_str("container port must not be zero"),
        }
    }
}

impl Error for ModelError {}

/// Opaque, non-empty application or resource identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Identifier(String);

impl Identifier {
    /// Creates an identifier without applying native-format naming rules.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_text("identifier", &value)?;
        Ok(Self(value))
    }

    /// Returns the authored identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who owns a named network or volume lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResourceOwnership {
    /// The application declares and owns the resource.
    Application,
    /// The target environment owns the resource outside this application.
    External,
    /// A source implementation supplied the resource implicitly.
    Implicit,
}

/// One application-level volume declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Volume {
    name: Identifier,
    ownership: ResourceOwnership,
}

impl Volume {
    /// Creates a volume declaration.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self { name, ownership }
    }

    /// Returns the neutral resource name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the resource lifecycle owner.
    #[must_use]
    pub const fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }
}

/// One application-level network declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Network {
    name: Identifier,
    ownership: ResourceOwnership,
}

impl Network {
    /// Creates a network declaration.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self { name, ownership }
    }

    /// Returns the neutral resource name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the resource lifecycle owner.
    #[must_use]
    pub const fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }
}

/// How a service command overrides the image command.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// Execute an argument vector without target-specific shell parsing.
    Exec(Vec<ProtectedString>),
    /// Execute authored shell text with target-specific shell semantics still unresolved.
    Shell(ProtectedString),
    /// Explicitly clear the image command.
    Empty,
}

/// Source of an environment variable's value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EnvironmentValue {
    /// A literal plain or sensitive value.
    Literal(ProtectedString),
    /// Resolve the value from an explicit caller-provided host environment provider.
    Host,
    /// Ensure the variable is absent.
    Unset,
}

/// One ordered service environment entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentVariable {
    name: Identifier,
    value: EnvironmentValue,
}

impl EnvironmentVariable {
    /// Creates an environment entry.
    #[must_use]
    pub const fn new(name: Identifier, value: EnvironmentValue) -> Self {
        Self { name, value }
    }

    /// Returns the variable name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the unresolved value form.
    #[must_use]
    pub const fn value(&self) -> &EnvironmentValue {
        &self.value
    }
}

/// Transport protocol attached to a published port.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Protocol {
    /// Transmission Control Protocol.
    Tcp,
    /// User Datagram Protocol.
    Udp,
    /// Stream Control Transmission Protocol.
    Sctp,
    /// A preserved protocol not yet understood by the neutral model.
    Other(String),
}

/// One container port and its optional single-host publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Port {
    container: u16,
    published: Option<u16>,
    host_address: Option<String>,
    protocol: Protocol,
}

impl Port {
    /// Creates a port declaration.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::ZeroContainerPort`] when `container` is zero.
    pub fn new(
        container: u16,
        published: Option<u16>,
        host_address: Option<String>,
        protocol: Protocol,
    ) -> Result<Self, ModelError> {
        if container == 0 {
            return Err(ModelError::ZeroContainerPort);
        }
        Ok(Self {
            container,
            published,
            host_address,
            protocol,
        })
    }

    /// Returns the container port.
    #[must_use]
    pub const fn container(&self) -> u16 {
        self.container
    }

    /// Returns the optional host port.
    #[must_use]
    pub const fn published(&self) -> Option<u16> {
        self.published
    }

    /// Returns the optional host address spelling.
    #[must_use]
    pub fn host_address(&self) -> Option<&str> {
        self.host_address.as_deref()
    }

    /// Returns the protocol.
    #[must_use]
    pub const fn protocol(&self) -> &Protocol {
        &self.protocol
    }
}

/// Storage backing attached to a service.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MountSource {
    /// Application-level named volume.
    Volume(Identifier),
    /// Authored host path whose target-specific resolution is deferred.
    HostPath(String),
    /// Anonymous target-managed storage.
    Anonymous,
}

/// `SELinux` label sharing requested for a bind mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SelinuxRelabel {
    /// Share the relabeled content between multiple containers (`z`).
    Shared,
    /// Give the content a private label for one container (`Z`).
    Private,
}

/// One service storage attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mount {
    source: MountSource,
    target: String,
    read_only: bool,
    selinux_relabel: Option<SelinuxRelabel>,
}

impl Mount {
    /// Creates a storage attachment with a non-empty target path.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`] for the target.
    pub fn new(source: MountSource, target: impl Into<String>, read_only: bool) -> Result<Self, ModelError> {
        let target = target.into();
        validate_text("mount target", &target)?;
        Ok(Self {
            source,
            target,
            read_only,
            selinux_relabel: None,
        })
    }

    /// Returns the source storage kind.
    #[must_use]
    pub const fn source(&self) -> &MountSource {
        &self.source
    }

    /// Returns the authored container target path.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns whether the target is read-only.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }

    /// Sets the requested `SELinux` relabel mode without erasing the source syntax decision.
    pub fn set_selinux_relabel(&mut self, relabel: SelinuxRelabel) {
        self.selinux_relabel = Some(relabel);
    }

    /// Returns the requested `SELinux` relabel mode.
    #[must_use]
    pub const fn selinux_relabel(&self) -> Option<SelinuxRelabel> {
        self.selinux_relabel
    }
}

/// One service attachment to an application network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkAttachment {
    network: Identifier,
    aliases: Vec<String>,
}

impl NetworkAttachment {
    /// Creates a network attachment with ordered aliases.
    #[must_use]
    pub const fn new(network: Identifier, aliases: Vec<String>) -> Self {
        Self { network, aliases }
    }

    /// Returns the application network name.
    #[must_use]
    pub const fn network(&self) -> &Identifier {
        &self.network
    }

    /// Returns aliases in authored order.
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

/// One application service with ordered attachments and source provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    name: Identifier,
    image: Option<Sourced<ImageReference>>,
    command: Option<Sourced<Command>>,
    environment: Vec<Sourced<EnvironmentVariable>>,
    ports: Vec<Sourced<Port>>,
    mounts: Vec<Sourced<Mount>>,
    networks: Vec<Sourced<NetworkAttachment>>,
}

impl Service {
    /// Creates an empty service shell for incremental adapter mapping.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            image: None,
            command: None,
            environment: Vec::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
            networks: Vec::new(),
        }
    }

    /// Returns the service name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Sets the optional image reference.
    pub fn set_image(&mut self, image: Sourced<ImageReference>) {
        self.image = Some(image);
    }

    /// Returns the optional image reference.
    #[must_use]
    pub const fn image(&self) -> Option<&Sourced<ImageReference>> {
        self.image.as_ref()
    }

    /// Sets the command override.
    pub fn set_command(&mut self, command: Sourced<Command>) {
        self.command = Some(command);
    }

    /// Returns the command override.
    #[must_use]
    pub const fn command(&self) -> Option<&Sourced<Command>> {
        self.command.as_ref()
    }

    /// Appends an environment entry.
    pub fn add_environment(&mut self, value: Sourced<EnvironmentVariable>) {
        self.environment.push(value);
    }

    /// Returns environment entries in authored order.
    #[must_use]
    pub fn environment(&self) -> &[Sourced<EnvironmentVariable>] {
        &self.environment
    }

    /// Appends a port.
    pub fn add_port(&mut self, value: Sourced<Port>) {
        self.ports.push(value);
    }

    /// Returns ports in authored order.
    #[must_use]
    pub fn ports(&self) -> &[Sourced<Port>] {
        &self.ports
    }

    /// Appends a storage attachment.
    pub fn add_mount(&mut self, value: Sourced<Mount>) {
        self.mounts.push(value);
    }

    /// Returns storage attachments in authored order.
    #[must_use]
    pub fn mounts(&self) -> &[Sourced<Mount>] {
        &self.mounts
    }

    /// Appends a network attachment.
    pub fn add_network(&mut self, value: Sourced<NetworkAttachment>) {
        self.networks.push(value);
    }

    /// Returns network attachments in authored order.
    #[must_use]
    pub fn networks(&self) -> &[Sourced<NetworkAttachment>] {
        &self.networks
    }
}

/// One ordered multi-service application graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    name: Identifier,
    services: Vec<Sourced<Service>>,
    volumes: Vec<Sourced<Volume>>,
    networks: Vec<Sourced<Network>>,
}

impl Application {
    /// Creates an empty application.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            services: Vec::new(),
            volumes: Vec::new(),
            networks: Vec::new(),
        }
    }

    /// Returns the application name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Adds a uniquely named service while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate service name.
    pub fn add_service(&mut self, service: Sourced<Service>) -> Result<(), ModelError> {
        ensure_unique(
            "service",
            service.value().name(),
            self.services.iter().map(|candidate| candidate.value().name()),
        )?;
        self.services.push(service);
        Ok(())
    }

    /// Returns services in declaration order.
    #[must_use]
    pub fn services(&self) -> &[Sourced<Service>] {
        &self.services
    }

    /// Adds a uniquely named volume while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate volume name.
    pub fn add_volume(&mut self, volume: Sourced<Volume>) -> Result<(), ModelError> {
        ensure_unique(
            "volume",
            volume.value().name(),
            self.volumes.iter().map(|candidate| candidate.value().name()),
        )?;
        self.volumes.push(volume);
        Ok(())
    }

    /// Returns volumes in declaration order.
    #[must_use]
    pub fn volumes(&self) -> &[Sourced<Volume>] {
        &self.volumes
    }

    /// Adds a uniquely named network while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate network name.
    pub fn add_network(&mut self, network: Sourced<Network>) -> Result<(), ModelError> {
        ensure_unique(
            "network",
            network.value().name(),
            self.networks.iter().map(|candidate| candidate.value().name()),
        )?;
        self.networks.push(network);
        Ok(())
    }

    /// Returns networks in declaration order.
    #[must_use]
    pub fn networks(&self) -> &[Sourced<Network>] {
        &self.networks
    }
}

fn ensure_unique<'a>(
    kind: &'static str,
    name: &Identifier,
    existing: impl Iterator<Item = &'a Identifier>,
) -> Result<(), ModelError> {
    if existing.into_iter().any(|candidate| candidate == name) {
        return Err(ModelError::DuplicateResource {
            kind,
            name: name.as_str().to_owned(),
        });
    }
    Ok(())
}

fn validate_text(kind: &'static str, value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::EmptyValue(kind));
    }
    if value.contains('\0') {
        return Err(ModelError::ContainsNul(kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Application, Identifier, ModelError, Service};
    use crate::Sourced;

    #[test]
    fn preserves_service_order_and_rejects_duplicate_names() -> Result<(), String> {
        let mut application = Application::new(id("example")?);
        application
            .add_service(Sourced::generated(Service::new(id("web")?)))
            .map_err(|error| error.to_string())?;
        application
            .add_service(Sourced::generated(Service::new(id("database")?)))
            .map_err(|error| error.to_string())?;

        let names: Vec<_> = application
            .services()
            .iter()
            .map(|service| service.value().name().as_str())
            .collect();
        assert_eq!(names, ["web", "database"]);

        let duplicate = application.add_service(Sourced::generated(Service::new(id("web")?)));
        assert!(matches!(duplicate, Err(ModelError::DuplicateResource { .. })));
        Ok(())
    }

    fn id(value: &str) -> Result<Identifier, String> {
        Identifier::new(value).map_err(|error| error.to_string())
    }
}
