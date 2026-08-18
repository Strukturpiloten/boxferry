//! Ordered neutral application graph and resource attachments.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    net::IpAddr,
};

use crate::{ImageAcquisition, ImageBuild, ImageReference, ProtectedString, Provenance, Sourced};

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
    /// A service was added to the same group more than once.
    DuplicateServiceGroupMember {
        /// Service-group name.
        group: String,
        /// Duplicate service name.
        service: String,
    },
    /// A service group referenced a service absent from the application.
    UnknownServiceGroupMember {
        /// Service-group name.
        group: String,
        /// Missing service name.
        service: String,
    },
    /// A service was assigned to more than one application group.
    ServiceInMultipleGroups {
        /// Service name.
        service: String,
        /// Existing service-group name.
        existing: String,
        /// Conflicting service-group name.
        replacement: String,
    },
    /// A service referenced an image acquisition absent from the application.
    UnknownImageAcquisitionReference {
        /// Referencing service name.
        service: String,
        /// Missing image-acquisition resource name.
        acquisition: String,
    },
    /// A service referenced an image build absent from the application.
    UnknownImageBuildReference {
        /// Referencing service name.
        service: String,
        /// Missing image-build resource name.
        build: String,
    },
    /// A volume referenced an image-acquisition resource absent from the application.
    UnknownVolumeImageAcquisitionReference {
        /// Referencing volume name.
        volume: String,
        /// Missing image-acquisition resource name.
        acquisition: String,
    },
    /// A volume referenced an image-build resource absent from the application.
    UnknownVolumeImageBuildReference {
        /// Referencing volume name.
        volume: String,
        /// Missing image-build resource name.
        build: String,
    },
    /// An explicitly supplied artifact dependency referred to a resource absent from the application.
    UnknownArtifactDependencyNode {
        /// Missing resource kind.
        kind: &'static str,
        /// Missing resource name.
        name: String,
    },
    /// Explicit artifact dependencies formed a cycle.
    ImageArtifactDependencyCycle {
        /// Stable, resource-qualified members of one detected cycle.
        nodes: Vec<String>,
    },
    /// An image reference had invalid component structure.
    InvalidImageReference(&'static str),
    /// A container port was zero.
    ZeroContainerPort,
    /// A requested service network-attachment index did not exist.
    UnknownNetworkAttachmentIndex {
        /// Requested attachment index.
        index: usize,
        /// Number of attachments present when replacement was attempted.
        len: usize,
    },
    /// A requested group-runtime network-attachment index did not exist.
    UnknownServiceGroupRuntimeNetworkIndex {
        /// Requested attachment index.
        index: usize,
        /// Number of attachments present when replacement was attempted.
        len: usize,
    },
    /// A service combined a root filesystem with an image source.
    RootfsImageSourceConflict {
        /// Referencing service name.
        service: String,
        /// Conflicting image source kind.
        source: &'static str,
    },
    /// A health-check retry count was not a non-negative decimal integer.
    InvalidHealthcheckRetries,
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
            Self::DuplicateServiceGroupMember { group, service } => {
                write!(
                    formatter,
                    "service group `{group}` contains duplicate member `{service}`"
                )
            }
            Self::UnknownServiceGroupMember { group, service } => {
                write!(
                    formatter,
                    "service group `{group}` references unknown service `{service}`"
                )
            }
            Self::ServiceInMultipleGroups {
                service,
                existing,
                replacement,
            } => write!(
                formatter,
                "service `{service}` belongs to both service groups `{existing}` and `{replacement}`"
            ),
            Self::UnknownImageAcquisitionReference { service, acquisition } => write!(
                formatter,
                "service `{service}` references unknown image acquisition `{acquisition}`"
            ),
            Self::UnknownImageBuildReference { service, build } => {
                write!(
                    formatter,
                    "service `{service}` references unknown image build `{build}`"
                )
            }
            Self::UnknownVolumeImageAcquisitionReference { volume, acquisition } => write!(
                formatter,
                "volume `{volume}` references unknown image acquisition `{acquisition}`"
            ),
            Self::UnknownVolumeImageBuildReference { volume, build } => {
                write!(formatter, "volume `{volume}` references unknown image build `{build}`")
            }
            Self::UnknownArtifactDependencyNode { kind, name } => {
                write!(formatter, "artifact dependency references unknown {kind} `{name}`")
            }
            Self::ImageArtifactDependencyCycle { nodes } => {
                write!(formatter, "image-artifact dependency cycle: {}", nodes.join(" -> "))
            }
            Self::InvalidImageReference(reason) => write!(formatter, "invalid image reference: {reason}"),
            Self::ZeroContainerPort => formatter.write_str("container port must not be zero"),
            Self::UnknownNetworkAttachmentIndex { index, len } => {
                write!(
                    formatter,
                    "network attachment index {index} is outside collection length {len}"
                )
            }
            Self::UnknownServiceGroupRuntimeNetworkIndex { index, len } => {
                write!(
                    formatter,
                    "group-runtime network attachment index {index} is outside collection length {len}"
                )
            }
            Self::RootfsImageSourceConflict { service, source } => write!(
                formatter,
                "service `{service}` combines rootfs with image source `{source}`"
            ),
            Self::InvalidHealthcheckRetries => {
                formatter.write_str("health-check retries must be a non-negative decimal integer")
            }
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

/// Who owns a named application resource lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResourceOwnership {
    /// The application declares and owns the resource.
    Application,
    /// The target environment owns the resource outside this application.
    External,
    /// A source implementation supplied the resource implicitly.
    Implicit,
    /// Runtime inspection established the resource but not who should manage its lifecycle.
    Uncertain,
}

/// One application-level volume declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Volume {
    name: Identifier,
    ownership: ResourceOwnership,
    runtime_name: Option<Sourced<ProtectedString>>,
    service_name: Option<Sourced<ProtectedString>>,
    driver: Option<Sourced<ProtectedString>>,
    device: Option<Sourced<ProtectedString>>,
    type_spelling: Option<Sourced<ProtectedString>>,
    options: Option<Sourced<ProtectedString>>,
    labels: Option<Vec<Sourced<MetadataLabel>>>,
    labels_origins: Vec<Provenance>,
    copy: Option<Sourced<bool>>,
    containers_conf_modules: Option<Vec<Sourced<ProtectedString>>>,
    containers_conf_modules_origins: Vec<Provenance>,
    global_args: Option<Vec<Sourced<ProtectedString>>>,
    global_args_origins: Vec<Provenance>,
    podman_args: Option<Vec<Sourced<ProtectedString>>>,
    podman_args_origins: Vec<Provenance>,
    user: Option<Sourced<ProtectedString>>,
    group: Option<Sourced<ProtectedString>>,
    uid: Option<Sourced<ProtectedString>>,
    gid: Option<Sourced<ProtectedString>>,
    image_source: Option<Sourced<VolumeImageSource>>,
}

impl Volume {
    /// Creates a volume declaration without inferring native defaults or names.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            runtime_name: None,
            service_name: None,
            driver: None,
            device: None,
            type_spelling: None,
            options: None,
            labels: None,
            labels_origins: Vec::new(),
            copy: None,
            containers_conf_modules: None,
            containers_conf_modules_origins: Vec::new(),
            global_args: None,
            global_args_origins: Vec::new(),
            podman_args: None,
            podman_args_origins: Vec::new(),
            user: None,
            group: None,
            uid: None,
            gid: None,
            image_source: None,
        }
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

    /// Sets the explicit runtime name, distinct from the logical resource key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit provider/runtime name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets the explicit service-manager unit name without changing its spelling.
    pub fn set_service_name(&mut self, name: Sourced<ProtectedString>) {
        self.service_name = Some(name);
    }

    /// Returns the explicit service-manager unit name.
    #[must_use]
    pub const fn service_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.service_name.as_ref()
    }

    /// Sets the source-authored volume driver spelling.
    pub fn set_driver(&mut self, driver: Sourced<ProtectedString>) {
        self.driver = Some(driver);
    }

    /// Returns the explicit volume driver spelling.
    #[must_use]
    pub const fn driver(&self) -> Option<&Sourced<ProtectedString>> {
        self.driver.as_ref()
    }

    /// Sets the source-authored local-driver device spelling.
    pub fn set_device(&mut self, device: Sourced<ProtectedString>) {
        self.device = Some(device);
    }

    /// Returns the explicit local-driver device spelling.
    #[must_use]
    pub const fn device(&self) -> Option<&Sourced<ProtectedString>> {
        self.device.as_ref()
    }

    /// Sets the source-authored local-driver type spelling.
    pub fn set_volume_type(&mut self, volume_type: Sourced<ProtectedString>) {
        self.type_spelling = Some(volume_type);
    }

    /// Returns the explicit local-driver type spelling.
    #[must_use]
    pub const fn volume_type(&self) -> Option<&Sourced<ProtectedString>> {
        self.type_spelling.as_ref()
    }

    /// Sets the explicit singleton local-driver options spelling.
    ///
    /// This is intentionally not a generic option bag: it retains the reviewed `Options=`
    /// setting, including the Compose local-driver `driver_opts.o` mapping.
    pub fn set_options(&mut self, options: Sourced<ProtectedString>) {
        self.options = Some(options);
    }

    /// Returns the explicit singleton local-driver options spelling.
    #[must_use]
    pub const fn options(&self) -> Option<&Sourced<ProtectedString>> {
        self.options.as_ref()
    }

    /// Sets metadata labels, retaining omission separately from an explicit empty reset.
    pub fn set_labels(&mut self, labels: Vec<Sourced<MetadataLabel>>) {
        self.set_labels_with_origins(labels, Vec::new());
    }

    /// Sets metadata labels with collection-level provenance.
    pub fn set_labels_with_origins(&mut self, labels: Vec<Sourced<MetadataLabel>>, origins: Vec<Provenance>) {
        self.labels = Some(labels);
        self.labels_origins = origins;
    }

    /// Appends one metadata label in source order.
    pub fn add_label(&mut self, label: Sourced<MetadataLabel>) {
        self.labels.get_or_insert_default().push(label);
    }

    /// Returns metadata labels in authored order, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn labels(&self) -> Option<&[Sourced<MetadataLabel>]> {
        self.labels.as_deref()
    }

    /// Returns collection-level label provenance.
    #[must_use]
    pub fn labels_origins(&self) -> &[Provenance] {
        &self.labels_origins
    }

    /// Sets the explicit copy choice without inferring an omitted source default.
    pub fn set_copy(&mut self, copy: Sourced<bool>) {
        self.copy = Some(copy);
    }

    /// Returns the explicit copy choice.
    #[must_use]
    pub const fn copy(&self) -> Option<&Sourced<bool>> {
        self.copy.as_ref()
    }

    /// Sets ordered protected `containers.conf` modules, retaining an explicit empty reset.
    pub fn set_containers_conf_modules(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_containers_conf_modules_with_origins(values, Vec::new());
    }

    /// Sets ordered protected `containers.conf` modules with collection-level provenance.
    pub fn set_containers_conf_modules_with_origins(
        &mut self,
        values: Vec<Sourced<ProtectedString>>,
        origins: Vec<Provenance>,
    ) {
        self.containers_conf_modules = Some(values);
        self.containers_conf_modules_origins = origins;
    }

    /// Returns protected `containers.conf` modules in authored order, if explicitly present.
    #[must_use]
    pub fn containers_conf_modules(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.containers_conf_modules.as_deref()
    }

    /// Returns collection-level `containers.conf` module provenance.
    #[must_use]
    pub fn containers_conf_modules_origins(&self) -> &[Provenance] {
        &self.containers_conf_modules_origins
    }

    /// Sets ordered protected global arguments, retaining an explicit empty reset.
    pub fn set_global_args(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_global_args_with_origins(values, Vec::new());
    }

    /// Sets ordered protected global arguments with collection-level provenance.
    pub fn set_global_args_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.global_args = Some(values);
        self.global_args_origins = origins;
    }

    /// Returns protected global arguments in authored order, if explicitly present.
    #[must_use]
    pub fn global_args(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.global_args.as_deref()
    }

    /// Returns collection-level global-argument provenance.
    #[must_use]
    pub fn global_args_origins(&self) -> &[Provenance] {
        &self.global_args_origins
    }

    /// Sets ordered protected raw Podman arguments authored by the source.
    ///
    /// These arguments are evidence only. Adapters must not synthesize them from typed settings.
    pub fn set_podman_args(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_podman_args_with_origins(values, Vec::new());
    }

    /// Sets raw Podman arguments with collection-level provenance.
    pub fn set_podman_args_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.podman_args = Some(values);
        self.podman_args_origins = origins;
    }

    /// Returns source-authored raw Podman arguments in order, if explicitly present.
    #[must_use]
    pub fn podman_args(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.podman_args.as_deref()
    }

    /// Returns collection-level raw-Podman-argument provenance.
    #[must_use]
    pub fn podman_args_origins(&self) -> &[Provenance] {
        &self.podman_args_origins
    }

    /// Sets the source-authored volume user identity.
    pub fn set_user(&mut self, user: Sourced<ProtectedString>) {
        self.user = Some(user);
    }

    /// Returns the source-authored volume user identity.
    #[must_use]
    pub const fn user(&self) -> Option<&Sourced<ProtectedString>> {
        self.user.as_ref()
    }

    /// Sets the source-authored volume group identity.
    pub fn set_group(&mut self, group: Sourced<ProtectedString>) {
        self.group = Some(group);
    }

    /// Returns the source-authored volume group identity.
    #[must_use]
    pub const fn group(&self) -> Option<&Sourced<ProtectedString>> {
        self.group.as_ref()
    }

    /// Sets the source-authored numeric user-ID spelling.
    pub fn set_uid(&mut self, uid: Sourced<ProtectedString>) {
        self.uid = Some(uid);
    }

    /// Returns the source-authored numeric user-ID spelling.
    #[must_use]
    pub const fn uid(&self) -> Option<&Sourced<ProtectedString>> {
        self.uid.as_ref()
    }

    /// Sets the source-authored numeric group-ID spelling.
    pub fn set_gid(&mut self, gid: Sourced<ProtectedString>) {
        self.gid = Some(gid);
    }

    /// Returns the source-authored numeric group-ID spelling.
    #[must_use]
    pub const fn gid(&self) -> Option<&Sourced<ProtectedString>> {
        self.gid.as_ref()
    }

    /// Sets the explicitly selected image source for an image-backed volume.
    ///
    /// Artifact references are checked by [`Application::validate_image_artifact_references`]
    /// after a complete application graph has been assembled.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`] for an invalid literal
    /// image source.
    pub fn set_image_source(&mut self, image_source: Sourced<VolumeImageSource>) -> Result<(), ModelError> {
        image_source.value().validate()?;
        self.image_source = Some(image_source);
        Ok(())
    }

    /// Returns the explicitly selected image source for an image-backed volume.
    #[must_use]
    pub const fn image_source(&self) -> Option<&Sourced<VolumeImageSource>> {
        self.image_source.as_ref()
    }
}

/// The explicit source of an image-backed volume.
///
/// Literal images and named image artifacts remain distinct. The literal form is protected so a
/// private registry location cannot leak through model debug output.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VolumeImageSource {
    /// One protected literal image spelling.
    Literal(ProtectedString),
    /// One named image-acquisition resource.
    ImageAcquisition(Identifier),
    /// One named image-build resource.
    ImageBuild(Identifier),
}

impl VolumeImageSource {
    fn validate(&self) -> Result<(), ModelError> {
        if let Self::Literal(image) = self {
            validate_text("volume image", image.expose())?;
        }
        Ok(())
    }
}

/// A format-neutral node used by explicit image-artifact dependency validation.
///
/// Native adapters create values only after they have established a typed reference. This model
/// does not parse native raw argument or mount spellings to invent an edge.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ArtifactDependencyNode {
    /// One application volume.
    Volume(Identifier),
    /// One image-acquisition resource.
    ImageAcquisition(Identifier),
    /// One image-build resource.
    ImageBuild(Identifier),
}

impl ArtifactDependencyNode {
    fn kind_and_name(&self) -> (&'static str, &Identifier) {
        match self {
            Self::Volume(name) => ("volume", name),
            Self::ImageAcquisition(name) => ("image acquisition", name),
            Self::ImageBuild(name) => ("image build", name),
        }
    }

    fn display_name(&self) -> String {
        let (kind, name) = self.kind_and_name();
        format!("{kind}:{}", name.as_str())
    }
}

/// One explicit, directed image-artifact dependency.
///
/// It intentionally carries independently sourced endpoints. Collection-level provenance belongs
/// to the caller's enclosing source declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDependency {
    source: Sourced<ArtifactDependencyNode>,
    target: Sourced<ArtifactDependencyNode>,
}

impl ArtifactDependency {
    /// Creates an explicit typed dependency.
    #[must_use]
    pub const fn new(source: Sourced<ArtifactDependencyNode>, target: Sourced<ArtifactDependencyNode>) -> Self {
        Self { source, target }
    }

    /// Returns the depending node and its provenance.
    #[must_use]
    pub const fn source(&self) -> &Sourced<ArtifactDependencyNode> {
        &self.source
    }

    /// Returns the required node and its provenance.
    #[must_use]
    pub const fn target(&self) -> &Sourced<ArtifactDependencyNode> {
        &self.target
    }
}

/// One application-level network declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Network {
    name: Identifier,
    ownership: ResourceOwnership,
    runtime_name: Option<Sourced<ProtectedString>>,
    driver: Option<Sourced<ProtectedString>>,
    driver_options: Option<Vec<Sourced<NetworkDriverOption>>>,
    driver_options_origins: Vec<Provenance>,
    labels: Option<Vec<Sourced<MetadataLabel>>>,
    labels_origins: Vec<Provenance>,
    internal: Option<Sourced<bool>>,
    ipv6: Option<Sourced<bool>>,
    ipam_driver: Option<Sourced<ProtectedString>>,
    ipam_configs: Option<Vec<Sourced<NetworkIpamConfig>>>,
    ipam_configs_origins: Vec<Provenance>,
}

/// One driver-specific network option with independently sourced key and value.
///
/// Values remain protected because provider options can carry deployment-specific credentials or
/// topology details. They remain separate fields so adapters never need to parse a synthetic
/// `key=value` assignment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDriverOption {
    name: Sourced<Identifier>,
    value: Sourced<ProtectedString>,
}

impl NetworkDriverOption {
    /// Creates one driver option with field-level provenance.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::ContainsNul`] when the option value contains a NUL byte. An empty
    /// value is retained because source mappings may explicitly reset a driver option.
    pub fn new(name: Sourced<Identifier>, value: Sourced<ProtectedString>) -> Result<Self, ModelError> {
        validate_no_nul("network driver option value", value.value().expose())?;
        Ok(Self { name, value })
    }

    /// Returns the driver-option key and its provenance.
    #[must_use]
    pub const fn name(&self) -> &Sourced<Identifier> {
        &self.name
    }

    /// Returns the protected driver-option value and its provenance.
    #[must_use]
    pub const fn value(&self) -> &Sourced<ProtectedString> {
        &self.value
    }
}

/// One explicitly associated IPAM configuration row.
///
/// A row never pairs independent native subnet, gateway, and range collections by position:
/// adapters retain the source association only when it is present. The subnet is required, while
/// gateway and range remain independently optional and sourced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkIpamConfig {
    subnet: Sourced<ProtectedString>,
    gateway: Option<Sourced<ProtectedString>>,
    ip_range: Option<Sourced<ProtectedString>>,
}

impl NetworkIpamConfig {
    /// Creates one IPAM row with a required non-empty subnet spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`] for the subnet.
    pub fn new(subnet: Sourced<ProtectedString>) -> Result<Self, ModelError> {
        validate_text("network IPAM subnet", subnet.value().expose())?;
        Ok(Self {
            subnet,
            gateway: None,
            ip_range: None,
        })
    }

    /// Returns the required subnet spelling and its provenance.
    #[must_use]
    pub const fn subnet(&self) -> &Sourced<ProtectedString> {
        &self.subnet
    }

    /// Sets an explicitly associated gateway spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn set_gateway(&mut self, gateway: Sourced<ProtectedString>) -> Result<(), ModelError> {
        validate_text("network IPAM gateway", gateway.value().expose())?;
        self.gateway = Some(gateway);
        Ok(())
    }

    /// Returns the optional gateway spelling and its provenance.
    #[must_use]
    pub const fn gateway(&self) -> Option<&Sourced<ProtectedString>> {
        self.gateway.as_ref()
    }

    /// Sets an explicitly associated IP-range spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn set_ip_range(&mut self, ip_range: Sourced<ProtectedString>) -> Result<(), ModelError> {
        validate_text("network IPAM IP range", ip_range.value().expose())?;
        self.ip_range = Some(ip_range);
        Ok(())
    }

    /// Returns the optional IP-range spelling and its provenance.
    #[must_use]
    pub const fn ip_range(&self) -> Option<&Sourced<ProtectedString>> {
        self.ip_range.as_ref()
    }
}

/// Material source for an application-managed configuration resource.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConfigMaterial {
    /// Read configuration bytes from a caller-resolved file source.
    File(ProtectedString),
    /// Read configuration bytes from an explicitly supplied environment value.
    Environment(ProtectedString),
    /// Use source-authored inline configuration content.
    Content(ProtectedString),
}

/// One application-level configuration declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    name: Identifier,
    ownership: ResourceOwnership,
    runtime_name: Option<Sourced<ProtectedString>>,
    material: Option<Sourced<ConfigMaterial>>,
}

impl Config {
    /// Creates a configuration declaration without guessing material or a runtime name.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            runtime_name: None,
            material: None,
        }
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

    /// Sets a provider/runtime-level name distinct from the neutral resource key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit provider/runtime-level name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets the application-managed material source.
    pub fn set_material(&mut self, material: Sourced<ConfigMaterial>) {
        self.material = Some(material);
    }

    /// Returns the optional material source.
    #[must_use]
    pub const fn material(&self) -> Option<&Sourced<ConfigMaterial>> {
        self.material.as_ref()
    }
}

/// Material source for an application-managed secret resource.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SecretMaterial {
    /// Read secret bytes from a caller-resolved file source.
    File(ProtectedString),
    /// Read secret bytes from an explicitly supplied environment value.
    Environment(ProtectedString),
}

/// One application-level secret declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Secret {
    name: Identifier,
    ownership: ResourceOwnership,
    runtime_name: Option<Sourced<ProtectedString>>,
    material: Option<Sourced<SecretMaterial>>,
}

impl Secret {
    /// Creates a secret declaration without guessing material or a runtime name.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            runtime_name: None,
            material: None,
        }
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

    /// Sets a provider/runtime-level name distinct from the neutral resource key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit provider/runtime-level name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets the application-managed material source.
    pub fn set_material(&mut self, material: Sourced<SecretMaterial>) {
        self.material = Some(material);
    }

    /// Returns the optional material source.
    #[must_use]
    pub const fn material(&self) -> Option<&Sourced<SecretMaterial>> {
        self.material.as_ref()
    }
}

/// Authored syntax family retained for a config or secret grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResourceGrantSyntax {
    /// Resource-name short syntax with source-format defaults.
    Short,
    /// Mapping-based syntax with separately authored options.
    Long,
}

/// One ordered service grant of a configuration or secret resource.
///
/// The containing service collection determines whether this grants a config or secret. Keeping
/// one shared shape avoids inventing differences between the common source/target/ownership
/// options while preserving the short/long syntax decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceGrant {
    source: ProtectedString,
    syntax: ResourceGrantSyntax,
    target: Option<Sourced<ProtectedString>>,
    uid: Option<Sourced<ProtectedString>>,
    gid: Option<Sourced<ProtectedString>>,
    mode: Option<Sourced<ProtectedString>>,
}

impl ResourceGrant {
    /// Creates a grant with a non-empty source resource name.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(source: ProtectedString, syntax: ResourceGrantSyntax) -> Result<Self, ModelError> {
        validate_text("resource grant source", source.expose())?;
        Ok(Self {
            source,
            syntax,
            target: None,
            uid: None,
            gid: None,
            mode: None,
        })
    }

    /// Returns the referenced neutral resource name.
    #[must_use]
    pub const fn source(&self) -> &ProtectedString {
        &self.source
    }

    /// Returns the authored short/long syntax family.
    #[must_use]
    pub const fn syntax(&self) -> ResourceGrantSyntax {
        self.syntax
    }

    /// Sets the requested container path or environment-variable name.
    pub fn set_target(&mut self, target: Sourced<ProtectedString>) {
        self.target = Some(target);
    }

    /// Returns the explicitly authored target.
    #[must_use]
    pub const fn target(&self) -> Option<&Sourced<ProtectedString>> {
        self.target.as_ref()
    }

    /// Sets the requested container user-ID spelling.
    pub fn set_uid(&mut self, uid: Sourced<ProtectedString>) {
        self.uid = Some(uid);
    }

    /// Returns the explicitly authored user-ID spelling.
    #[must_use]
    pub const fn uid(&self) -> Option<&Sourced<ProtectedString>> {
        self.uid.as_ref()
    }

    /// Sets the requested container group-ID spelling.
    pub fn set_gid(&mut self, gid: Sourced<ProtectedString>) {
        self.gid = Some(gid);
    }

    /// Returns the explicitly authored group-ID spelling.
    #[must_use]
    pub const fn gid(&self) -> Option<&Sourced<ProtectedString>> {
        self.gid.as_ref()
    }

    /// Sets the requested permission-mode spelling.
    pub fn set_mode(&mut self, mode: Sourced<ProtectedString>) {
        self.mode = Some(mode);
    }

    /// Returns the explicitly authored permission-mode spelling.
    #[must_use]
    pub const fn mode(&self) -> Option<&Sourced<ProtectedString>> {
        self.mode.as_ref()
    }
}

impl Network {
    /// Creates a network declaration without inferring runtime settings or defaults.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            runtime_name: None,
            driver: None,
            driver_options: None,
            driver_options_origins: Vec::new(),
            labels: None,
            labels_origins: Vec::new(),
            internal: None,
            ipv6: None,
            ipam_driver: None,
            ipam_configs: None,
            ipam_configs_origins: Vec::new(),
        }
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

    /// Sets the explicit provider/runtime name distinct from the logical resource key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit provider/runtime name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets the source-authored network driver spelling.
    pub fn set_driver(&mut self, driver: Sourced<ProtectedString>) {
        self.driver = Some(driver);
    }

    /// Returns the explicit network driver spelling.
    #[must_use]
    pub const fn driver(&self) -> Option<&Sourced<ProtectedString>> {
        self.driver.as_ref()
    }

    /// Sets ordered driver options while retaining explicit emptiness as a reset.
    pub fn set_driver_options(&mut self, options: Vec<Sourced<NetworkDriverOption>>) {
        self.driver_options = Some(options);
        self.driver_options_origins.clear();
    }

    /// Sets ordered driver options with collection-level provenance.
    pub fn set_driver_options_with_origins(
        &mut self,
        options: Vec<Sourced<NetworkDriverOption>>,
        origins: Vec<Provenance>,
    ) {
        self.driver_options = Some(options);
        self.driver_options_origins = origins;
    }

    /// Appends one driver option in source order.
    pub fn add_driver_option(&mut self, option: Sourced<NetworkDriverOption>) {
        self.driver_options.get_or_insert_default().push(option);
    }

    /// Returns ordered driver options, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn driver_options(&self) -> Option<&[Sourced<NetworkDriverOption>]> {
        self.driver_options.as_deref()
    }

    /// Returns collection-level driver-option provenance.
    #[must_use]
    pub fn driver_options_origins(&self) -> &[Provenance] {
        &self.driver_options_origins
    }

    /// Sets ordered network metadata labels while retaining explicit emptiness as a reset.
    pub fn set_labels(&mut self, labels: Vec<Sourced<MetadataLabel>>) {
        self.labels = Some(labels);
        self.labels_origins.clear();
    }

    /// Sets ordered network metadata labels with collection-level provenance.
    pub fn set_labels_with_origins(&mut self, labels: Vec<Sourced<MetadataLabel>>, origins: Vec<Provenance>) {
        self.labels = Some(labels);
        self.labels_origins = origins;
    }

    /// Appends one network metadata label in source order.
    pub fn add_label(&mut self, label: Sourced<MetadataLabel>) {
        self.labels.get_or_insert_default().push(label);
    }

    /// Returns ordered network labels, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn labels(&self) -> Option<&[Sourced<MetadataLabel>]> {
        self.labels.as_deref()
    }

    /// Returns collection-level network-label provenance.
    #[must_use]
    pub fn labels_origins(&self) -> &[Provenance] {
        &self.labels_origins
    }

    /// Sets the literal `internal` network flag without inferring a source default.
    pub fn set_internal(&mut self, internal: Sourced<bool>) {
        self.internal = Some(internal);
    }

    /// Returns the explicitly authored `internal` network flag.
    #[must_use]
    pub const fn internal(&self) -> Option<&Sourced<bool>> {
        self.internal.as_ref()
    }

    /// Sets the literal IPv6-enable network flag without inferring a source default.
    pub fn set_ipv6(&mut self, ipv6: Sourced<bool>) {
        self.ipv6 = Some(ipv6);
    }

    /// Returns the explicitly authored IPv6-enable network flag.
    #[must_use]
    pub const fn ipv6(&self) -> Option<&Sourced<bool>> {
        self.ipv6.as_ref()
    }

    /// Sets the source-authored IPAM driver spelling.
    pub fn set_ipam_driver(&mut self, driver: Sourced<ProtectedString>) {
        self.ipam_driver = Some(driver);
    }

    /// Returns the explicit IPAM driver spelling.
    #[must_use]
    pub const fn ipam_driver(&self) -> Option<&Sourced<ProtectedString>> {
        self.ipam_driver.as_ref()
    }

    /// Sets ordered associated IPAM rows while retaining explicit emptiness as a reset.
    pub fn set_ipam_configs(&mut self, configs: Vec<Sourced<NetworkIpamConfig>>) {
        self.ipam_configs = Some(configs);
        self.ipam_configs_origins.clear();
    }

    /// Sets ordered associated IPAM rows with collection-level provenance.
    pub fn set_ipam_configs_with_origins(
        &mut self,
        configs: Vec<Sourced<NetworkIpamConfig>>,
        origins: Vec<Provenance>,
    ) {
        self.ipam_configs = Some(configs);
        self.ipam_configs_origins = origins;
    }

    /// Appends one independently associated IPAM row in source order.
    pub fn add_ipam_config(&mut self, config: Sourced<NetworkIpamConfig>) {
        self.ipam_configs.get_or_insert_default().push(config);
    }

    /// Returns ordered associated IPAM rows, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn ipam_configs(&self) -> Option<&[Sourced<NetworkIpamConfig>]> {
        self.ipam_configs.as_deref()
    }

    /// Returns collection-level IPAM configuration provenance.
    #[must_use]
    pub fn ipam_configs_origins(&self) -> &[Provenance] {
        &self.ipam_configs_origins
    }
}

/// One structural group of application services.
///
/// Membership alone does not imply shared Linux namespaces, an infra container, or a target
/// workload kind. Source and target adapters must model or report those semantics separately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceGroup {
    name: Identifier,
    ownership: ResourceOwnership,
    members: Vec<Sourced<Identifier>>,
    runtime: Option<Sourced<ServiceGroupRuntime>>,
}

impl ServiceGroup {
    /// Creates an empty structural service group.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            members: Vec::new(),
            runtime: None,
        }
    }

    /// Returns the neutral group name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the group lifecycle owner.
    #[must_use]
    pub const fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }

    /// Appends one uniquely named member in source order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateServiceGroupMember`] when the service was already added.
    pub fn add_member(&mut self, member: Sourced<Identifier>) -> Result<(), ModelError> {
        if self.members.iter().any(|candidate| candidate.value() == member.value()) {
            return Err(ModelError::DuplicateServiceGroupMember {
                group: self.name.as_str().to_owned(),
                service: member.value().as_str().to_owned(),
            });
        }
        self.members.push(member);
        Ok(())
    }

    /// Returns member service names in source order with relationship provenance.
    #[must_use]
    pub fn members(&self) -> &[Sourced<Identifier>] {
        &self.members
    }

    /// Sets the optional runtime settings associated with this structural group.
    ///
    /// These settings do not alter membership semantics. In particular, their presence does not
    /// infer namespace sharing for a group that did not author such settings.
    pub fn set_runtime(&mut self, runtime: Sourced<ServiceGroupRuntime>) {
        self.runtime = Some(runtime);
    }

    /// Returns the optional native group-runtime settings.
    #[must_use]
    pub const fn runtime(&self) -> Option<&Sourced<ServiceGroupRuntime>> {
        self.runtime.as_ref()
    }
}

/// Pod exit behavior retained without assigning lifecycle semantics to group membership.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GroupExitPolicy {
    /// Stop the pod when a member container exits.
    Stop,
    /// Keep the pod running when a member container exits.
    Continue,
    /// Preserve a source-native exit-policy spelling for target-side classification.
    Raw(ProtectedString),
}

/// Native runtime settings owned by one [`ServiceGroup`].
///
/// The group's logical [`ServiceGroup::name`] remains distinct from an optional runtime pod name
/// and the optional systemd service name. All settings are group-scoped; adapters must not assign
/// them to an arbitrary member service.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServiceGroupRuntime {
    runtime_name: Option<Sourced<ProtectedString>>,
    service_name: Option<Sourced<ProtectedString>>,
    host_mappings: Option<Vec<Sourced<HostMapping>>>,
    host_mappings_origins: Vec<Provenance>,
    ports: Option<Vec<Sourced<Port>>>,
    ports_origins: Vec<Provenance>,
    networks: Option<Vec<Sourced<NetworkAttachment>>>,
    networks_origins: Vec<Provenance>,
    user_namespace: Option<Sourced<ProtectedString>>,
    mounts: Option<Vec<Sourced<Mount>>>,
    mounts_origins: Vec<Provenance>,
    shm_size: Option<Sourced<ProtectedString>>,
    exit_policy: Option<Sourced<GroupExitPolicy>>,
    stop_timeout: Option<Sourced<StopTimeout>>,
}

impl ServiceGroupRuntime {
    /// Creates empty group-runtime settings for incremental adapter mapping.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            runtime_name: None,
            service_name: None,
            host_mappings: None,
            host_mappings_origins: Vec::new(),
            ports: None,
            ports_origins: Vec::new(),
            networks: None,
            networks_origins: Vec::new(),
            user_namespace: None,
            mounts: None,
            mounts_origins: Vec::new(),
            shm_size: None,
            exit_policy: None,
            stop_timeout: None,
        }
    }

    /// Sets the pod/runtime name distinct from the neutral group key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit pod/runtime name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets the systemd service name distinct from the logical and runtime names.
    pub fn set_service_name(&mut self, name: Sourced<ProtectedString>) {
        self.service_name = Some(name);
    }

    /// Returns the explicit systemd service name.
    #[must_use]
    pub const fn service_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.service_name.as_ref()
    }

    /// Sets pod host mappings, preserving omitted versus explicit-empty state.
    pub fn set_host_mappings(&mut self, values: Vec<Sourced<HostMapping>>) {
        self.set_host_mappings_with_origins(values, Vec::new());
    }

    /// Sets pod host mappings with collection-level provenance.
    pub fn set_host_mappings_with_origins(&mut self, values: Vec<Sourced<HostMapping>>, origins: Vec<Provenance>) {
        self.host_mappings = Some(values);
        self.host_mappings_origins = origins;
    }

    /// Appends one pod host mapping in source order.
    pub fn add_host_mapping(&mut self, value: Sourced<HostMapping>) {
        self.host_mappings.get_or_insert_default().push(value);
    }

    /// Returns pod host mappings, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn host_mappings(&self) -> Option<&[Sourced<HostMapping>]> {
        self.host_mappings.as_deref()
    }

    /// Returns collection-level pod-host-mapping provenance.
    #[must_use]
    pub fn host_mappings_origins(&self) -> &[Provenance] {
        &self.host_mappings_origins
    }

    /// Sets pod ports, preserving omitted versus explicit-empty state.
    pub fn set_ports(&mut self, values: Vec<Sourced<Port>>) {
        self.set_ports_with_origins(values, Vec::new());
    }

    /// Sets pod ports with collection-level provenance.
    pub fn set_ports_with_origins(&mut self, values: Vec<Sourced<Port>>, origins: Vec<Provenance>) {
        self.ports = Some(values);
        self.ports_origins = origins;
    }

    /// Appends one pod port in source order.
    pub fn add_port(&mut self, value: Sourced<Port>) {
        self.ports.get_or_insert_default().push(value);
    }

    /// Returns pod ports, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn ports(&self) -> Option<&[Sourced<Port>]> {
        self.ports.as_deref()
    }

    /// Returns collection-level pod-port provenance.
    #[must_use]
    pub fn ports_origins(&self) -> &[Provenance] {
        &self.ports_origins
    }

    /// Sets pod network attachments, preserving omitted versus explicit-empty state.
    pub fn set_networks(&mut self, values: Vec<Sourced<NetworkAttachment>>) {
        self.set_networks_with_origins(values, Vec::new());
    }

    /// Sets pod network attachments with collection-level provenance.
    pub fn set_networks_with_origins(&mut self, values: Vec<Sourced<NetworkAttachment>>, origins: Vec<Provenance>) {
        self.networks = Some(values);
        self.networks_origins = origins;
    }

    /// Appends one pod network attachment in source order.
    pub fn add_network(&mut self, value: Sourced<NetworkAttachment>) {
        self.networks.get_or_insert_default().push(value);
    }

    /// Replaces one pod network attachment without changing authored order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownServiceGroupRuntimeNetworkIndex`] when `index` is outside
    /// the explicitly authored attachment collection.
    pub fn replace_network(
        &mut self,
        index: usize,
        value: Sourced<NetworkAttachment>,
    ) -> Result<Sourced<NetworkAttachment>, ModelError> {
        let len = self.networks.as_ref().map_or(0, Vec::len);
        let Some(networks) = self.networks.as_mut() else {
            return Err(ModelError::UnknownServiceGroupRuntimeNetworkIndex { index, len });
        };
        let Some(slot) = networks.get_mut(index) else {
            return Err(ModelError::UnknownServiceGroupRuntimeNetworkIndex { index, len });
        };
        Ok(std::mem::replace(slot, value))
    }

    /// Returns pod network attachments, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn networks(&self) -> Option<&[Sourced<NetworkAttachment>]> {
        self.networks.as_deref()
    }

    /// Returns collection-level pod-network provenance.
    #[must_use]
    pub fn networks_origins(&self) -> &[Provenance] {
        &self.networks_origins
    }

    /// Sets the raw-preserving pod user-namespace mode.
    pub fn set_user_namespace(&mut self, value: Sourced<ProtectedString>) {
        self.user_namespace = Some(value);
    }

    /// Returns the explicit pod user-namespace mode.
    #[must_use]
    pub const fn user_namespace(&self) -> Option<&Sourced<ProtectedString>> {
        self.user_namespace.as_ref()
    }

    /// Sets pod mounts, preserving omitted versus explicit-empty state.
    pub fn set_mounts(&mut self, values: Vec<Sourced<Mount>>) {
        self.set_mounts_with_origins(values, Vec::new());
    }

    /// Sets pod mounts with collection-level provenance.
    pub fn set_mounts_with_origins(&mut self, values: Vec<Sourced<Mount>>, origins: Vec<Provenance>) {
        self.mounts = Some(values);
        self.mounts_origins = origins;
    }

    /// Appends one pod mount in source order.
    pub fn add_mount(&mut self, value: Sourced<Mount>) {
        self.mounts.get_or_insert_default().push(value);
    }

    /// Returns pod mounts, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn mounts(&self) -> Option<&[Sourced<Mount>]> {
        self.mounts.as_deref()
    }

    /// Returns collection-level pod-mount provenance.
    #[must_use]
    pub fn mounts_origins(&self) -> &[Provenance] {
        &self.mounts_origins
    }

    /// Sets the raw protected pod shared-memory-size spelling.
    pub fn set_shm_size(&mut self, value: Sourced<ProtectedString>) {
        self.shm_size = Some(value);
    }

    /// Returns the raw protected pod shared-memory-size spelling.
    #[must_use]
    pub const fn shm_size(&self) -> Option<&Sourced<ProtectedString>> {
        self.shm_size.as_ref()
    }

    /// Sets the pod exit policy.
    pub fn set_exit_policy(&mut self, value: Sourced<GroupExitPolicy>) {
        self.exit_policy = Some(value);
    }

    /// Returns the explicit pod exit policy.
    #[must_use]
    pub const fn exit_policy(&self) -> Option<&Sourced<GroupExitPolicy>> {
        self.exit_policy.as_ref()
    }

    /// Sets the raw stop-grace duration for the pod.
    pub fn set_stop_timeout(&mut self, value: Sourced<StopTimeout>) {
        self.stop_timeout = Some(value);
    }

    /// Returns the explicit raw pod stop-grace duration.
    #[must_use]
    pub const fn stop_timeout(&self) -> Option<&Sourced<StopTimeout>> {
        self.stop_timeout.as_ref()
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

/// Startup notification behavior requested for one service.
///
/// The variants retain the portable meanings of Quadlet's `Notify=false`, `Notify=true`, and
/// `Notify=healthy` without making a target implementation implicit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StartupNotification {
    /// The runtime owns readiness notification (`Notify=false`).
    Runtime,
    /// The application process owns readiness notification (`Notify=true`).
    Application,
    /// Readiness is reported from the service health check (`Notify=healthy`).
    Healthy,
}

/// How a service overrides the image entrypoint.
///
/// This remains distinct from [`Command`]: container runtimes combine the two values differently,
/// and a source can explicitly clear either image default independently.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Entrypoint {
    /// Execute an argument vector without target-specific shell parsing.
    Exec(Vec<ProtectedString>),
    /// Execute authored shell text with target-specific shell semantics still unresolved.
    Shell(ProtectedString),
    /// Explicitly clear the image entrypoint.
    Empty,
}

/// Source-independent image pull intent.
///
/// Variants beyond the shared policies intentionally retain source-native behavior for a target
/// adapter to classify rather than silently reducing it to a different policy.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PullPolicy {
    /// Always acquire the image before starting.
    Always,
    /// Acquire the image only when it is absent locally.
    Missing,
    /// Never acquire the image automatically.
    Never,
    /// A source-specific spelling with semantics similar to [`Self::Missing`].
    IfNotPresent,
    /// Build an image instead of pulling it.
    Build,
    /// Refresh the image daily.
    Daily,
    /// Refresh the image weekly.
    Weekly,
    /// Refresh using a source-native interval spelling.
    Every(ProtectedString),
    /// Retain any other target-native policy spelling.
    Raw(ProtectedString),
}

/// Raw-preserving stop grace duration.
///
/// Source adapters validate native duration grammar; target adapters report spellings their
/// selected implementation cannot express.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopTimeout(String);

impl StopTimeout {
    /// Creates a non-empty duration spelling without imposing one source format's grammar.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_text("stop timeout", &value)?;
        Ok(Self(value))
    }

    /// Returns the source adapter's retained duration spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A container port exposed for inter-service use without publishing it to a host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExposedPort {
    container: u16,
    protocol: Protocol,
}

impl ExposedPort {
    /// Creates one exposed container port.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::ZeroContainerPort`] when `container` is zero.
    pub fn new(container: u16, protocol: Protocol) -> Result<Self, ModelError> {
        if container == 0 {
            return Err(ModelError::ZeroContainerPort);
        }
        Ok(Self { container, protocol })
    }

    /// Returns the container-side port number.
    #[must_use]
    pub const fn container(&self) -> u16 {
        self.container
    }

    /// Returns the requested transport protocol.
    #[must_use]
    pub const fn protocol(&self) -> &Protocol {
        &self.protocol
    }
}

/// Container-level automatic restart intent.
///
/// This policy is distinct from Compose dependency restart propagation and orchestrator-level
/// deployment restart policies. A limited on-failure policy uses a non-zero retry count so the
/// absence of a limit remains distinguishable from an invalid zero-valued limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RestartPolicy {
    /// Never restart the container automatically.
    Never,
    /// Restart after every container exit.
    Always,
    /// Restart after a failed container exit, optionally up to a finite retry count.
    OnFailure {
        /// Maximum number of restart attempts; `None` means no policy-specific limit.
        maximum_retries: Option<std::num::NonZeroU64>,
    },
    /// Restart automatically unless an explicit stop state must survive runtime restart.
    UnlessStopped,
}

impl RestartPolicy {
    /// Creates an on-failure policy with an optional non-zero retry limit.
    #[must_use]
    pub const fn on_failure(maximum_retries: Option<std::num::NonZeroU64>) -> Self {
        Self::OnFailure { maximum_retries }
    }

    /// Returns the finite retry limit of an on-failure policy.
    #[must_use]
    pub const fn maximum_retries(self) -> Option<std::num::NonZeroU64> {
        match self {
            Self::OnFailure { maximum_retries } => maximum_retries,
            Self::Never | Self::Always | Self::UnlessStopped => None,
        }
    }
}

/// How a container runtime executes one service health check.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HealthcheckCommand {
    /// Execute an argument vector without a container shell.
    Exec(Vec<ProtectedString>),
    /// Execute authored command text through the container shell.
    Shell(ProtectedString),
}

/// Raw-preserving duration shared by container health-check implementations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthcheckDuration(String);

impl HealthcheckDuration {
    /// Creates a non-empty duration spelling without imposing one source format's grammar.
    ///
    /// Source adapters remain responsible for validating native duration syntax. Target adapters
    /// must report spellings that their selected implementation cannot represent.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_text("health-check duration", &value)?;
        Ok(Self(value))
    }

    /// Returns the source adapter's retained duration spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Raw-preserving non-negative health-check retry count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthcheckRetries(String);

impl HealthcheckRetries {
    /// Creates a retry count while retaining its authored decimal spelling.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`], [`ModelError::ContainsNul`], or
    /// [`ModelError::InvalidHealthcheckRetries`].
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_text("health-check retries", &value)?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ModelError::InvalidHealthcheckRetries);
        }
        Ok(Self(value))
    }

    /// Returns the source adapter's retained decimal spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Format-independent service health-check intent with field-level provenance.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Healthcheck {
    command: Option<Sourced<HealthcheckCommand>>,
    disabled: Option<Sourced<bool>>,
    interval: Option<Sourced<HealthcheckDuration>>,
    timeout: Option<Sourced<HealthcheckDuration>>,
    retries: Option<Sourced<HealthcheckRetries>>,
    start_period: Option<Sourced<HealthcheckDuration>>,
    start_interval: Option<Sourced<HealthcheckDuration>>,
}

impl Healthcheck {
    /// Creates an empty health-check definition for incremental source-adapter mapping.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            command: None,
            disabled: None,
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            start_interval: None,
        }
    }

    /// Sets the command executed by the runtime.
    pub fn set_command(&mut self, command: Sourced<HealthcheckCommand>) {
        self.command = Some(command);
    }

    /// Returns the optional health command.
    #[must_use]
    pub const fn command(&self) -> Option<&Sourced<HealthcheckCommand>> {
        self.command.as_ref()
    }

    /// Retains an explicit enable/disable decision.
    pub fn set_disabled(&mut self, disabled: Sourced<bool>) {
        self.disabled = Some(disabled);
    }

    /// Returns the explicit disable decision, if the source supplied one.
    #[must_use]
    pub const fn disabled(&self) -> Option<&Sourced<bool>> {
        self.disabled.as_ref()
    }

    /// Sets the interval between regular checks.
    pub fn set_interval(&mut self, interval: Sourced<HealthcheckDuration>) {
        self.interval = Some(interval);
    }

    /// Returns the interval between regular checks.
    #[must_use]
    pub const fn interval(&self) -> Option<&Sourced<HealthcheckDuration>> {
        self.interval.as_ref()
    }

    /// Sets the maximum duration of one check.
    pub fn set_timeout(&mut self, timeout: Sourced<HealthcheckDuration>) {
        self.timeout = Some(timeout);
    }

    /// Returns the maximum duration of one check.
    #[must_use]
    pub const fn timeout(&self) -> Option<&Sourced<HealthcheckDuration>> {
        self.timeout.as_ref()
    }

    /// Sets the number of failures required before becoming unhealthy.
    pub fn set_retries(&mut self, retries: Sourced<HealthcheckRetries>) {
        self.retries = Some(retries);
    }

    /// Returns the failure threshold.
    #[must_use]
    pub const fn retries(&self) -> Option<&Sourced<HealthcheckRetries>> {
        self.retries.as_ref()
    }

    /// Sets the startup grace period.
    pub fn set_start_period(&mut self, start_period: Sourced<HealthcheckDuration>) {
        self.start_period = Some(start_period);
    }

    /// Returns the startup grace period.
    #[must_use]
    pub const fn start_period(&self) -> Option<&Sourced<HealthcheckDuration>> {
        self.start_period.as_ref()
    }

    /// Sets the check interval used during the startup grace period.
    pub fn set_start_interval(&mut self, start_interval: Sourced<HealthcheckDuration>) {
        self.start_interval = Some(start_interval);
    }

    /// Returns the check interval used during the startup grace period.
    #[must_use]
    pub const fn start_interval(&self) -> Option<&Sourced<HealthcheckDuration>> {
        self.start_interval.as_ref()
    }
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

/// Authored syntax family retained for an environment-file declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EnvironmentFileSyntax {
    /// Path-only short syntax with source-format defaults.
    Short,
    /// Mapping-based syntax with separately authored options.
    Long,
}

/// Explicit parsing mode requested for an environment file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EnvironmentFileFormat {
    /// Preserve values without interpolation or quote processing when the source supports it.
    Raw,
}

/// One ordered environment-file declaration.
///
/// This value describes source intent only. Importing it never reads the referenced file. A
/// caller that wants to materialize environment values must cross a separate filesystem-access
/// boundary and apply the source implementation's parsing rules explicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentFile {
    path: ProtectedString,
    syntax: EnvironmentFileSyntax,
    required: Option<Sourced<bool>>,
    format: Option<Sourced<EnvironmentFileFormat>>,
}

impl EnvironmentFile {
    /// Creates an environment-file declaration with a non-empty path.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(path: ProtectedString, syntax: EnvironmentFileSyntax) -> Result<Self, ModelError> {
        validate_text("environment-file path", path.expose())?;
        Ok(Self {
            path,
            syntax,
            required: None,
            format: None,
        })
    }

    /// Returns the source-authored path without resolving or reading it.
    #[must_use]
    pub const fn path(&self) -> &ProtectedString {
        &self.path
    }

    /// Returns the authored short/long syntax family.
    #[must_use]
    pub const fn syntax(&self) -> EnvironmentFileSyntax {
        self.syntax
    }

    /// Retains an explicit required/optional choice.
    pub fn set_required(&mut self, required: Sourced<bool>) {
        self.required = Some(required);
    }

    /// Returns the explicit required/optional choice, if authored.
    #[must_use]
    pub const fn required(&self) -> Option<&Sourced<bool>> {
        self.required.as_ref()
    }

    /// Returns whether the source requires the file, including the default of `true`.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required.as_ref().is_none_or(|required| *required.value())
    }

    /// Retains an explicitly selected parsing mode.
    pub fn set_format(&mut self, format: Sourced<EnvironmentFileFormat>) {
        self.format = Some(format);
    }

    /// Returns the explicitly selected parsing mode, if authored.
    #[must_use]
    pub const fn format(&self) -> Option<&Sourced<EnvironmentFileFormat>> {
        self.format.as_ref()
    }
}

/// One portable metadata label attached to an application resource.
///
/// Label names remain opaque because Docker, Podman, Compose, and future targets do not share one
/// useful restrictive grammar. Values use [`ProtectedString`] so runtime-derived metadata cannot
/// leak through debug output before a caller explicitly authorizes rendering it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataLabel {
    name: Identifier,
    value: ProtectedString,
}

impl MetadataLabel {
    /// Creates one metadata label.
    #[must_use]
    pub const fn new(name: Identifier, value: ProtectedString) -> Self {
        Self { name, value }
    }

    /// Returns the opaque metadata-label name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Returns the protected metadata-label value.
    #[must_use]
    pub const fn value(&self) -> &ProtectedString {
        &self.value
    }
}

/// One protected annotation with independently sourced name and value.
///
/// Unlike service metadata labels, annotations can be target-scoped. Their opaque spelling and
/// field provenance remain available for a later target adapter to make that decision explicitly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    name: Sourced<Identifier>,
    value: Sourced<ProtectedString>,
}

impl Annotation {
    /// Creates one annotation with separately retained name and value provenance.
    #[must_use]
    pub const fn new(name: Sourced<Identifier>, value: Sourced<ProtectedString>) -> Self {
        Self { name, value }
    }

    /// Returns the opaque annotation name and its provenance.
    #[must_use]
    pub const fn name(&self) -> &Sourced<Identifier> {
        &self.name
    }

    /// Returns the protected annotation value and its provenance.
    #[must_use]
    pub const fn value(&self) -> &Sourced<ProtectedString> {
        &self.value
    }
}

/// One provider-specific logging option with independently sourced name and value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingOption {
    name: Sourced<Identifier>,
    value: Sourced<ProtectedString>,
}

impl LoggingOption {
    /// Creates one logging option without imposing a provider's option grammar.
    #[must_use]
    pub const fn new(name: Sourced<Identifier>, value: Sourced<ProtectedString>) -> Self {
        Self { name, value }
    }

    /// Returns the provider-specific option name and its provenance.
    #[must_use]
    pub const fn name(&self) -> &Sourced<Identifier> {
        &self.name
    }

    /// Returns the protected provider-specific option value and its provenance.
    #[must_use]
    pub const fn value(&self) -> &Sourced<ProtectedString> {
        &self.value
    }
}

/// Provider-specific logging intent.
///
/// A missing options collection differs from an explicit empty collection, which can reset
/// provider defaults. Option names and values remain opaque and ordered.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Logging {
    driver: Option<Sourced<ProtectedString>>,
    options: Option<Vec<Sourced<LoggingOption>>>,
    options_origins: Vec<Provenance>,
}

impl Logging {
    /// Creates an empty logging declaration for incremental source-adapter mapping.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            driver: None,
            options: None,
            options_origins: Vec::new(),
        }
    }

    /// Sets the provider-specific logging driver spelling.
    pub fn set_driver(&mut self, driver: Sourced<ProtectedString>) {
        self.driver = Some(driver);
    }

    /// Returns the explicitly authored logging driver.
    #[must_use]
    pub const fn driver(&self) -> Option<&Sourced<ProtectedString>> {
        self.driver.as_ref()
    }

    /// Sets ordered logging options while preserving explicit emptiness.
    pub fn set_options(&mut self, options: Vec<Sourced<LoggingOption>>) {
        self.options = Some(options);
        self.options_origins.clear();
    }

    /// Appends one logging option in source order.
    pub fn add_option(&mut self, option: Sourced<LoggingOption>) {
        self.options.get_or_insert_default().push(option);
    }

    /// Sets ordered logging options and collection-level provenance.
    pub fn set_options_with_origins(&mut self, options: Vec<Sourced<LoggingOption>>, origins: Vec<Provenance>) {
        self.options = Some(options);
        self.options_origins = origins;
    }

    /// Returns ordered logging options, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn options(&self) -> Option<&[Sourced<LoggingOption>]> {
        self.options.as_deref()
    }

    /// Returns collection-level logging option provenance.
    #[must_use]
    pub fn options_origins(&self) -> &[Provenance] {
        &self.options_origins
    }
}

/// One mutually exclusive reload action.
///
/// Reloading is lifecycle control, not a regular command or a lifecycle hook. A service therefore
/// retains one explicit action rather than allowing command and signal declarations to conflict.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReloadAction {
    /// Execute the supplied command to request a reload.
    Command(Command),
    /// Deliver the supplied signal spelling to request a reload.
    Signal(ProtectedString),
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

/// One raw-preserving address or runtime token used by a service host mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostAddress {
    raw: String,
    kind: HostAddressKind,
}

impl HostAddress {
    /// Classifies an authored host-mapping address without normalizing it.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] or [`ModelError::ContainsNul`].
    pub fn new(raw: impl Into<String>) -> Result<Self, ModelError> {
        let raw = raw.into();
        validate_text("host mapping address", &raw)?;
        let unbracketed = raw
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(&raw);
        let kind = if raw == "host-gateway" {
            HostAddressKind::HostGateway
        } else {
            match unbracketed.parse::<IpAddr>() {
                Ok(IpAddr::V4(_)) => HostAddressKind::Ipv4,
                Ok(IpAddr::V6(_)) => HostAddressKind::Ipv6 {
                    bracketed: raw.starts_with('[') && raw.ends_with(']'),
                },
                Err(_) => HostAddressKind::Other,
            }
        };
        Ok(Self { raw, kind })
    }

    /// Returns the address or runtime token exactly as supplied by the source adapter.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Returns the conservative lexical classification.
    #[must_use]
    pub const fn kind(&self) -> HostAddressKind {
        self.kind
    }
}

/// Conservative kind of one service host-mapping address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HostAddressKind {
    /// An IPv4 address.
    Ipv4,
    /// An IPv6 address, retaining whether its source spelling used brackets.
    Ipv6 {
        /// Whether the source adapter observed `[::1]` rather than `::1`.
        bracketed: bool,
    },
    /// The runtime-specific `host-gateway` token.
    HostGateway,
    /// A deferred or implementation-specific value.
    Other,
}

/// One ordered service hostname-to-address mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostMapping {
    hostname: Identifier,
    address: HostAddress,
}

impl HostMapping {
    /// Creates a service host mapping.
    #[must_use]
    pub const fn new(hostname: Identifier, address: HostAddress) -> Self {
        Self { hostname, address }
    }

    /// Returns the hostname written into the target hosts file.
    #[must_use]
    pub const fn hostname(&self) -> &Identifier {
        &self.hostname
    }

    /// Returns the raw-preserving address or runtime token.
    #[must_use]
    pub const fn address(&self) -> &HostAddress {
        &self.address
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
#[derive(Clone, Eq, PartialEq)]
pub struct NetworkAttachment {
    network: Identifier,
    aliases: Vec<String>,
    alias_sensitivities: Vec<bool>,
    alias_origins: Vec<Vec<Provenance>>,
    ipv4_address: Option<Sourced<ProtectedString>>,
    ipv6_address: Option<Sourced<ProtectedString>>,
}

impl NetworkAttachment {
    /// Creates a network attachment with ordered provenance-bearing aliases.
    #[must_use]
    pub fn new(network: Identifier, aliases: Vec<Sourced<ProtectedString>>) -> Self {
        Self {
            network,
            aliases: aliases.iter().map(|alias| alias.value().expose().to_owned()).collect(),
            alias_sensitivities: aliases.iter().map(|alias| alias.value().is_sensitive()).collect(),
            alias_origins: aliases.into_iter().map(|alias| alias.origins().to_vec()).collect(),
            ipv4_address: None,
            ipv6_address: None,
        }
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

    /// Returns alias origins in the same order as [`Self::aliases`].
    #[must_use]
    pub fn alias_origins(&self) -> &[Vec<Provenance>] {
        &self.alias_origins
    }

    /// Returns per-alias sensitivity flags in the same order as [`Self::aliases`].
    ///
    /// Target adapters use this boundary to avoid passing protected aliases into native APIs that
    /// cannot redact them.
    #[must_use]
    pub fn alias_sensitivities(&self) -> &[bool] {
        &self.alias_sensitivities
    }

    /// Replaces aliases with ordered provenance-bearing source values.
    pub fn set_aliases_with_provenance(&mut self, aliases: Vec<Sourced<ProtectedString>>) {
        self.aliases = aliases.iter().map(|alias| alias.value().expose().to_owned()).collect();
        self.alias_sensitivities = aliases.iter().map(|alias| alias.value().is_sensitive()).collect();
        self.alias_origins = aliases.into_iter().map(|alias| alias.origins().to_vec()).collect();
    }

    /// Appends one alias and its provenance.
    pub fn add_alias(&mut self, alias: &Sourced<ProtectedString>) {
        self.aliases.push(alias.value().expose().to_owned());
        self.alias_sensitivities.push(alias.value().is_sensitive());
        self.alias_origins.push(alias.origins().to_vec());
    }

    /// Sets the attachment's explicit IPv4 address spelling.
    pub fn set_ipv4_address(&mut self, address: Sourced<ProtectedString>) {
        self.ipv4_address = Some(address);
    }

    /// Returns the attachment's explicit IPv4 address spelling.
    #[must_use]
    pub const fn ipv4_address(&self) -> Option<&Sourced<ProtectedString>> {
        self.ipv4_address.as_ref()
    }

    /// Sets the attachment's explicit IPv6 address spelling.
    pub fn set_ipv6_address(&mut self, address: Sourced<ProtectedString>) {
        self.ipv6_address = Some(address);
    }

    /// Returns the attachment's explicit IPv6 address spelling.
    #[must_use]
    pub const fn ipv6_address(&self) -> Option<&Sourced<ProtectedString>> {
        self.ipv6_address.as_ref()
    }
}

impl fmt::Debug for NetworkAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let aliases = self
            .aliases
            .iter()
            .enumerate()
            .map(|(index, alias)| {
                if self.alias_sensitivities.get(index).copied().unwrap_or(false) {
                    "[REDACTED]"
                } else {
                    alias.as_str()
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("NetworkAttachment")
            .field("network", &self.network)
            .field("aliases", &aliases)
            .field("alias_origins", &self.alias_origins)
            .field("ipv4_address", &self.ipv4_address)
            .field("ipv6_address", &self.ipv6_address)
            .finish()
    }
}

/// Readiness state a service dependency must reach before its dependent starts.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ServiceDependencyCondition {
    /// The dependency's service startup completed.
    Started,
    /// The dependency reported healthy readiness.
    Healthy,
    /// The dependency exited successfully.
    CompletedSuccessfully,
    /// A source-specific condition retained for explicit target-side reporting.
    Other(ProtectedString),
}

/// One ordered dependency edge from a service to another application service.
///
/// Optional fields distinguish source defaults from explicitly authored values. The surrounding
/// [`Sourced`] value carries the referenced service-name provenance, while each option retains its
/// own field-level provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceDependency {
    service: Identifier,
    condition: Option<Sourced<ServiceDependencyCondition>>,
    restart: Option<Sourced<bool>>,
    required: Option<Sourced<bool>>,
}

/// One raw-preserving kernel-parameter assignment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KernelParameter {
    name: ProtectedString,
    value: ProtectedString,
}

impl KernelParameter {
    /// Creates an assignment without interpreting kernel namespaces or privileges.
    #[must_use]
    pub const fn new(name: ProtectedString, value: ProtectedString) -> Self {
        Self { name, value }
    }

    /// Returns the authored parameter name.
    #[must_use]
    pub const fn name(&self) -> &ProtectedString {
        &self.name
    }

    /// Returns the authored scalar value spelling.
    #[must_use]
    pub const fn value(&self) -> &ProtectedString {
        &self.value
    }
}

/// One raw-preserving resource-limit declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLimit {
    name: ProtectedString,
    soft: Option<Sourced<ProtectedString>>,
    hard: Option<Sourced<ProtectedString>>,
}

impl ResourceLimit {
    /// Creates a limit with independently sourced soft and hard values.
    #[must_use]
    pub const fn new(
        name: ProtectedString,
        soft: Option<Sourced<ProtectedString>>,
        hard: Option<Sourced<ProtectedString>>,
    ) -> Self {
        Self { name, soft, hard }
    }

    /// Returns the raw limit name.
    #[must_use]
    pub const fn name(&self) -> &ProtectedString {
        &self.name
    }

    /// Returns the optional soft value.
    #[must_use]
    pub const fn soft(&self) -> Option<&Sourced<ProtectedString>> {
        self.soft.as_ref()
    }

    /// Returns the optional hard value.
    #[must_use]
    pub const fn hard(&self) -> Option<&Sourced<ProtectedString>> {
        self.hard.as_ref()
    }
}

/// A service device declaration with its authored syntax retained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Device {
    /// A raw short device spelling.
    Short(ProtectedString),
    /// A long device mapping with independently sourced members.
    Long {
        /// Host-device source spelling.
        source: Option<Sourced<ProtectedString>>,
        /// Container-device target spelling.
        target: Option<Sourced<ProtectedString>>,
        /// Raw permission spelling.
        permissions: Option<Sourced<ProtectedString>>,
    },
}

/// One format-independent service security option.
///
/// Native adapters retain ordering and duplicates around this value. They classify any
/// source-specific singleton conflicts rather than imposing those rules on the neutral model.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SecurityOption {
    /// Selects an `AppArmor` profile.
    AppArmor(ProtectedString),
    /// Enables or disables the no-new-privileges security bit.
    NoNewPrivileges(bool),
    /// Selects a seccomp profile.
    SeccompProfile(ProtectedString),
    /// Selects whether `SELinux` labeling is disabled (`true` disables labels).
    SecurityLabelDisable(bool),
    /// Selects the `SELinux` file type.
    SecurityLabelFileType(ProtectedString),
    /// Selects the `SELinux` level.
    SecurityLabelLevel(ProtectedString),
    /// Enables or disables nested `SELinux` labeling.
    SecurityLabelNested(bool),
    /// Selects the `SELinux` type.
    SecurityLabelType(ProtectedString),
    /// Masks one or more colon-separated container paths, or `ALL`.
    Mask(ProtectedString),
    /// Unmasks one or more colon-separated container paths, or `ALL`.
    Unmask(ProtectedString),
}

impl ServiceDependency {
    /// Creates an edge using source-format defaults for readiness, restart propagation, and
    /// requirement strength.
    #[must_use]
    pub const fn new(service: Identifier) -> Self {
        Self {
            service,
            condition: None,
            restart: None,
            required: None,
        }
    }

    /// Returns the referenced application service.
    #[must_use]
    pub const fn service(&self) -> &Identifier {
        &self.service
    }

    /// Sets the explicitly authored readiness condition.
    pub fn set_condition(&mut self, condition: Sourced<ServiceDependencyCondition>) {
        self.condition = Some(condition);
    }

    /// Returns the explicitly authored readiness condition, if any.
    #[must_use]
    pub const fn condition(&self) -> Option<&Sourced<ServiceDependencyCondition>> {
        self.condition.as_ref()
    }

    /// Retains whether source-controlled dependency updates restart the dependent service.
    pub fn set_restart(&mut self, restart: Sourced<bool>) {
        self.restart = Some(restart);
    }

    /// Returns the explicit restart-propagation choice, if any.
    #[must_use]
    pub const fn restart(&self) -> Option<&Sourced<bool>> {
        self.restart.as_ref()
    }

    /// Retains whether absence or failure of the dependency blocks the dependent service.
    pub fn set_required(&mut self, required: Sourced<bool>) {
        self.required = Some(required);
    }

    /// Returns the explicit requirement-strength choice, if any.
    #[must_use]
    pub const fn required(&self) -> Option<&Sourced<bool>> {
        self.required.as_ref()
    }

    /// Returns the effective source requirement, including the default of `true`.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required.as_ref().is_none_or(|required| *required.value())
    }
}

/// One application service with ordered attachments and source provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    name: Identifier,
    runtime_name: Option<Sourced<ProtectedString>>,
    rootfs: Option<Sourced<ProtectedString>>,
    image: Option<Sourced<ImageReference>>,
    image_acquisition: Option<Sourced<Identifier>>,
    image_build: Option<Sourced<Identifier>>,
    command: Option<Sourced<Command>>,
    startup_notification: Option<Sourced<StartupNotification>>,
    entrypoint: Option<Sourced<Entrypoint>>,
    run_init: Option<Sourced<bool>>,
    stop_timeout: Option<Sourced<StopTimeout>>,
    pull_policy: Option<Sourced<PullPolicy>>,
    memory_limit: Option<Sourced<ProtectedString>>,
    exposed_ports: Option<Vec<Sourced<ExposedPort>>>,
    exposed_ports_origins: Vec<Provenance>,
    restart_policy: Option<Sourced<RestartPolicy>>,
    healthcheck: Option<Sourced<Healthcheck>>,
    labels: Vec<Sourced<MetadataLabel>>,
    annotations: Option<Vec<Sourced<Annotation>>>,
    annotations_origins: Vec<Provenance>,
    logging: Option<Sourced<Logging>>,
    reload_action: Option<Sourced<ReloadAction>>,
    user: Option<Sourced<ProtectedString>>,
    group: Option<Sourced<ProtectedString>>,
    user_namespace: Option<Sourced<ProtectedString>>,
    supplementary_groups: Vec<Sourced<ProtectedString>>,
    working_directory: Option<Sourced<ProtectedString>>,
    read_only_root_filesystem: Option<Sourced<bool>>,
    hostname: Option<Sourced<ProtectedString>>,
    dns_servers: Option<Vec<Sourced<ProtectedString>>>,
    dns_servers_origins: Vec<Provenance>,
    dns_options: Option<Vec<Sourced<ProtectedString>>>,
    dns_options_origins: Vec<Provenance>,
    dns_search_domains: Option<Vec<Sourced<ProtectedString>>>,
    dns_search_domains_origins: Vec<Provenance>,
    security_options: Option<Vec<Sourced<SecurityOption>>>,
    security_options_origins: Vec<Provenance>,
    pids_limit: Option<Sourced<ProtectedString>>,
    shm_size: Option<Sourced<ProtectedString>>,
    cap_add: Option<Vec<Sourced<ProtectedString>>>,
    cap_add_origins: Vec<Provenance>,
    cap_drop: Option<Vec<Sourced<ProtectedString>>>,
    cap_drop_origins: Vec<Provenance>,
    tmpfs: Option<Vec<Sourced<ProtectedString>>>,
    tmpfs_origins: Vec<Provenance>,
    sysctls: Option<Vec<Sourced<KernelParameter>>>,
    sysctls_origins: Vec<Provenance>,
    ulimits: Option<Vec<Sourced<ResourceLimit>>>,
    ulimits_origins: Vec<Provenance>,
    devices: Option<Vec<Sourced<Device>>>,
    devices_origins: Vec<Provenance>,
    stop_signal: Option<Sourced<ProtectedString>>,
    podman_args: Option<Vec<Sourced<ProtectedString>>>,
    podman_args_origins: Vec<Provenance>,
    environment: Vec<Sourced<EnvironmentVariable>>,
    environment_files: Vec<Sourced<EnvironmentFile>>,
    host_mappings: Vec<Sourced<HostMapping>>,
    ports: Vec<Sourced<Port>>,
    mounts: Vec<Sourced<Mount>>,
    config_grants: Vec<Sourced<ResourceGrant>>,
    secret_grants: Vec<Sourced<ResourceGrant>>,
    networks: Vec<Sourced<NetworkAttachment>>,
    dependencies: Vec<Sourced<ServiceDependency>>,
}

impl Service {
    /// Creates an empty service shell for incremental adapter mapping.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            runtime_name: None,
            rootfs: None,
            image: None,
            image_acquisition: None,
            image_build: None,
            command: None,
            startup_notification: None,
            entrypoint: None,
            run_init: None,
            stop_timeout: None,
            pull_policy: None,
            memory_limit: None,
            exposed_ports: None,
            exposed_ports_origins: Vec::new(),
            restart_policy: None,
            healthcheck: None,
            labels: Vec::new(),
            annotations: None,
            annotations_origins: Vec::new(),
            logging: None,
            reload_action: None,
            user: None,
            group: None,
            user_namespace: None,
            supplementary_groups: Vec::new(),
            working_directory: None,
            read_only_root_filesystem: None,
            hostname: None,
            dns_servers: None,
            dns_servers_origins: Vec::new(),
            dns_options: None,
            dns_options_origins: Vec::new(),
            dns_search_domains: None,
            dns_search_domains_origins: Vec::new(),
            security_options: None,
            security_options_origins: Vec::new(),
            pids_limit: None,
            shm_size: None,
            cap_add: None,
            cap_add_origins: Vec::new(),
            cap_drop: None,
            cap_drop_origins: Vec::new(),
            tmpfs: None,
            tmpfs_origins: Vec::new(),
            sysctls: None,
            sysctls_origins: Vec::new(),
            ulimits: None,
            ulimits_origins: Vec::new(),
            devices: None,
            devices_origins: Vec::new(),
            stop_signal: None,
            podman_args: None,
            podman_args_origins: Vec::new(),
            environment: Vec::new(),
            environment_files: Vec::new(),
            host_mappings: Vec::new(),
            ports: Vec::new(),
            mounts: Vec::new(),
            config_grants: Vec::new(),
            secret_grants: Vec::new(),
            networks: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// Returns the service name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Sets an explicit provider/runtime-level container name distinct from the service key.
    pub fn set_runtime_name(&mut self, name: Sourced<ProtectedString>) {
        self.runtime_name = Some(name);
    }

    /// Returns the explicit provider/runtime-level container name.
    #[must_use]
    pub const fn runtime_name(&self) -> Option<&Sourced<ProtectedString>> {
        self.runtime_name.as_ref()
    }

    /// Sets a protected root-filesystem path instead of an image source.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::RootfsImageSourceConflict`] when this service already has an image,
    /// image acquisition, or image build reference.
    pub fn set_rootfs(&mut self, rootfs: Sourced<ProtectedString>) -> Result<(), ModelError> {
        self.ensure_rootfs_is_compatible()?;
        self.rootfs = Some(rootfs);
        Ok(())
    }

    /// Returns the protected root-filesystem path, if explicitly authored.
    #[must_use]
    pub const fn rootfs(&self) -> Option<&Sourced<ProtectedString>> {
        self.rootfs.as_ref()
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

    /// References a separately declared image-acquisition resource.
    ///
    /// This does not replace [`Self::image`], which remains the runtime container image reference.
    pub fn set_image_acquisition(&mut self, acquisition: Sourced<Identifier>) {
        self.image_acquisition = Some(acquisition);
    }

    /// Returns the separately declared image-acquisition resource reference.
    #[must_use]
    pub const fn image_acquisition(&self) -> Option<&Sourced<Identifier>> {
        self.image_acquisition.as_ref()
    }

    /// References a separately declared image-build resource.
    ///
    /// This does not replace [`Self::image`] or any container runtime settings.
    pub fn set_image_build(&mut self, build: Sourced<Identifier>) {
        self.image_build = Some(build);
    }

    /// Returns the separately declared image-build resource reference.
    #[must_use]
    pub const fn image_build(&self) -> Option<&Sourced<Identifier>> {
        self.image_build.as_ref()
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

    /// Sets the source-authored startup-notification behavior.
    pub fn set_startup_notification(&mut self, notification: Sourced<StartupNotification>) {
        self.startup_notification = Some(notification);
    }

    /// Returns the explicit startup-notification behavior.
    #[must_use]
    pub const fn startup_notification(&self) -> Option<&Sourced<StartupNotification>> {
        self.startup_notification.as_ref()
    }

    /// Sets the entrypoint override independently from the command override.
    pub fn set_entrypoint(&mut self, entrypoint: Sourced<Entrypoint>) {
        self.entrypoint = Some(entrypoint);
    }

    /// Returns the optional entrypoint override.
    #[must_use]
    pub const fn entrypoint(&self) -> Option<&Sourced<Entrypoint>> {
        self.entrypoint.as_ref()
    }

    /// Sets whether the runtime should run its init process.
    pub fn set_run_init(&mut self, run_init: Sourced<bool>) {
        self.run_init = Some(run_init);
    }

    /// Returns the explicit init-process choice.
    #[must_use]
    pub const fn run_init(&self) -> Option<&Sourced<bool>> {
        self.run_init.as_ref()
    }

    /// Sets the raw stop-grace duration.
    pub fn set_stop_timeout(&mut self, timeout: Sourced<StopTimeout>) {
        self.stop_timeout = Some(timeout);
    }

    /// Returns the explicit raw stop-grace duration.
    #[must_use]
    pub const fn stop_timeout(&self) -> Option<&Sourced<StopTimeout>> {
        self.stop_timeout.as_ref()
    }

    /// Sets the source-independent image pull intent.
    pub fn set_pull_policy(&mut self, policy: Sourced<PullPolicy>) {
        self.pull_policy = Some(policy);
    }

    /// Returns the explicit image pull intent.
    #[must_use]
    pub const fn pull_policy(&self) -> Option<&Sourced<PullPolicy>> {
        self.pull_policy.as_ref()
    }

    /// Sets the raw protected memory-limit spelling.
    pub fn set_memory_limit(&mut self, limit: Sourced<ProtectedString>) {
        self.memory_limit = Some(limit);
    }

    /// Returns the raw protected memory-limit spelling.
    #[must_use]
    pub const fn memory_limit(&self) -> Option<&Sourced<ProtectedString>> {
        self.memory_limit.as_ref()
    }

    /// Sets exposed container ports, preserving omission separately from an explicit empty list.
    pub fn set_exposed_ports(&mut self, ports: Vec<Sourced<ExposedPort>>) {
        self.exposed_ports = Some(ports);
        self.exposed_ports_origins.clear();
    }

    /// Sets exposed container ports and collection-level provenance.
    pub fn set_exposed_ports_with_origins(&mut self, ports: Vec<Sourced<ExposedPort>>, origins: Vec<Provenance>) {
        self.exposed_ports = Some(ports);
        self.exposed_ports_origins = origins;
    }

    /// Appends one exposed container port without publishing it to a host.
    pub fn add_exposed_port(&mut self, port: Sourced<ExposedPort>) {
        self.exposed_ports.get_or_insert_default().push(port);
    }

    /// Returns exposed container ports in authored order, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn exposed_ports(&self) -> Option<&[Sourced<ExposedPort>]> {
        self.exposed_ports.as_deref()
    }

    /// Returns collection-level exposed-port provenance.
    #[must_use]
    pub fn exposed_ports_origins(&self) -> &[Provenance] {
        &self.exposed_ports_origins
    }

    /// Sets the container-level automatic restart policy.
    pub fn set_restart_policy(&mut self, restart_policy: Sourced<RestartPolicy>) {
        self.restart_policy = Some(restart_policy);
    }

    /// Returns the container-level automatic restart policy.
    #[must_use]
    pub const fn restart_policy(&self) -> Option<&Sourced<RestartPolicy>> {
        self.restart_policy.as_ref()
    }

    /// Sets the service health-check definition.
    pub fn set_healthcheck(&mut self, healthcheck: Sourced<Healthcheck>) {
        self.healthcheck = Some(healthcheck);
    }

    /// Returns the optional service health-check definition.
    #[must_use]
    pub const fn healthcheck(&self) -> Option<&Sourced<Healthcheck>> {
        self.healthcheck.as_ref()
    }

    /// Appends one service metadata label while preserving source order and provenance.
    pub fn add_label(&mut self, label: Sourced<MetadataLabel>) {
        self.labels.push(label);
    }

    /// Returns service metadata labels in source order.
    #[must_use]
    pub fn labels(&self) -> &[Sourced<MetadataLabel>] {
        &self.labels
    }

    /// Sets repeatable annotations while preserving omission separately from an explicit empty list.
    pub fn set_annotations(&mut self, annotations: Vec<Sourced<Annotation>>) {
        self.annotations = Some(annotations);
        self.annotations_origins.clear();
    }

    /// Appends one annotation in source order.
    pub fn add_annotation(&mut self, annotation: Sourced<Annotation>) {
        self.annotations.get_or_insert_default().push(annotation);
    }

    /// Sets repeatable annotations with collection-level provenance.
    pub fn set_annotations_with_origins(&mut self, annotations: Vec<Sourced<Annotation>>, origins: Vec<Provenance>) {
        self.annotations = Some(annotations);
        self.annotations_origins = origins;
    }

    /// Returns annotations, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn annotations(&self) -> Option<&[Sourced<Annotation>]> {
        self.annotations.as_deref()
    }

    /// Returns collection-level annotation provenance.
    #[must_use]
    pub fn annotations_origins(&self) -> &[Provenance] {
        &self.annotations_origins
    }

    /// Sets provider-specific logging intent.
    pub fn set_logging(&mut self, logging: Sourced<Logging>) {
        self.logging = Some(logging);
    }

    /// Returns provider-specific logging intent.
    #[must_use]
    pub const fn logging(&self) -> Option<&Sourced<Logging>> {
        self.logging.as_ref()
    }

    /// Sets the mutually exclusive reload action.
    pub fn set_reload_action(&mut self, reload_action: Sourced<ReloadAction>) {
        self.reload_action = Some(reload_action);
    }

    /// Returns the one explicit reload action, if any.
    #[must_use]
    pub const fn reload_action(&self) -> Option<&Sourced<ReloadAction>> {
        self.reload_action.as_ref()
    }

    /// Sets the primary identity used inside the service container.
    pub fn set_user(&mut self, user: Sourced<ProtectedString>) {
        self.user = Some(user);
    }

    /// Returns the primary identity used inside the service container.
    #[must_use]
    pub const fn user(&self) -> Option<&Sourced<ProtectedString>> {
        self.user.as_ref()
    }

    /// Sets the primary group used inside the service container.
    pub fn set_group(&mut self, group: Sourced<ProtectedString>) {
        self.group = Some(group);
    }

    /// Returns the primary group used inside the service container.
    #[must_use]
    pub const fn group(&self) -> Option<&Sourced<ProtectedString>> {
        self.group.as_ref()
    }

    /// Sets the requested user-namespace mode without imposing one runtime's grammar.
    pub fn set_user_namespace(&mut self, user_namespace: Sourced<ProtectedString>) {
        self.user_namespace = Some(user_namespace);
    }

    /// Returns the raw-preserving user-namespace mode.
    #[must_use]
    pub const fn user_namespace(&self) -> Option<&Sourced<ProtectedString>> {
        self.user_namespace.as_ref()
    }

    /// Appends one supplementary group in source order.
    pub fn add_supplementary_group(&mut self, group: Sourced<ProtectedString>) {
        self.supplementary_groups.push(group);
    }

    /// Returns supplementary groups in source order.
    #[must_use]
    pub fn supplementary_groups(&self) -> &[Sourced<ProtectedString>] {
        &self.supplementary_groups
    }

    /// Sets the working directory inside the service container.
    pub fn set_working_directory(&mut self, working_directory: Sourced<ProtectedString>) {
        self.working_directory = Some(working_directory);
    }

    /// Returns the working directory inside the service container.
    #[must_use]
    pub const fn working_directory(&self) -> Option<&Sourced<ProtectedString>> {
        self.working_directory.as_ref()
    }

    /// Sets the explicit read-only root-filesystem choice.
    pub fn set_read_only_root_filesystem(&mut self, read_only: Sourced<bool>) {
        self.read_only_root_filesystem = Some(read_only);
    }

    /// Returns the explicit read-only root-filesystem choice.
    #[must_use]
    pub const fn read_only_root_filesystem(&self) -> Option<&Sourced<bool>> {
        self.read_only_root_filesystem.as_ref()
    }

    /// Sets the explicit container hostname without inferring namespace ownership.
    pub fn set_hostname(&mut self, hostname: Sourced<ProtectedString>) {
        self.hostname = Some(hostname);
    }

    /// Returns the raw-preserving explicit hostname.
    #[must_use]
    pub const fn hostname(&self) -> Option<&Sourced<ProtectedString>> {
        self.hostname.as_ref()
    }

    /// Sets ordered DNS servers, preserving omission separately from an explicit empty list.
    pub fn set_dns_servers_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.dns_servers = Some(values);
        self.dns_servers_origins = origins;
    }

    /// Sets ordered DNS servers without separate collection provenance.
    pub fn set_dns_servers(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_dns_servers_with_origins(values, Vec::new());
    }

    /// Returns ordered DNS servers when explicitly authored.
    #[must_use]
    pub fn dns_servers(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.dns_servers.as_deref()
    }

    /// Returns collection provenance for explicitly authored DNS servers.
    #[must_use]
    pub fn dns_servers_origins(&self) -> &[Provenance] {
        &self.dns_servers_origins
    }

    /// Sets ordered DNS resolver options, preserving omission separately from an explicit empty list.
    pub fn set_dns_options_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.dns_options = Some(values);
        self.dns_options_origins = origins;
    }

    /// Sets ordered DNS resolver options without separate collection provenance.
    pub fn set_dns_options(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_dns_options_with_origins(values, Vec::new());
    }

    /// Returns ordered DNS resolver options when explicitly authored.
    #[must_use]
    pub fn dns_options(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.dns_options.as_deref()
    }

    /// Returns collection provenance for explicitly authored DNS resolver options.
    #[must_use]
    pub fn dns_options_origins(&self) -> &[Provenance] {
        &self.dns_options_origins
    }

    /// Sets ordered DNS search domains, preserving omission separately from an explicit empty list.
    pub fn set_dns_search_domains_with_origins(
        &mut self,
        values: Vec<Sourced<ProtectedString>>,
        origins: Vec<Provenance>,
    ) {
        self.dns_search_domains = Some(values);
        self.dns_search_domains_origins = origins;
    }

    /// Sets ordered DNS search domains without separate collection provenance.
    pub fn set_dns_search_domains(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_dns_search_domains_with_origins(values, Vec::new());
    }

    /// Returns ordered DNS search domains when explicitly authored.
    #[must_use]
    pub fn dns_search_domains(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.dns_search_domains.as_deref()
    }

    /// Returns collection provenance for explicitly authored DNS search domains.
    #[must_use]
    pub fn dns_search_domains_origins(&self) -> &[Provenance] {
        &self.dns_search_domains_origins
    }

    /// Sets ordered security options, preserving omission separately from an explicit empty list.
    pub fn set_security_options_with_origins(
        &mut self,
        values: Vec<Sourced<SecurityOption>>,
        origins: Vec<Provenance>,
    ) {
        self.security_options = Some(values);
        self.security_options_origins = origins;
    }

    /// Sets ordered security options without separate collection provenance.
    pub fn set_security_options(&mut self, values: Vec<Sourced<SecurityOption>>) {
        self.set_security_options_with_origins(values, Vec::new());
    }

    /// Returns ordered security options when explicitly authored.
    #[must_use]
    pub fn security_options(&self) -> Option<&[Sourced<SecurityOption>]> {
        self.security_options.as_deref()
    }

    /// Returns collection provenance for explicitly authored security options.
    #[must_use]
    pub fn security_options_origins(&self) -> &[Provenance] {
        &self.security_options_origins
    }

    /// Sets the raw process-ID limit spelling.
    pub fn set_pids_limit(&mut self, limit: Sourced<ProtectedString>) {
        self.pids_limit = Some(limit);
    }

    /// Returns the raw process-ID limit spelling.
    #[must_use]
    pub const fn pids_limit(&self) -> Option<&Sourced<ProtectedString>> {
        self.pids_limit.as_ref()
    }

    /// Sets the raw shared-memory size spelling.
    pub fn set_shm_size(&mut self, size: Sourced<ProtectedString>) {
        self.shm_size = Some(size);
    }

    /// Returns the raw shared-memory size spelling.
    #[must_use]
    pub const fn shm_size(&self) -> Option<&Sourced<ProtectedString>> {
        self.shm_size.as_ref()
    }

    /// Sets the complete ordered capability-add collection; `Some([])` retains an explicit reset.
    pub fn set_cap_add(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.cap_add = Some(values);
        self.cap_add_origins.clear();
    }

    /// Sets capability additions with collection-level provenance for an explicit empty/reset value.
    pub fn set_cap_add_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.cap_add = Some(values);
        self.cap_add_origins = origins;
    }

    /// Returns capability additions, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn cap_add(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.cap_add.as_deref()
    }

    /// Returns the collection-level capability-add provenance.
    #[must_use]
    pub fn cap_add_origins(&self) -> &[Provenance] {
        &self.cap_add_origins
    }

    /// Sets the complete ordered capability-drop collection; `Some([])` retains an explicit reset.
    pub fn set_cap_drop(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.cap_drop = Some(values);
        self.cap_drop_origins.clear();
    }

    /// Sets capability removals with collection-level provenance for an explicit empty/reset value.
    pub fn set_cap_drop_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.cap_drop = Some(values);
        self.cap_drop_origins = origins;
    }

    /// Returns capability removals, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn cap_drop(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.cap_drop.as_deref()
    }

    /// Returns the collection-level capability-drop provenance.
    #[must_use]
    pub fn cap_drop_origins(&self) -> &[Provenance] {
        &self.cap_drop_origins
    }

    /// Sets ordered raw temporary-filesystem declarations.
    pub fn set_tmpfs(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.tmpfs = Some(values);
        self.tmpfs_origins.clear();
    }

    /// Sets temporary filesystems with collection-level provenance.
    pub fn set_tmpfs_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.tmpfs = Some(values);
        self.tmpfs_origins = origins;
    }

    /// Returns temporary-filesystem declarations, preserving explicit-empty state.
    #[must_use]
    pub fn tmpfs(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.tmpfs.as_deref()
    }

    /// Returns collection-level temporary-filesystem provenance.
    #[must_use]
    pub fn tmpfs_origins(&self) -> &[Provenance] {
        &self.tmpfs_origins
    }

    /// Sets ordered raw kernel-parameter assignments.
    pub fn set_sysctls(&mut self, values: Vec<Sourced<KernelParameter>>) {
        self.sysctls = Some(values);
        self.sysctls_origins.clear();
    }

    /// Sets kernel parameters with collection-level provenance.
    pub fn set_sysctls_with_origins(&mut self, values: Vec<Sourced<KernelParameter>>, origins: Vec<Provenance>) {
        self.sysctls = Some(values);
        self.sysctls_origins = origins;
    }

    /// Returns kernel-parameter assignments, preserving explicit-empty state.
    #[must_use]
    pub fn sysctls(&self) -> Option<&[Sourced<KernelParameter>]> {
        self.sysctls.as_deref()
    }

    /// Returns collection-level kernel-parameter provenance.
    #[must_use]
    pub fn sysctls_origins(&self) -> &[Provenance] {
        &self.sysctls_origins
    }

    /// Sets ordered resource limits.
    pub fn set_ulimits(&mut self, values: Vec<Sourced<ResourceLimit>>) {
        self.ulimits = Some(values);
        self.ulimits_origins.clear();
    }

    /// Sets resource limits with collection-level provenance.
    pub fn set_ulimits_with_origins(&mut self, values: Vec<Sourced<ResourceLimit>>, origins: Vec<Provenance>) {
        self.ulimits = Some(values);
        self.ulimits_origins = origins;
    }

    /// Returns resource limits, preserving explicit-empty state.
    #[must_use]
    pub fn ulimits(&self) -> Option<&[Sourced<ResourceLimit>]> {
        self.ulimits.as_deref()
    }

    /// Returns collection-level resource-limit provenance.
    #[must_use]
    pub fn ulimits_origins(&self) -> &[Provenance] {
        &self.ulimits_origins
    }

    /// Sets ordered short/long device declarations.
    pub fn set_devices(&mut self, values: Vec<Sourced<Device>>) {
        self.devices = Some(values);
        self.devices_origins.clear();
    }

    /// Sets devices with collection-level provenance.
    pub fn set_devices_with_origins(&mut self, values: Vec<Sourced<Device>>, origins: Vec<Provenance>) {
        self.devices = Some(values);
        self.devices_origins = origins;
    }

    /// Returns device declarations, preserving explicit-empty state.
    #[must_use]
    pub fn devices(&self) -> Option<&[Sourced<Device>]> {
        self.devices.as_deref()
    }

    /// Returns collection-level device provenance.
    #[must_use]
    pub fn devices_origins(&self) -> &[Provenance] {
        &self.devices_origins
    }

    /// Sets the explicit stop-signal spelling.
    pub fn set_stop_signal(&mut self, signal: Sourced<ProtectedString>) {
        self.stop_signal = Some(signal);
    }

    /// Returns the raw explicit stop-signal spelling.
    #[must_use]
    pub const fn stop_signal(&self) -> Option<&Sourced<ProtectedString>> {
        self.stop_signal.as_ref()
    }

    /// Sets ordered source-authored Podman arguments, retaining an explicit empty collection.
    pub fn set_podman_args(&mut self, values: Vec<Sourced<ProtectedString>>) {
        self.set_podman_args_with_origins(values, Vec::new());
    }

    /// Sets ordered source-authored Podman arguments with collection-level provenance.
    pub fn set_podman_args_with_origins(&mut self, values: Vec<Sourced<ProtectedString>>, origins: Vec<Provenance>) {
        self.podman_args = Some(values);
        self.podman_args_origins = origins;
    }

    /// Appends one protected source-authored Podman argument in source order.
    pub fn add_podman_arg(&mut self, value: Sourced<ProtectedString>) {
        self.podman_args.get_or_insert_default().push(value);
    }

    /// Returns source-authored Podman arguments, preserving omitted versus explicit-empty state.
    #[must_use]
    pub fn podman_args(&self) -> Option<&[Sourced<ProtectedString>]> {
        self.podman_args.as_deref()
    }

    /// Returns collection-level Podman-argument provenance.
    #[must_use]
    pub fn podman_args_origins(&self) -> &[Provenance] {
        &self.podman_args_origins
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

    /// Appends an environment-file declaration without reading the referenced file.
    pub fn add_environment_file(&mut self, value: Sourced<EnvironmentFile>) {
        self.environment_files.push(value);
    }

    /// Returns environment-file declarations in authored order.
    #[must_use]
    pub fn environment_files(&self) -> &[Sourced<EnvironmentFile>] {
        &self.environment_files
    }

    /// Appends an explicit hostname-to-address mapping.
    pub fn add_host_mapping(&mut self, value: Sourced<HostMapping>) {
        self.host_mappings.push(value);
    }

    /// Returns explicit host mappings in authored order.
    #[must_use]
    pub fn host_mappings(&self) -> &[Sourced<HostMapping>] {
        &self.host_mappings
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

    /// Appends a configuration grant in source order.
    pub fn add_config_grant(&mut self, value: Sourced<ResourceGrant>) {
        self.config_grants.push(value);
    }

    /// Returns configuration grants in source order.
    #[must_use]
    pub fn config_grants(&self) -> &[Sourced<ResourceGrant>] {
        &self.config_grants
    }

    /// Appends a secret grant in source order.
    pub fn add_secret_grant(&mut self, value: Sourced<ResourceGrant>) {
        self.secret_grants.push(value);
    }

    /// Returns secret grants in source order.
    #[must_use]
    pub fn secret_grants(&self) -> &[Sourced<ResourceGrant>] {
        &self.secret_grants
    }

    /// Appends a network attachment.
    pub fn add_network(&mut self, value: Sourced<NetworkAttachment>) {
        self.networks.push(value);
    }

    /// Replaces one existing network attachment without changing authored order.
    ///
    /// Returns the previous attachment. This narrow mutation boundary lets importers enrich an
    /// attachment only after later native entries establish attachment-scoped details.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::UnknownNetworkAttachmentIndex`] when `index` is outside the ordered
    /// attachment collection.
    pub fn replace_network(
        &mut self,
        index: usize,
        value: Sourced<NetworkAttachment>,
    ) -> Result<Sourced<NetworkAttachment>, ModelError> {
        let len = self.networks.len();
        let Some(slot) = self.networks.get_mut(index) else {
            return Err(ModelError::UnknownNetworkAttachmentIndex { index, len });
        };
        Ok(std::mem::replace(slot, value))
    }

    /// Returns network attachments in authored order.
    #[must_use]
    pub fn networks(&self) -> &[Sourced<NetworkAttachment>] {
        &self.networks
    }

    /// Appends a service dependency in source order.
    pub fn add_dependency(&mut self, value: Sourced<ServiceDependency>) {
        self.dependencies.push(value);
    }

    /// Returns service dependencies in source order.
    #[must_use]
    pub fn dependencies(&self) -> &[Sourced<ServiceDependency>] {
        &self.dependencies
    }

    /// Validates that a root filesystem was not combined with any image source.
    ///
    /// This is public so adapters that incrementally map services can surface an invalid native
    /// combination before inserting it into an [`Application`].
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::RootfsImageSourceConflict`] when both forms are present.
    pub fn validate_image_source_exclusivity(&self) -> Result<(), ModelError> {
        if self.rootfs.is_some() {
            self.ensure_rootfs_is_compatible()?;
        }
        Ok(())
    }

    fn ensure_rootfs_is_compatible(&self) -> Result<(), ModelError> {
        let source = if self.image.is_some() {
            Some("image")
        } else if self.image_acquisition.is_some() {
            Some("image acquisition")
        } else if self.image_build.is_some() {
            Some("image build")
        } else {
            None
        };
        if let Some(source) = source {
            return Err(ModelError::RootfsImageSourceConflict {
                service: self.name.as_str().to_owned(),
                source,
            });
        }
        Ok(())
    }
}

/// One ordered multi-service application graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    name: Identifier,
    image_acquisitions: Vec<Sourced<ImageAcquisition>>,
    image_builds: Vec<Sourced<ImageBuild>>,
    services: Vec<Sourced<Service>>,
    service_groups: Vec<Sourced<ServiceGroup>>,
    volumes: Vec<Sourced<Volume>>,
    networks: Vec<Sourced<Network>>,
    configs: Vec<Sourced<Config>>,
    secrets: Vec<Sourced<Secret>>,
}

impl Application {
    /// Creates an empty application.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            image_acquisitions: Vec::new(),
            image_builds: Vec::new(),
            services: Vec::new(),
            service_groups: Vec::new(),
            volumes: Vec::new(),
            networks: Vec::new(),
            configs: Vec::new(),
            secrets: Vec::new(),
        }
    }

    /// Returns the application name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Adds a uniquely named image-acquisition resource while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate acquisition name.
    pub fn add_image_acquisition(&mut self, acquisition: Sourced<ImageAcquisition>) -> Result<(), ModelError> {
        ensure_unique(
            "image acquisition",
            acquisition.value().name(),
            self.image_acquisitions.iter().map(|candidate| candidate.value().name()),
        )?;
        self.image_acquisitions.push(acquisition);
        Ok(())
    }

    /// Returns image-acquisition resources in declaration order.
    #[must_use]
    pub fn image_acquisitions(&self) -> &[Sourced<ImageAcquisition>] {
        &self.image_acquisitions
    }

    /// Adds a uniquely named image-build resource while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate build name.
    pub fn add_image_build(&mut self, build: Sourced<ImageBuild>) -> Result<(), ModelError> {
        ensure_unique(
            "image build",
            build.value().name(),
            self.image_builds.iter().map(|candidate| candidate.value().name()),
        )?;
        self.image_builds.push(build);
        Ok(())
    }

    /// Returns image-build resources in declaration order.
    #[must_use]
    pub fn image_builds(&self) -> &[Sourced<ImageBuild>] {
        &self.image_builds
    }

    /// Validates every typed image-artifact reference after the complete graph is assembled.
    ///
    /// Unlike incremental insertion, this validation does not make a source document's
    /// declaration order significant. Adapters that receive forward references should add their
    /// resources first and invoke this method before treating the application as convertible.
    ///
    /// # Errors
    ///
    /// Returns the matching unknown-reference error for a service or image-backed volume.
    pub fn validate_image_artifact_references(&self) -> Result<(), ModelError> {
        for service in &self.services {
            if let Some(acquisition) = service.value().image_acquisition() {
                if !self.contains_image_acquisition(acquisition.value()) {
                    return Err(ModelError::UnknownImageAcquisitionReference {
                        service: service.value().name().as_str().to_owned(),
                        acquisition: acquisition.value().as_str().to_owned(),
                    });
                }
            }
            if let Some(build) = service.value().image_build() {
                if !self.contains_image_build(build.value()) {
                    return Err(ModelError::UnknownImageBuildReference {
                        service: service.value().name().as_str().to_owned(),
                        build: build.value().as_str().to_owned(),
                    });
                }
            }
        }
        for volume in &self.volumes {
            let Some(source) = volume.value().image_source() else {
                continue;
            };
            match source.value() {
                VolumeImageSource::Literal(_) => {}
                VolumeImageSource::ImageAcquisition(acquisition) => {
                    if !self.contains_image_acquisition(acquisition) {
                        return Err(ModelError::UnknownVolumeImageAcquisitionReference {
                            volume: volume.value().name().as_str().to_owned(),
                            acquisition: acquisition.as_str().to_owned(),
                        });
                    }
                }
                VolumeImageSource::ImageBuild(build) => {
                    if !self.contains_image_build(build) {
                        return Err(ModelError::UnknownVolumeImageBuildReference {
                            volume: volume.value().name().as_str().to_owned(),
                            build: build.as_str().to_owned(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Validates explicit format-neutral artifact edges for missing nodes and cycles.
    ///
    /// The supplied edges must already be typed by a source adapter. In particular, `BoxFerry` does
    /// not parse native raw argument, mount, or unit-name text to infer dependencies. Duplicate
    /// edges are ignored deterministically, and input order does not affect validation.
    ///
    /// # Errors
    ///
    /// Returns a missing-reference error, [`ModelError::UnknownArtifactDependencyNode`], or
    /// [`ModelError::ImageArtifactDependencyCycle`].
    pub fn validate_image_artifact_dependencies(
        &self,
        dependencies: &[Sourced<ArtifactDependency>],
    ) -> Result<(), ModelError> {
        self.validate_image_artifact_references()?;

        let mut graph = BTreeMap::<ArtifactDependencyNode, BTreeSet<ArtifactDependencyNode>>::new();
        for dependency in dependencies {
            let source = dependency.value().source().value();
            let target = dependency.value().target().value();
            self.validate_artifact_dependency_node(source)?;
            self.validate_artifact_dependency_node(target)?;
            graph.entry(source.clone()).or_default().insert(target.clone());
            graph.entry(target.clone()).or_default();
        }

        let mut state = BTreeMap::<ArtifactDependencyNode, VisitState>::new();
        let mut path = Vec::new();
        for node in graph.keys() {
            if state.get(node).is_some_and(|state| *state == VisitState::Finished) {
                continue;
            }
            if let Some(cycle) = detect_artifact_cycle(node, &graph, &mut state, &mut path) {
                return Err(ModelError::ImageArtifactDependencyCycle {
                    nodes: cycle.into_iter().map(|node| node.display_name()).collect(),
                });
            }
        }
        Ok(())
    }

    fn contains_image_acquisition(&self, name: &Identifier) -> bool {
        self.image_acquisitions
            .iter()
            .any(|candidate| candidate.value().name() == name)
    }

    fn contains_image_build(&self, name: &Identifier) -> bool {
        self.image_builds
            .iter()
            .any(|candidate| candidate.value().name() == name)
    }

    fn validate_artifact_dependency_node(&self, node: &ArtifactDependencyNode) -> Result<(), ModelError> {
        let (kind, name) = node.kind_and_name();
        let exists = match node {
            ArtifactDependencyNode::Volume(_) => self.volumes.iter().any(|volume| volume.value().name() == name),
            ArtifactDependencyNode::ImageAcquisition(_) => self.contains_image_acquisition(name),
            ArtifactDependencyNode::ImageBuild(_) => self.contains_image_build(name),
        };
        if exists {
            Ok(())
        } else {
            Err(ModelError::UnknownArtifactDependencyNode {
                kind,
                name: name.as_str().to_owned(),
            })
        }
    }

    /// Adds a uniquely named service while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate service name,
    /// [`ModelError::UnknownImageAcquisitionReference`], or
    /// [`ModelError::UnknownImageBuildReference`] for an unresolved artifact reference.
    pub fn add_service(&mut self, service: Sourced<Service>) -> Result<(), ModelError> {
        ensure_unique(
            "service",
            service.value().name(),
            self.services.iter().map(|candidate| candidate.value().name()),
        )?;
        service.value().validate_image_source_exclusivity()?;
        if let Some(acquisition) = service.value().image_acquisition() {
            if !self
                .image_acquisitions
                .iter()
                .any(|candidate| candidate.value().name() == acquisition.value())
            {
                return Err(ModelError::UnknownImageAcquisitionReference {
                    service: service.value().name().as_str().to_owned(),
                    acquisition: acquisition.value().as_str().to_owned(),
                });
            }
        }
        if let Some(build) = service.value().image_build() {
            if !self
                .image_builds
                .iter()
                .any(|candidate| candidate.value().name() == build.value())
            {
                return Err(ModelError::UnknownImageBuildReference {
                    service: service.value().name().as_str().to_owned(),
                    build: build.value().as_str().to_owned(),
                });
            }
        }
        self.services.push(service);
        Ok(())
    }

    /// Returns services in declaration order.
    #[must_use]
    pub fn services(&self) -> &[Sourced<Service>] {
        &self.services
    }

    /// Adds a uniquely named structural service group.
    ///
    /// Every referenced service must already exist in the application, and one service may belong
    /// to at most one group.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`], [`ModelError::UnknownServiceGroupMember`], or
    /// [`ModelError::ServiceInMultipleGroups`] when a relationship is ambiguous.
    pub fn add_service_group(&mut self, group: Sourced<ServiceGroup>) -> Result<(), ModelError> {
        ensure_unique(
            "service group",
            group.value().name(),
            self.service_groups.iter().map(|candidate| candidate.value().name()),
        )?;
        for member in group.value().members() {
            if !self
                .services
                .iter()
                .any(|service| service.value().name() == member.value())
            {
                return Err(ModelError::UnknownServiceGroupMember {
                    group: group.value().name().as_str().to_owned(),
                    service: member.value().as_str().to_owned(),
                });
            }
            if let Some(existing) = self.service_groups.iter().find(|candidate| {
                candidate
                    .value()
                    .members()
                    .iter()
                    .any(|candidate_member| candidate_member.value() == member.value())
            }) {
                return Err(ModelError::ServiceInMultipleGroups {
                    service: member.value().as_str().to_owned(),
                    existing: existing.value().name().as_str().to_owned(),
                    replacement: group.value().name().as_str().to_owned(),
                });
            }
        }
        self.service_groups.push(group);
        Ok(())
    }

    /// Returns structural service groups in source order.
    #[must_use]
    pub fn service_groups(&self) -> &[Sourced<ServiceGroup>] {
        &self.service_groups
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

    /// Adds a uniquely named configuration while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate configuration name.
    pub fn add_config(&mut self, config: Sourced<Config>) -> Result<(), ModelError> {
        ensure_unique(
            "config",
            config.value().name(),
            self.configs.iter().map(|candidate| candidate.value().name()),
        )?;
        self.configs.push(config);
        Ok(())
    }

    /// Returns configuration resources in declaration order.
    #[must_use]
    pub fn configs(&self) -> &[Sourced<Config>] {
        &self.configs
    }

    /// Adds a uniquely named secret while preserving declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::DuplicateResource`] for a duplicate secret name.
    pub fn add_secret(&mut self, secret: Sourced<Secret>) -> Result<(), ModelError> {
        ensure_unique(
            "secret",
            secret.value().name(),
            self.secrets.iter().map(|candidate| candidate.value().name()),
        )?;
        self.secrets.push(secret);
        Ok(())
    }

    /// Returns secret resources in declaration order.
    #[must_use]
    pub fn secrets(&self) -> &[Sourced<Secret>] {
        &self.secrets
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Finished,
}

fn detect_artifact_cycle(
    node: &ArtifactDependencyNode,
    graph: &BTreeMap<ArtifactDependencyNode, BTreeSet<ArtifactDependencyNode>>,
    state: &mut BTreeMap<ArtifactDependencyNode, VisitState>,
    path: &mut Vec<ArtifactDependencyNode>,
) -> Option<Vec<ArtifactDependencyNode>> {
    if state.get(node).is_some_and(|state| *state == VisitState::Visiting) {
        let index = path.iter().position(|candidate| candidate == node)?;
        let mut cycle = path[index..].to_vec();
        cycle.push(node.clone());
        return Some(cycle);
    }
    if state.get(node).is_some_and(|state| *state == VisitState::Finished) {
        return None;
    }

    state.insert(node.clone(), VisitState::Visiting);
    path.push(node.clone());
    if let Some(targets) = graph.get(node) {
        for target in targets {
            if let Some(cycle) = detect_artifact_cycle(target, graph, state, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    state.insert(node.clone(), VisitState::Finished);
    None
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
    validate_no_nul(kind, value)
}

fn validate_no_nul(kind: &'static str, value: &str) -> Result<(), ModelError> {
    if value.contains('\0') {
        return Err(ModelError::ContainsNul(kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Annotation, Application, ArtifactDependency, ArtifactDependencyNode, Command, Config, ConfigMaterial, Device,
        Entrypoint, EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax, ExposedPort, GroupExitPolicy,
        HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier,
        KernelParameter, Logging, LoggingOption, MetadataLabel, ModelError, Mount, MountSource, Network,
        NetworkAttachment, NetworkDriverOption, NetworkIpamConfig, Protocol, PullPolicy, ReloadAction, ResourceGrant,
        ResourceGrantSyntax, ResourceLimit, ResourceOwnership, RestartPolicy, Secret, SecretMaterial, SecurityOption,
        Service, ServiceDependency, ServiceDependencyCondition, ServiceGroup, ServiceGroupRuntime, StartupNotification,
        StopTimeout, Volume, VolumeImageSource,
    };
    use crate::{ImageAcquisition, ImageBuild, ImageReference, ProtectedString, Sourced};

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

    #[test]
    fn keeps_the_service_key_and_explicit_runtime_name_distinct() -> Result<(), String> {
        let mut service = Service::new(id("web")?);
        service.set_runtime_name(Sourced::generated(ProtectedString::plain("production-web")));

        assert_eq!(service.name().as_str(), "web");
        assert_eq!(
            service.runtime_name().map(|name| name.value().expose()),
            Some("production-web")
        );
        Ok(())
    }

    #[test]
    fn network_keeps_logical_and_runtime_names_and_literal_flags_distinct() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut network = Network::new(id("frontend")?, ResourceOwnership::Application);
        network.set_runtime_name(Sourced::from_source(
            ProtectedString::plain("production-frontend"),
            origin.clone(),
        ));
        network.set_driver(Sourced::from_source(ProtectedString::plain("bridge"), origin.clone()));
        network.set_internal(Sourced::from_source(true, origin.clone()));
        network.set_ipv6(Sourced::from_source(false, origin.clone()));
        network.set_ipam_driver(Sourced::from_source(ProtectedString::plain("default"), origin));

        assert_eq!(network.name().as_str(), "frontend");
        assert_eq!(
            network.runtime_name().map(|value| value.value().expose()),
            Some("production-frontend")
        );
        assert_eq!(network.driver().map(|value| value.value().expose()), Some("bridge"));
        assert_eq!(network.internal().map(Sourced::value), Some(&true));
        assert_eq!(network.ipv6().map(Sourced::value), Some(&false));
        assert_eq!(
            network.ipam_driver().map(|value| value.value().expose()),
            Some("default")
        );
        Ok(())
    }

    #[test]
    fn network_collections_retain_resets_provenance_and_redact_protected_values() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut network = Network::new(id("frontend")?, ResourceOwnership::Application);
        let option = NetworkDriverOption::new(
            Sourced::from_source(id("com.example.token")?, origin.clone()),
            Sourced::from_source(ProtectedString::sensitive("never-print-this"), origin.clone()),
        )
        .map_err(|error| error.to_string())?;
        let label = MetadataLabel::new(id("com.example.label")?, ProtectedString::sensitive("also-private"));

        network
            .set_driver_options_with_origins(vec![Sourced::from_source(option, origin.clone())], vec![origin.clone()]);
        network.set_labels_with_origins(vec![Sourced::from_source(label, origin.clone())], vec![origin.clone()]);
        network.set_ipam_configs_with_origins(Vec::new(), vec![origin]);

        assert_eq!(network.driver_options().map(<[_]>::len), Some(1));
        assert_eq!(network.labels().map(<[_]>::len), Some(1));
        assert_eq!(network.ipam_configs().map(<[_]>::len), Some(0));
        assert_eq!(network.driver_options_origins().len(), 1);
        assert_eq!(network.labels_origins().len(), 1);
        assert_eq!(network.ipam_configs_origins().len(), 1);
        let debug = format!("{network:?}");
        assert!(!debug.contains("never-print-this"));
        assert!(!debug.contains("also-private"));
        assert!(debug.contains("[REDACTED]"));

        network.set_driver_options(Vec::new());
        network.set_labels(Vec::new());
        network.set_ipam_configs(Vec::new());
        assert_eq!(network.driver_options().map(<[_]>::len), Some(0));
        assert_eq!(network.labels().map(<[_]>::len), Some(0));
        assert_eq!(network.ipam_configs().map(<[_]>::len), Some(0));
        assert!(network.driver_options_origins().is_empty());
        assert!(network.labels_origins().is_empty());
        assert!(network.ipam_configs_origins().is_empty());
        Ok(())
    }

    #[test]
    fn network_ipam_rows_preserve_association_order_and_reject_subnetless_values() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut first = NetworkIpamConfig::new(Sourced::from_source(
            ProtectedString::plain("10.10.0.0/24"),
            origin.clone(),
        ))
        .map_err(|error| error.to_string())?;
        first
            .set_gateway(Sourced::from_source(
                ProtectedString::plain("10.10.0.1"),
                origin.clone(),
            ))
            .map_err(|error| error.to_string())?;
        let mut second = NetworkIpamConfig::new(Sourced::from_source(
            ProtectedString::plain("fd00:10::/64"),
            origin.clone(),
        ))
        .map_err(|error| error.to_string())?;
        second
            .set_ip_range(Sourced::from_source(
                ProtectedString::plain("fd00:10::100/120"),
                origin.clone(),
            ))
            .map_err(|error| error.to_string())?;

        let mut network = Network::new(id("frontend")?, ResourceOwnership::Application);
        network.set_ipam_configs_with_origins(
            vec![
                Sourced::from_source(first, origin.clone()),
                Sourced::from_source(second, origin),
            ],
            Vec::new(),
        );
        let rows = network
            .ipam_configs()
            .ok_or_else(|| "IPAM configs were omitted".to_owned())?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].value().subnet().value().expose(), "10.10.0.0/24");
        assert_eq!(
            rows[0].value().gateway().map(|value| value.value().expose()),
            Some("10.10.0.1")
        );
        assert_eq!(rows[0].value().ip_range(), None);
        assert_eq!(rows[1].value().subnet().value().expose(), "fd00:10::/64");
        assert_eq!(rows[1].value().gateway(), None);
        assert_eq!(
            rows[1].value().ip_range().map(|value| value.value().expose()),
            Some("fd00:10::100/120")
        );

        assert!(matches!(
            NetworkIpamConfig::new(Sourced::generated(ProtectedString::plain(""))),
            Err(ModelError::EmptyValue("network IPAM subnet"))
        ));
        assert!(matches!(
            NetworkIpamConfig::new(Sourced::generated(ProtectedString::plain("10.0.0.0/24\0bad"))),
            Err(ModelError::ContainsNul("network IPAM subnet"))
        ));
        assert!(matches!(
            NetworkDriverOption::new(
                Sourced::generated(id("option")?),
                Sourced::generated(ProtectedString::plain("bad\0value")),
            ),
            Err(ModelError::ContainsNul("network driver option value"))
        ));
        Ok(())
    }

    #[test]
    fn image_artifact_resources_are_ordered_unique_and_referenced_explicitly() -> Result<(), String> {
        let mut application = Application::new(id("example")?);
        application
            .add_image_acquisition(Sourced::generated(ImageAcquisition::new(id("base-image")?)))
            .map_err(|error| error.to_string())?;
        application
            .add_image_build(Sourced::generated(ImageBuild::new(id("web-build")?)))
            .map_err(|error| error.to_string())?;

        let mut web = Service::new(id("web")?);
        web.set_image_acquisition(Sourced::generated(id("base-image")?));
        web.set_image_build(Sourced::generated(id("web-build")?));
        application
            .add_service(Sourced::generated(web))
            .map_err(|error| error.to_string())?;

        assert_eq!(
            application.image_acquisitions()[0].value().name().as_str(),
            "base-image"
        );
        assert_eq!(application.image_builds()[0].value().name().as_str(), "web-build");
        assert!(matches!(
            application.add_image_build(Sourced::generated(ImageBuild::new(id("web-build")?))),
            Err(ModelError::DuplicateResource {
                kind: "image build",
                ..
            })
        ));

        let mut missing = Service::new(id("missing")?);
        missing.set_image_build(Sourced::generated(id("absent-build")?));
        assert!(matches!(
            application.add_service(Sourced::generated(missing)),
            Err(ModelError::UnknownImageBuildReference { .. })
        ));
        Ok(())
    }

    #[test]
    fn volume_keeps_logical_runtime_and_service_names_and_local_fields_distinct() -> Result<(), String> {
        let origin = crate::Provenance::source(crate::SourceId::new("data.volume").map_err(|error| error.to_string())?);
        let mut volume = Volume::new(id("data")?, ResourceOwnership::Application);
        volume.set_runtime_name(Sourced::from_source(
            ProtectedString::plain("production-data"),
            origin.clone(),
        ));
        volume.set_service_name(Sourced::from_source(
            ProtectedString::plain("data-volume.service"),
            origin.clone(),
        ));
        volume.set_driver(Sourced::from_source(ProtectedString::plain("local"), origin.clone()));
        volume.set_device(Sourced::from_source(
            ProtectedString::plain("/srv/data"),
            origin.clone(),
        ));
        volume.set_volume_type(Sourced::from_source(ProtectedString::plain("none"), origin.clone()));
        volume.set_options(Sourced::from_source(ProtectedString::plain("bind"), origin.clone()));

        assert_eq!(volume.name().as_str(), "data");
        assert_eq!(
            volume.runtime_name().map(|name| name.value().expose()),
            Some("production-data")
        );
        assert_eq!(
            volume.service_name().map(|name| name.value().expose()),
            Some("data-volume.service")
        );
        assert_eq!(volume.driver().map(|value| value.value().expose()), Some("local"));
        assert_eq!(volume.device().map(|value| value.value().expose()), Some("/srv/data"));
        assert_eq!(volume.volume_type().map(|value| value.value().expose()), Some("none"));
        assert_eq!(volume.options().map(|value| value.value().expose()), Some("bind"));
        assert_eq!(
            volume.options().map(Sourced::origins),
            Some(std::slice::from_ref(&origin))
        );
        Ok(())
    }

    #[test]
    fn volume_preserves_resets_order_protected_values_and_identity_dimensions() -> Result<(), String> {
        let origin = crate::Provenance::source(crate::SourceId::new("data.volume").map_err(|error| error.to_string())?);
        let mut volume = Volume::new(id("data")?, ResourceOwnership::Application);
        assert!(volume.labels().is_none());
        assert!(volume.containers_conf_modules().is_none());
        assert!(volume.global_args().is_none());
        assert!(volume.podman_args().is_none());
        volume.set_labels_with_origins(Vec::new(), vec![origin.clone()]);
        volume.set_containers_conf_modules_with_origins(Vec::new(), vec![origin.clone()]);
        volume.set_global_args_with_origins(
            vec![
                Sourced::from_source(ProtectedString::plain("--first"), origin.clone()),
                Sourced::from_source(ProtectedString::sensitive("--token=never-print"), origin.clone()),
            ],
            vec![origin.clone()],
        );
        volume.set_podman_args_with_origins(
            vec![
                Sourced::from_source(ProtectedString::plain("--replace"), origin.clone()),
                Sourced::from_source(ProtectedString::sensitive("--secret=never-print"), origin.clone()),
            ],
            vec![origin.clone()],
        );
        volume.set_user(Sourced::from_source(
            ProtectedString::plain("named-user"),
            origin.clone(),
        ));
        volume.set_group(Sourced::from_source(
            ProtectedString::plain("named-group"),
            origin.clone(),
        ));
        volume.set_uid(Sourced::from_source(ProtectedString::plain("1001"), origin.clone()));
        volume.set_gid(Sourced::from_source(ProtectedString::plain("1002"), origin));

        assert_eq!(volume.labels().map(<[_]>::len), Some(0));
        assert_eq!(volume.containers_conf_modules().map(<[_]>::len), Some(0));
        assert_eq!(volume.global_args().map(<[_]>::len), Some(2));
        assert_eq!(volume.podman_args().map(<[_]>::len), Some(2));
        assert_eq!(volume.user().map(|value| value.value().expose()), Some("named-user"));
        assert_eq!(volume.group().map(|value| value.value().expose()), Some("named-group"));
        assert_eq!(volume.uid().map(|value| value.value().expose()), Some("1001"));
        assert_eq!(volume.gid().map(|value| value.value().expose()), Some("1002"));
        let debug = format!("{volume:?}");
        assert!(!debug.contains("never-print"));
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn volume_copy_and_image_sources_preserve_absence_and_typed_distinctions() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("cache.volume").map_err(|error| error.to_string())?);
        let mut volume = Volume::new(id("cache")?, ResourceOwnership::Application);
        assert_eq!(volume.copy(), None);
        volume.set_copy(Sourced::from_source(false, origin.clone()));
        assert_eq!(volume.copy().map(Sourced::value), Some(&false));
        volume.set_copy(Sourced::from_source(true, origin.clone()));
        assert_eq!(volume.copy().map(Sourced::value), Some(&true));

        volume
            .set_image_source(Sourced::from_source(
                VolumeImageSource::Literal(ProtectedString::sensitive("registry.example/private:1")),
                origin.clone(),
            ))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            volume.image_source().map(Sourced::value),
            Some(VolumeImageSource::Literal(_))
        ));
        assert!(!format!("{volume:?}").contains("registry.example/private:1"));

        volume
            .set_image_source(Sourced::from_source(
                VolumeImageSource::ImageAcquisition(id("cache-image")?),
                origin.clone(),
            ))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            volume.image_source().map(Sourced::value),
            Some(VolumeImageSource::ImageAcquisition(name)) if name.as_str() == "cache-image"
        ));
        volume
            .set_image_source(Sourced::from_source(
                VolumeImageSource::ImageBuild(id("cache-build")?),
                origin,
            ))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            volume.image_source().map(Sourced::value),
            Some(VolumeImageSource::ImageBuild(name)) if name.as_str() == "cache-build"
        ));
        Ok(())
    }

    #[test]
    fn volume_artifact_validation_is_deferred_and_explicit_edges_find_cycles() -> Result<(), String> {
        let mut application = Application::new(id("example")?);
        let mut volume = Volume::new(id("cache")?, ResourceOwnership::Application);
        volume
            .set_image_source(Sourced::generated(VolumeImageSource::ImageBuild(id("cache-build")?)))
            .map_err(|error| error.to_string())?;
        application
            .add_volume(Sourced::generated(volume))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            application.validate_image_artifact_references(),
            Err(ModelError::UnknownVolumeImageBuildReference { .. })
        ));

        application
            .add_image_build(Sourced::generated(ImageBuild::new(id("cache-build")?)))
            .map_err(|error| error.to_string())?;
        application
            .validate_image_artifact_references()
            .map_err(|error| error.to_string())?;

        let volume_node = ArtifactDependencyNode::Volume(id("cache")?);
        let build_node = ArtifactDependencyNode::ImageBuild(id("cache-build")?);
        let dependencies = vec![
            Sourced::generated(ArtifactDependency::new(
                Sourced::generated(volume_node.clone()),
                Sourced::generated(build_node.clone()),
            )),
            Sourced::generated(ArtifactDependency::new(
                Sourced::generated(build_node),
                Sourced::generated(volume_node),
            )),
        ];
        assert!(matches!(
            application.validate_image_artifact_dependencies(&dependencies),
            Err(ModelError::ImageArtifactDependencyCycle { .. })
        ));
        let missing = vec![Sourced::generated(ArtifactDependency::new(
            Sourced::generated(ArtifactDependencyNode::ImageBuild(id("cache-build")?)),
            Sourced::generated(ArtifactDependencyNode::Volume(id("missing")?)),
        ))];
        assert!(matches!(
            application.validate_image_artifact_dependencies(&missing),
            Err(ModelError::UnknownArtifactDependencyNode { kind: "volume", .. })
        ));
        Ok(())
    }

    #[test]
    fn volume_rejects_invalid_literal_image_values() -> Result<(), String> {
        let mut volume = Volume::new(id("data")?, ResourceOwnership::Application);
        assert!(matches!(
            volume.set_image_source(Sourced::generated(VolumeImageSource::Literal(ProtectedString::plain(
                ""
            )))),
            Err(ModelError::EmptyValue("volume image"))
        ));
        assert!(matches!(
            volume.set_image_source(Sourced::generated(VolumeImageSource::Literal(ProtectedString::plain(
                "bad\0image"
            )))),
            Err(ModelError::ContainsNul("volume image"))
        ));
        Ok(())
    }

    #[test]
    fn collection_resets_retain_explicit_emptiness_and_clear_stale_origins() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?);
        let mut service = Service::new(id("web")?);

        service.set_cap_add_with_origins(Vec::new(), vec![origin.clone()]);
        service.set_cap_drop_with_origins(Vec::new(), vec![origin.clone()]);
        service.set_tmpfs_with_origins(Vec::new(), vec![origin.clone()]);
        service.set_sysctls_with_origins(Vec::new(), vec![origin.clone()]);
        service.set_ulimits_with_origins(Vec::new(), vec![origin.clone()]);
        service.set_devices_with_origins(Vec::new(), vec![origin]);

        assert_eq!(service.cap_add().map(<[_]>::len), Some(0));
        assert_eq!(service.cap_drop().map(<[_]>::len), Some(0));
        assert_eq!(service.tmpfs().map(<[_]>::len), Some(0));
        assert_eq!(service.sysctls().map(<[_]>::len), Some(0));
        assert_eq!(service.ulimits().map(<[_]>::len), Some(0));
        assert_eq!(service.devices().map(<[_]>::len), Some(0));
        assert_eq!(service.cap_add_origins().len(), 1);
        assert_eq!(service.cap_drop_origins().len(), 1);
        assert_eq!(service.tmpfs_origins().len(), 1);
        assert_eq!(service.sysctls_origins().len(), 1);
        assert_eq!(service.ulimits_origins().len(), 1);
        assert_eq!(service.devices_origins().len(), 1);

        service.set_cap_add(Vec::new());
        service.set_cap_drop(Vec::new());
        service.set_tmpfs(Vec::new());
        service.set_sysctls(Vec::<Sourced<KernelParameter>>::new());
        service.set_ulimits(Vec::<Sourced<ResourceLimit>>::new());
        service.set_devices(Vec::<Sourced<Device>>::new());

        assert!(service.cap_add_origins().is_empty());
        assert!(service.cap_drop_origins().is_empty());
        assert!(service.tmpfs_origins().is_empty());
        assert!(service.sysctls_origins().is_empty());
        assert!(service.ulimits_origins().is_empty());
        assert!(service.devices_origins().is_empty());
        Ok(())
    }

    #[test]
    fn restart_policy_keeps_unlimited_and_finite_on_failure_distinct() {
        let finite = std::num::NonZeroU64::new(4);
        assert_eq!(RestartPolicy::on_failure(None).maximum_retries(), None);
        assert_eq!(RestartPolicy::on_failure(finite).maximum_retries(), finite);
        assert_eq!(RestartPolicy::Always.maximum_retries(), None);
    }

    #[test]
    fn metadata_labels_preserve_empty_and_protected_values() -> Result<(), String> {
        let empty = MetadataLabel::new(id("com.example.empty")?, ProtectedString::plain(""));
        let protected = MetadataLabel::new(id("com.example.token")?, ProtectedString::sensitive("never-print-this"));
        let mut service = Service::new(id("web")?);
        service.add_label(Sourced::generated(empty));
        service.add_label(Sourced::generated(protected));

        assert_eq!(service.labels()[0].value().value().expose(), "");
        let debug = format!("{:?}", service.labels()[1]);
        assert!(!debug.contains("never-print-this"));
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn environment_files_preserve_order_options_provenance_and_redaction() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut service = Service::new(id("web")?);
        service.add_environment_file(Sourced::from_source(
            EnvironmentFile::new(ProtectedString::plain("./base.env"), EnvironmentFileSyntax::Short)
                .map_err(|error| error.to_string())?,
            origin.clone(),
        ));
        let mut local = EnvironmentFile::new(ProtectedString::sensitive("./private.env"), EnvironmentFileSyntax::Long)
            .map_err(|error| error.to_string())?;
        local.set_required(Sourced::from_source(false, origin.clone()));
        local.set_format(Sourced::from_source(EnvironmentFileFormat::Raw, origin.clone()));
        service.add_environment_file(Sourced::from_source(local, origin));

        assert_eq!(service.environment_files().len(), 2);
        assert_eq!(service.environment_files()[0].value().path().expose(), "./base.env");
        assert_eq!(
            service.environment_files()[0].value().syntax(),
            EnvironmentFileSyntax::Short
        );
        assert!(service.environment_files()[0].value().is_required());
        let local = service.environment_files()[1].value();
        assert_eq!(local.syntax(), EnvironmentFileSyntax::Long);
        assert!(!local.is_required());
        assert_eq!(local.required().map_or(0, |value| value.origins().len()), 1);
        assert!(matches!(
            local.format().map(Sourced::value),
            Some(EnvironmentFileFormat::Raw)
        ));
        let debug = format!("{service:?}");
        assert!(!debug.contains("private.env"));
        assert!(debug.contains("[REDACTED]"));
        assert!(matches!(
            EnvironmentFile::new(ProtectedString::plain(""), EnvironmentFileSyntax::Short),
            Err(ModelError::EmptyValue("environment-file path"))
        ));
        Ok(())
    }

    #[test]
    fn service_groups_preserve_order_and_reject_ambiguous_membership() -> Result<(), String> {
        let mut application = Application::new(id("example")?);
        for name in ["web", "worker"] {
            application
                .add_service(Sourced::generated(Service::new(id(name)?)))
                .map_err(|error| error.to_string())?;
        }

        let mut frontend = ServiceGroup::new(id("frontend")?, ResourceOwnership::Uncertain);
        frontend
            .add_member(Sourced::generated(id("web")?))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            frontend.add_member(Sourced::generated(id("web")?)),
            Err(ModelError::DuplicateServiceGroupMember { .. })
        ));
        application
            .add_service_group(Sourced::generated(frontend))
            .map_err(|error| error.to_string())?;

        assert_eq!(application.service_groups()[0].value().name().as_str(), "frontend");
        assert_eq!(
            application.service_groups()[0].value().members()[0].value().as_str(),
            "web"
        );

        let mut conflicting = ServiceGroup::new(id("backend")?, ResourceOwnership::Application);
        conflicting
            .add_member(Sourced::generated(id("web")?))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            application.add_service_group(Sourced::generated(conflicting)),
            Err(ModelError::ServiceInMultipleGroups { .. })
        ));

        let mut missing = ServiceGroup::new(id("missing")?, ResourceOwnership::External);
        missing
            .add_member(Sourced::generated(id("database")?))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            application.add_service_group(Sourced::generated(missing)),
            Err(ModelError::UnknownServiceGroupMember { .. })
        ));
        Ok(())
    }

    #[test]
    fn group_runtime_keeps_group_names_and_pod_settings_distinct() -> Result<(), String> {
        let source = crate::SourceId::new("pod.pod").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut group = ServiceGroup::new(id("frontend")?, ResourceOwnership::Application);
        let mut runtime = ServiceGroupRuntime::new();
        runtime.set_runtime_name(Sourced::from_source(
            ProtectedString::plain("production-frontend"),
            origin.clone(),
        ));
        runtime.set_service_name(Sourced::from_source(
            ProtectedString::plain("frontend-pod"),
            origin.clone(),
        ));
        runtime.set_host_mappings_with_origins(
            vec![Sourced::from_source(
                HostMapping::new(
                    id("host.docker.internal")?,
                    HostAddress::new("host-gateway").map_err(|error| error.to_string())?,
                ),
                origin.clone(),
            )],
            vec![origin.clone()],
        );
        runtime.set_ports_with_origins(Vec::new(), vec![origin.clone()]);
        runtime.set_networks_with_origins(
            vec![Sourced::from_source(
                NetworkAttachment::new(
                    id("edge")?,
                    vec![Sourced::from_source(
                        ProtectedString::sensitive("private-alias"),
                        origin.clone(),
                    )],
                ),
                origin.clone(),
            )],
            vec![origin.clone()],
        );
        runtime.set_user_namespace(Sourced::from_source(ProtectedString::plain("keep-id"), origin.clone()));
        runtime.set_mounts_with_origins(
            vec![Sourced::from_source(
                Mount::new(MountSource::Anonymous, "/cache", false).map_err(|error| error.to_string())?,
                origin.clone(),
            )],
            vec![origin.clone()],
        );
        runtime.set_shm_size(Sourced::from_source(ProtectedString::sensitive("64m"), origin.clone()));
        runtime.set_exit_policy(Sourced::from_source(
            GroupExitPolicy::Raw(ProtectedString::sensitive("preserve-this")),
            origin.clone(),
        ));
        runtime.set_stop_timeout(Sourced::from_source(
            StopTimeout::new("30s").map_err(|error| error.to_string())?,
            origin.clone(),
        ));
        assert!(matches!(
            runtime.replace_network(1, Sourced::generated(NetworkAttachment::new(id("other")?, Vec::new()))),
            Err(ModelError::UnknownServiceGroupRuntimeNetworkIndex { index: 1, len: 1 })
        ));
        group.set_runtime(Sourced::from_source(runtime, origin));

        let runtime = group
            .runtime()
            .ok_or_else(|| "group runtime was omitted".to_owned())?
            .value();
        assert_eq!(group.name().as_str(), "frontend");
        assert_eq!(
            runtime.runtime_name().map(|name| name.value().expose()),
            Some("production-frontend")
        );
        assert_eq!(
            runtime.service_name().map(|name| name.value().expose()),
            Some("frontend-pod")
        );
        assert_eq!(runtime.host_mappings().map(<[_]>::len), Some(1));
        assert_eq!(runtime.ports().map(<[_]>::len), Some(0));
        assert_eq!(runtime.networks_origins().len(), 1);
        assert_eq!(runtime.mounts().map(<[_]>::len), Some(1));
        assert!(matches!(
            runtime.exit_policy().map(Sourced::value),
            Some(GroupExitPolicy::Raw(_))
        ));
        let debug = format!("{group:?}");
        for sensitive in ["private-alias", "64m", "preserve-this"] {
            assert!(!debug.contains(sensitive));
        }
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn rootfs_startup_notification_and_podman_args_preserve_safe_contracts() -> Result<(), String> {
        let source = crate::SourceId::new("web.container").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut service = Service::new(id("web")?);
        service.set_startup_notification(Sourced::from_source(StartupNotification::Healthy, origin.clone()));
        service.set_podman_args_with_origins(
            vec![
                Sourced::from_source(ProtectedString::plain("--replace"), origin.clone()),
                Sourced::from_source(ProtectedString::sensitive("--secret=never-print"), origin.clone()),
                Sourced::from_source(ProtectedString::plain("--replace"), origin.clone()),
            ],
            vec![origin.clone()],
        );
        assert_eq!(service.podman_args().map(<[_]>::len), Some(3));
        assert_eq!(service.podman_args_origins(), std::slice::from_ref(&origin));
        assert!(matches!(
            service.startup_notification().map(Sourced::value),
            Some(StartupNotification::Healthy)
        ));
        assert!(!format!("{service:?}").contains("never-print"));

        let mut with_image = Service::new(id("image-first")?);
        with_image.set_image(Sourced::generated(
            ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
        ));
        assert!(matches!(
            with_image.set_rootfs(Sourced::generated(ProtectedString::plain("/srv/rootfs"))),
            Err(ModelError::RootfsImageSourceConflict { source: "image", .. })
        ));

        let mut with_rootfs = Service::new(id("rootfs-first")?);
        with_rootfs
            .set_rootfs(Sourced::generated(ProtectedString::sensitive("/private/rootfs")))
            .map_err(|error| error.to_string())?;
        with_rootfs.set_image_build(Sourced::generated(id("web-build")?));
        let mut application = Application::new(id("example")?);
        application
            .add_image_build(Sourced::generated(ImageBuild::new(id("web-build")?)))
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            application.add_service(Sourced::generated(with_rootfs)),
            Err(ModelError::RootfsImageSourceConflict {
                source: "image build",
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn validates_raw_preserving_healthcheck_scalars() -> Result<(), String> {
        let duration = HealthcheckDuration::new("1m30s").map_err(|error| error.to_string())?;
        let retries = HealthcheckRetries::new("003").map_err(|error| error.to_string())?;
        assert_eq!(duration.as_str(), "1m30s");
        assert_eq!(retries.as_str(), "003");
        assert_eq!(
            HealthcheckRetries::new("three"),
            Err(ModelError::InvalidHealthcheckRetries)
        );
        assert!(matches!(
            HealthcheckDuration::new(""),
            Err(ModelError::EmptyValue("health-check duration"))
        ));
        Ok(())
    }

    #[test]
    fn preserves_ordered_dependency_edges_and_field_provenance() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut service = Service::new(id("web")?);

        let mut database = ServiceDependency::new(id("database")?);
        database.set_condition(Sourced::from_source(
            ServiceDependencyCondition::Healthy,
            origin.clone(),
        ));
        database.set_required(Sourced::from_source(true, origin.clone()));
        service.add_dependency(Sourced::from_source(database, origin.clone()));

        let cache = ServiceDependency::new(id("cache")?);
        assert!(cache.is_required());
        service.add_dependency(Sourced::from_source(cache, origin));

        assert_eq!(
            service
                .dependencies()
                .iter()
                .map(|dependency| dependency.value().service().as_str())
                .collect::<Vec<_>>(),
            ["database", "cache"]
        );
        assert!(matches!(
            service.dependencies()[0].value().condition().map(Sourced::value),
            Some(ServiceDependencyCondition::Healthy)
        ));
        assert_eq!(service.dependencies()[0].origins().len(), 1);
        assert_eq!(
            service.dependencies()[0]
                .value()
                .condition()
                .map_or(0, |condition| condition.origins().len()),
            1
        );
        Ok(())
    }

    #[test]
    fn retains_execution_identity_context_order_provenance_and_redaction() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut service = Service::new(id("web")?);

        service.set_user(Sourced::from_source(ProtectedString::sensitive("1001"), origin.clone()));
        service.set_group(Sourced::from_source(ProtectedString::plain("1002"), origin.clone()));
        service.set_user_namespace(Sourced::from_source(ProtectedString::plain("keep-id"), origin.clone()));
        service.add_supplementary_group(Sourced::from_source(ProtectedString::plain("audio"), origin.clone()));
        service.add_supplementary_group(Sourced::from_source(ProtectedString::plain("44"), origin.clone()));
        service.set_working_directory(Sourced::from_source(ProtectedString::plain("/srv/app"), origin.clone()));
        service.set_read_only_root_filesystem(Sourced::from_source(true, origin));

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
                .map(|group| group.value().expose())
                .collect::<Vec<_>>(),
            ["audio", "44"]
        );
        assert_eq!(
            service.working_directory().map(|value| value.value().expose()),
            Some("/srv/app")
        );
        assert_eq!(service.read_only_root_filesystem().map(Sourced::value), Some(&true));
        assert_eq!(service.user().map_or(0, |value| value.origins().len()), 1);
        let debug = format!("{service:?}");
        assert!(!debug.contains("1001"));
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn retains_config_secret_resources_grants_provenance_and_redaction() -> Result<(), String> {
        let source = crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let origin = crate::Provenance::source(source);
        let mut application = Application::new(id("example")?);

        let mut config = Config::new(id("settings")?, ResourceOwnership::Application);
        config.set_material(Sourced::from_source(
            ConfigMaterial::Content(ProtectedString::sensitive("private-config")),
            origin.clone(),
        ));
        application
            .add_config(Sourced::from_source(config, origin.clone()))
            .map_err(|error| error.to_string())?;

        let mut secret = Secret::new(id("password")?, ResourceOwnership::External);
        secret.set_runtime_name(Sourced::from_source(
            ProtectedString::plain("production-password"),
            origin.clone(),
        ));
        secret.set_material(Sourced::from_source(
            SecretMaterial::Environment(ProtectedString::sensitive("private-environment-name")),
            origin.clone(),
        ));
        application
            .add_secret(Sourced::from_source(secret, origin.clone()))
            .map_err(|error| error.to_string())?;

        let mut service = Service::new(id("web")?);
        service.add_config_grant(Sourced::from_source(
            ResourceGrant::new(ProtectedString::plain("settings"), ResourceGrantSyntax::Short)
                .map_err(|error| error.to_string())?,
            origin.clone(),
        ));
        let mut secret_grant = ResourceGrant::new(
            ProtectedString::sensitive("private-grant-source"),
            ResourceGrantSyntax::Long,
        )
        .map_err(|error| error.to_string())?;
        secret_grant.set_target(Sourced::from_source(
            ProtectedString::plain("database-password"),
            origin.clone(),
        ));
        secret_grant.set_uid(Sourced::from_source(ProtectedString::plain("1001"), origin.clone()));
        secret_grant.set_gid(Sourced::from_source(ProtectedString::plain("1002"), origin.clone()));
        secret_grant.set_mode(Sourced::from_source(ProtectedString::plain("0440"), origin.clone()));
        service.add_secret_grant(Sourced::from_source(secret_grant, origin.clone()));
        application
            .add_service(Sourced::from_source(service, origin))
            .map_err(|error| error.to_string())?;

        assert_eq!(application.configs().len(), 1);
        assert_eq!(application.secrets().len(), 1);
        assert_eq!(application.services()[0].value().config_grants().len(), 1);
        let grant = &application.services()[0].value().secret_grants()[0];
        assert_eq!(grant.value().syntax(), ResourceGrantSyntax::Long);
        assert_eq!(
            grant.value().target().map(|value| value.value().expose()),
            Some("database-password")
        );
        assert_eq!(grant.value().uid().map_or(0, |value| value.origins().len()), 1);
        assert_eq!(grant.origins().len(), 1);
        let debug = format!("{application:?}");
        for secret in ["private-config", "private-environment-name", "private-grant-source"] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));

        assert!(matches!(
            ResourceGrant::new(ProtectedString::plain(""), ResourceGrantSyntax::Short),
            Err(ModelError::EmptyValue("resource grant source"))
        ));
        assert!(matches!(
            application.add_config(Sourced::generated(Config::new(
                id("settings")?,
                ResourceOwnership::External,
            ))),
            Err(ModelError::DuplicateResource { kind: "config", .. })
        ));
        assert!(matches!(
            application.add_secret(Sourced::generated(Secret::new(
                id("password")?,
                ResourceOwnership::External,
            ))),
            Err(ModelError::DuplicateResource { kind: "secret", .. })
        ));
        Ok(())
    }

    #[test]
    fn host_mappings_preserve_order_spelling_and_runtime_tokens() -> Result<(), String> {
        let mut service = Service::new(id("web")?);
        service.add_host_mapping(Sourced::generated(HostMapping::new(
            id("host.docker.internal")?,
            HostAddress::new("host-gateway").map_err(|error| error.to_string())?,
        )));
        service.add_host_mapping(Sourced::generated(HostMapping::new(
            id("ipv6")?,
            HostAddress::new("[::1]").map_err(|error| error.to_string())?,
        )));

        assert_eq!(service.host_mappings().len(), 2);
        assert_eq!(
            service.host_mappings()[0].value().address().kind(),
            HostAddressKind::HostGateway
        );
        assert_eq!(service.host_mappings()[1].value().address().raw(), "[::1]");
        assert_eq!(
            service.host_mappings()[1].value().address().kind(),
            HostAddressKind::Ipv6 { bracketed: true }
        );
        assert!(matches!(HostAddress::new(""), Err(ModelError::EmptyValue(_))));
        Ok(())
    }

    #[test]
    fn dns_collections_preserve_order_provenance_and_explicit_empty_state() -> Result<(), String> {
        let mut service = Service::new(id("web")?);
        assert!(service.dns_servers().is_none());
        service.set_dns_servers(Vec::new());
        assert!(matches!(service.dns_servers(), Some(values) if values.is_empty()));
        service.set_dns_options(vec![
            Sourced::generated(ProtectedString::plain("ndots:5")),
            Sourced::generated(ProtectedString::sensitive("rotate")),
        ]);
        service.set_dns_search_domains(vec![Sourced::generated(ProtectedString::plain("example.test"))]);
        assert_eq!(
            service
                .dns_options()
                .unwrap_or_default()
                .iter()
                .map(|value| value.value().expose())
                .collect::<Vec<_>>(),
            ["ndots:5", "rotate"]
        );
        assert!(!format!("{service:?}").contains("rotate"));
        Ok(())
    }

    #[test]
    fn security_options_preserve_empty_order_duplicates_provenance_and_redaction() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?);
        let mut service = Service::new(id("web")?);

        assert!(service.security_options().is_none());
        service.set_security_options_with_origins(Vec::new(), vec![origin.clone()]);
        assert_eq!(service.security_options().map(<[_]>::len), Some(0));
        assert_eq!(service.security_options_origins(), std::slice::from_ref(&origin));

        service.set_security_options_with_origins(
            vec![
                Sourced::from_source(
                    SecurityOption::AppArmor(ProtectedString::sensitive("apparmor-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(SecurityOption::NoNewPrivileges(true), origin.clone()),
                Sourced::from_source(
                    SecurityOption::SeccompProfile(ProtectedString::sensitive("seccomp-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(SecurityOption::SecurityLabelDisable(false), origin.clone()),
                Sourced::from_source(
                    SecurityOption::SecurityLabelFileType(ProtectedString::sensitive("file-type-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(
                    SecurityOption::SecurityLabelLevel(ProtectedString::sensitive("level-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(SecurityOption::SecurityLabelNested(true), origin.clone()),
                Sourced::from_source(
                    SecurityOption::SecurityLabelType(ProtectedString::sensitive("type-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(
                    SecurityOption::Mask(ProtectedString::sensitive("mask-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(
                    SecurityOption::Unmask(ProtectedString::sensitive("unmask-secret")),
                    origin.clone(),
                ),
                Sourced::from_source(
                    SecurityOption::Mask(ProtectedString::sensitive("mask-secret")),
                    origin.clone(),
                ),
            ],
            vec![origin.clone()],
        );

        let options = service.security_options().unwrap_or_default();
        assert_eq!(options.len(), 11);
        assert!(
            matches!(options[0].value(), SecurityOption::AppArmor(profile) if profile.expose() == "apparmor-secret")
        );
        assert!(matches!(options[1].value(), SecurityOption::NoNewPrivileges(true)));
        assert!(
            matches!(options[2].value(), SecurityOption::SeccompProfile(profile) if profile.expose() == "seccomp-secret")
        );
        assert!(matches!(
            options[3].value(),
            SecurityOption::SecurityLabelDisable(false)
        ));
        assert!(
            matches!(options[4].value(), SecurityOption::SecurityLabelFileType(profile) if profile.expose() == "file-type-secret")
        );
        assert!(
            matches!(options[5].value(), SecurityOption::SecurityLabelLevel(profile) if profile.expose() == "level-secret")
        );
        assert!(matches!(options[6].value(), SecurityOption::SecurityLabelNested(true)));
        assert!(
            matches!(options[7].value(), SecurityOption::SecurityLabelType(profile) if profile.expose() == "type-secret")
        );
        assert!(matches!(options[8].value(), SecurityOption::Mask(path) if path.expose() == "mask-secret"));
        assert!(matches!(options[9].value(), SecurityOption::Unmask(path) if path.expose() == "unmask-secret"));
        assert!(matches!(options[10].value(), SecurityOption::Mask(path) if path.expose() == "mask-secret"));
        assert_eq!(options[0].origins(), std::slice::from_ref(&origin));
        assert_eq!(service.security_options_origins(), std::slice::from_ref(&origin));

        let debug = format!("{service:?}");
        for secret in [
            "apparmor-secret",
            "seccomp-secret",
            "file-type-secret",
            "level-secret",
            "type-secret",
            "mask-secret",
            "unmask-secret",
        ] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));

        service.set_security_options(Vec::new());
        assert_eq!(service.security_options().map(<[_]>::len), Some(0));
        assert!(service.security_options_origins().is_empty());
        Ok(())
    }

    #[test]
    fn retains_entrypoint_run_init_stop_pull_memory_and_exposed_port_intent() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?);
        let mut service = Service::new(id("web")?);
        service.set_command(Sourced::from_source(
            Command::Exec(vec![ProtectedString::plain("serve")]),
            origin.clone(),
        ));
        service.set_entrypoint(Sourced::from_source(
            Entrypoint::Shell(ProtectedString::sensitive("/bin/sh -c private-entrypoint")),
            origin.clone(),
        ));
        service.set_run_init(Sourced::from_source(true, origin.clone()));
        service.set_stop_timeout(Sourced::from_source(
            StopTimeout::new("01m30s").map_err(|error| error.to_string())?,
            origin.clone(),
        ));
        service.set_pull_policy(Sourced::from_source(
            PullPolicy::Every(ProtectedString::sensitive("12h")),
            origin.clone(),
        ));
        service.set_memory_limit(Sourced::from_source(
            ProtectedString::sensitive("512MiB"),
            origin.clone(),
        ));
        assert!(service.exposed_ports().is_none());
        service.set_exposed_ports_with_origins(Vec::new(), vec![origin.clone()]);
        assert_eq!(service.exposed_ports().map(<[_]>::len), Some(0));
        assert_eq!(service.exposed_ports_origins(), std::slice::from_ref(&origin));
        service.add_exposed_port(Sourced::from_source(
            ExposedPort::new(8080, Protocol::Tcp).map_err(|error| error.to_string())?,
            origin.clone(),
        ));
        service.add_exposed_port(Sourced::from_source(
            ExposedPort::new(8080, Protocol::Tcp).map_err(|error| error.to_string())?,
            origin,
        ));

        assert!(matches!(service.command().map(Sourced::value), Some(Command::Exec(_))));
        assert!(matches!(
            service.entrypoint().map(Sourced::value),
            Some(Entrypoint::Shell(_))
        ));
        assert_eq!(service.run_init().map(Sourced::value), Some(&true));
        assert_eq!(
            service.stop_timeout().map(|timeout| timeout.value().as_str()),
            Some("01m30s")
        );
        assert!(matches!(
            service.pull_policy().map(Sourced::value),
            Some(PullPolicy::Every(_))
        ));
        assert_eq!(
            service.memory_limit().map(|limit| limit.value().expose()),
            Some("512MiB")
        );
        let exposed_ports = service.exposed_ports().ok_or("missing exposed ports")?;
        assert_eq!(exposed_ports.len(), 2);
        assert_eq!(exposed_ports[0].value().container(), 8080);
        assert_eq!(exposed_ports[0].value().protocol(), &Protocol::Tcp);
        assert!(matches!(
            ExposedPort::new(0, Protocol::Udp),
            Err(ModelError::ZeroContainerPort)
        ));
        assert!(matches!(
            StopTimeout::new(""),
            Err(ModelError::EmptyValue("stop timeout"))
        ));

        let debug = format!("{service:?}");
        for secret in ["private-entrypoint", "512MiB", "12h"] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn annotations_and_logging_preserve_empty_order_field_provenance_and_redaction() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("quadlet.container").map_err(|error| error.to_string())?);
        let mut service = Service::new(id("web")?);

        assert!(service.annotations().is_none());
        service.set_annotations_with_origins(Vec::new(), vec![origin.clone()]);
        assert_eq!(service.annotations().map(<[_]>::len), Some(0));
        assert_eq!(service.annotations_origins(), std::slice::from_ref(&origin));

        service.set_annotations_with_origins(
            vec![
                Sourced::from_source(
                    Annotation::new(
                        Sourced::from_source(id("io.example.first")?, origin.clone()),
                        Sourced::from_source(ProtectedString::sensitive("annotation-secret"), origin.clone()),
                    ),
                    origin.clone(),
                ),
                Sourced::from_source(
                    Annotation::new(
                        Sourced::from_source(id("io.example.second")?, origin.clone()),
                        Sourced::from_source(ProtectedString::plain(""), origin.clone()),
                    ),
                    origin.clone(),
                ),
            ],
            vec![origin.clone()],
        );
        let annotations = service.annotations().unwrap_or_default();
        assert_eq!(annotations.len(), 2);
        assert_eq!(annotations[0].value().name().value().as_str(), "io.example.first");
        assert_eq!(annotations[1].value().value().value().expose(), "");
        assert_eq!(annotations[0].value().name().origins(), std::slice::from_ref(&origin));
        assert_eq!(annotations[0].value().value().origins(), std::slice::from_ref(&origin));

        let mut logging = Logging::new();
        assert!(logging.options().is_none());
        logging.set_driver(Sourced::from_source(ProtectedString::plain("journald"), origin.clone()));
        logging.set_options_with_origins(
            vec![
                Sourced::from_source(
                    LoggingOption::new(
                        Sourced::from_source(id("tag")?, origin.clone()),
                        Sourced::from_source(ProtectedString::sensitive("logging-secret"), origin.clone()),
                    ),
                    origin.clone(),
                ),
                Sourced::from_source(
                    LoggingOption::new(
                        Sourced::from_source(id("labels")?, origin.clone()),
                        Sourced::from_source(ProtectedString::plain(""), origin.clone()),
                    ),
                    origin.clone(),
                ),
            ],
            vec![origin.clone()],
        );
        service.set_logging(Sourced::from_source(logging, origin));

        let logging = service.logging().map(Sourced::value).ok_or("missing logging")?;
        assert_eq!(logging.driver().map(|driver| driver.value().expose()), Some("journald"));
        assert_eq!(logging.options().map(<[_]>::len), Some(2));
        assert_eq!(
            logging.options().unwrap_or_default()[0].value().name().value().as_str(),
            "tag"
        );
        assert_eq!(logging.options_origins().len(), 1);
        let debug = format!("{service:?}");
        assert!(!debug.contains("annotation-secret"));
        assert!(!debug.contains("logging-secret"));
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn network_attachments_retain_alias_provenance_and_redact_sensitive_values() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("compose.yaml").map_err(|error| error.to_string())?);
        let mut attachment = NetworkAttachment::new(
            id("frontend")?,
            vec![
                Sourced::from_source(ProtectedString::plain("web"), origin.clone()),
                Sourced::from_source(ProtectedString::sensitive("private-alias"), origin.clone()),
            ],
        );
        attachment.set_ipv4_address(Sourced::from_source(
            ProtectedString::plain("192.0.2.10"),
            origin.clone(),
        ));
        attachment.set_ipv6_address(Sourced::from_source(
            ProtectedString::plain("2001:db8::10"),
            origin.clone(),
        ));
        let metrics = Sourced::generated(ProtectedString::plain("metrics"));
        attachment.add_alias(&metrics);

        assert_eq!(attachment.aliases(), ["web", "private-alias", "metrics"]);
        assert_eq!(attachment.alias_sensitivities(), [false, true, false]);
        assert_eq!(attachment.alias_origins().len(), 3);
        assert_eq!(attachment.alias_origins()[0].len(), 1);
        assert_eq!(attachment.alias_origins()[1], std::slice::from_ref(&origin));
        assert!(attachment.alias_origins()[2].is_empty());
        assert_eq!(
            attachment.ipv4_address().map(|address| address.value().expose()),
            Some("192.0.2.10")
        );
        assert_eq!(
            attachment.ipv6_address().map(|address| address.value().expose()),
            Some("2001:db8::10")
        );
        let debug = format!("{attachment:?}");
        assert!(!debug.contains("private-alias"));
        assert!(debug.contains("[REDACTED]"));

        let mut service = Service::new(id("web")?);
        service.add_network(Sourced::generated(NetworkAttachment::new(
            id("previous")?,
            vec![Sourced::generated(ProtectedString::plain("previous-alias"))],
        )));
        let previous = service
            .replace_network(0, Sourced::generated(attachment))
            .map_err(|error| error.to_string())?;
        assert_eq!(previous.value().network().as_str(), "previous");
        assert_eq!(service.networks()[0].value().network().as_str(), "frontend");
        assert!(matches!(
            service.replace_network(1, Sourced::generated(NetworkAttachment::new(id("unused")?, Vec::new()))),
            Err(ModelError::UnknownNetworkAttachmentIndex { index: 1, len: 1 })
        ));
        Ok(())
    }

    #[test]
    fn reload_action_is_one_explicit_command_or_signal() -> Result<(), String> {
        let origin =
            crate::Provenance::source(crate::SourceId::new("quadlet.container").map_err(|error| error.to_string())?);
        let mut service = Service::new(id("web")?);
        service.set_reload_action(Sourced::from_source(
            ReloadAction::Command(Command::Exec(vec![ProtectedString::plain("reload")])),
            origin.clone(),
        ));
        assert!(matches!(
            service.reload_action().map(Sourced::value),
            Some(ReloadAction::Command(Command::Exec(_)))
        ));

        service.set_reload_action(Sourced::from_source(
            ReloadAction::Signal(ProtectedString::sensitive("SIGHUP")),
            origin,
        ));
        assert!(matches!(
            service.reload_action().map(Sourced::value),
            Some(ReloadAction::Signal(_))
        ));
        let debug = format!("{service:?}");
        assert!(!debug.contains("SIGHUP"));
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    fn id(value: &str) -> Result<Identifier, String> {
        Identifier::new(value).map_err(|error| error.to_string())
    }
}
