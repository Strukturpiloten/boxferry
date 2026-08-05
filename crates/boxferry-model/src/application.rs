//! Ordered neutral application graph and resource attachments.

use std::{error::Error, fmt, net::IpAddr};

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
    /// An image reference had invalid component structure.
    InvalidImageReference(&'static str),
    /// A container port was zero.
    ZeroContainerPort,
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
            Self::InvalidImageReference(reason) => write!(formatter, "invalid image reference: {reason}"),
            Self::ZeroContainerPort => formatter.write_str("container port must not be zero"),
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

/// One structural group of application services.
///
/// Membership alone does not imply shared Linux namespaces, an infra container, or a target
/// workload kind. Source and target adapters must model or report those semantics separately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceGroup {
    name: Identifier,
    ownership: ResourceOwnership,
    members: Vec<Sourced<Identifier>>,
}

impl ServiceGroup {
    /// Creates an empty structural service group.
    #[must_use]
    pub const fn new(name: Identifier, ownership: ResourceOwnership) -> Self {
        Self {
            name,
            ownership,
            members: Vec::new(),
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
    image: Option<Sourced<ImageReference>>,
    command: Option<Sourced<Command>>,
    restart_policy: Option<Sourced<RestartPolicy>>,
    healthcheck: Option<Sourced<Healthcheck>>,
    labels: Vec<Sourced<MetadataLabel>>,
    user: Option<Sourced<ProtectedString>>,
    group: Option<Sourced<ProtectedString>>,
    user_namespace: Option<Sourced<ProtectedString>>,
    supplementary_groups: Vec<Sourced<ProtectedString>>,
    working_directory: Option<Sourced<ProtectedString>>,
    read_only_root_filesystem: Option<Sourced<bool>>,
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
            image: None,
            command: None,
            restart_policy: None,
            healthcheck: None,
            labels: Vec::new(),
            user: None,
            group: None,
            user_namespace: None,
            supplementary_groups: Vec::new(),
            working_directory: None,
            read_only_root_filesystem: None,
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
}

/// One ordered multi-service application graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    name: Identifier,
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
    use super::{
        Application, Config, ConfigMaterial, EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax,
        HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier, MetadataLabel,
        ModelError, ResourceGrant, ResourceGrantSyntax, ResourceOwnership, RestartPolicy, Secret, SecretMaterial,
        Service, ServiceDependency, ServiceDependencyCondition, ServiceGroup,
    };
    use crate::{ProtectedString, Sourced};

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

    fn id(value: &str) -> Result<Identifier, String> {
        Identifier::new(value).map_err(|error| error.to_string())
    }
}
