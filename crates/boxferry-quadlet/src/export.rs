//! Neutral-model-to-Quadlet planning.

use std::{collections::BTreeSet, error::Error, fmt};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, EnvironmentValue, Mount, MountSource, NetworkAttachment, Port, Protocol, Provenance,
    ResourceOwnership, SelinuxRelabel, Service, Sourced,
};
use quadlet_lens::{
    capability::{CapabilityCatalogue, CatalogueError, PodmanTarget, PodmanVersion, SupportClassification},
    model::{ContainerKey, NetworkKey, PodKey, QuadletUnitType, VolumeKey},
    path::{PathForm, classify_path},
    render::{EntryValue, QuadletDocumentBuilder, RenderError},
    source::SourceId,
};

use crate::QuadletOutput;

/// Caller-selected relationship between source services and generated Quadlet units.
///
/// Compose services normally have separate network namespaces, so separate `.container` units
/// are the lossless default. A shared Podman pod changes that topology and is therefore always
/// reported as an approximation, even when the adapter proves that declared ports and network
/// attachments do not conflict.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletGroupingPolicy {
    /// Preserve each service as an independently networked `.container` unit.
    #[default]
    SeparateContainers,
    /// Place every service in one application-owned `.pod` unit when declared topology permits.
    SinglePod,
}

/// Native target adapter for validated Podman Quadlet output.
#[derive(Clone, Debug)]
pub struct QuadletExporter {
    catalogue: CapabilityCatalogue,
    codes: Codes,
    relative_bind_root: Option<String>,
    grouping_policy: QuadletGroupingPolicy,
}

impl QuadletExporter {
    /// Creates an exporter backed by `QuadletLens`'s finite built-in capability catalogue.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletExporterError`] if embedded capability data or diagnostic codes are
    /// invalid.
    pub fn new() -> Result<Self, QuadletExporterError> {
        Ok(Self {
            catalogue: CapabilityCatalogue::supported_range()?,
            codes: Codes {
                invalid_target: DiagnosticCode::new("BFQ0001")?,
                assumed_future: DiagnosticCode::new("BFQ0002")?,
                unsupported: DiagnosticCode::new("BFQ0003")?,
                invalid_value: DiagnosticCode::new("BFQ0004")?,
                generation: DiagnosticCode::new("BFQ0005")?,
                capability: DiagnosticCode::new("BFQ0006")?,
                grouping: DiagnosticCode::new("BFQ0007")?,
            },
            relative_bind_root: None,
            grouping_policy: QuadletGroupingPolicy::SeparateContainers,
        })
    }

    /// Returns the exact capability catalogue used for target planning.
    #[must_use]
    pub const fn catalogue(&self) -> &CapabilityCatalogue {
        &self.catalogue
    }

    /// Resolves Compose-relative bind sources against an explicit absolute project root.
    ///
    /// Resolution is lexical and performs no filesystem access. The caller is responsible for
    /// supplying the source implementation's actual project directory.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletExporterError::InvalidRelativeBindRoot`] when the root is not an absolute,
    /// safely encodable POSIX path or lexically traverses above `/`.
    pub fn with_relative_bind_root(mut self, root: impl Into<String>) -> Result<Self, QuadletExporterError> {
        let root = root.into();
        let Some(root) = normalize_absolute_path(&root) else {
            return Err(QuadletExporterError::InvalidRelativeBindRoot(root));
        };
        self.relative_bind_root = Some(root);
        Ok(self)
    }

    /// Returns the normalized caller-selected Compose project root, when configured.
    #[must_use]
    pub fn relative_bind_root(&self) -> Option<&str> {
        self.relative_bind_root.as_deref()
    }

    /// Selects how services are grouped into native Quadlet units.
    #[must_use]
    pub const fn with_grouping_policy(mut self, policy: QuadletGroupingPolicy) -> Self {
        self.grouping_policy = policy;
        self
    }

    /// Returns the caller-selected service grouping policy.
    #[must_use]
    pub const fn grouping_policy(&self) -> QuadletGroupingPolicy {
        self.grouping_policy
    }
}

impl ExportAdapter for QuadletExporter {
    type Output = QuadletOutput;

    fn plan(
        &self,
        application: &Application,
        target: &TargetProfile,
    ) -> Result<ConversionPlan<Self::Output>, PlanError> {
        let mut mapping = Mapping::new(self, application, target);
        if mapping.validate_target() {
            mapping.map_application();
        }
        let (candidate, outcomes, diagnostics) = mapping.finish();
        ConversionPlan::new(candidate, outcomes, diagnostics)
    }
}

/// Failure while creating a Quadlet exporter.
#[derive(Debug)]
#[non_exhaustive]
pub enum QuadletExporterError {
    /// `QuadletLens`'s embedded capability catalogue failed validation.
    Catalogue(CatalogueError),
    /// A stable diagnostic code embedded in this adapter was malformed.
    DiagnosticCode(InvalidDiagnosticCode),
    /// A caller-selected relative bind root was not a safe absolute POSIX path.
    InvalidRelativeBindRoot(String),
}

impl fmt::Display for QuadletExporterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalogue(error) => write!(formatter, "cannot load Quadlet capability catalogue: {error}"),
            Self::DiagnosticCode(error) => write!(formatter, "invalid Quadlet adapter diagnostic code: {error}"),
            Self::InvalidRelativeBindRoot(root) => {
                write!(formatter, "invalid absolute relative-bind root `{root}`")
            }
        }
    }
}

impl Error for QuadletExporterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Catalogue(error) => Some(error),
            Self::DiagnosticCode(error) => Some(error),
            Self::InvalidRelativeBindRoot(_) => None,
        }
    }
}

impl From<CatalogueError> for QuadletExporterError {
    fn from(error: CatalogueError) -> Self {
        Self::Catalogue(error)
    }
}

impl From<InvalidDiagnosticCode> for QuadletExporterError {
    fn from(error: InvalidDiagnosticCode) -> Self {
        Self::DiagnosticCode(error)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_target: DiagnosticCode,
    assumed_future: DiagnosticCode,
    unsupported: DiagnosticCode,
    invalid_value: DiagnosticCode,
    generation: DiagnosticCode,
    capability: DiagnosticCode,
    grouping: DiagnosticCode,
}

struct Mapping<'a> {
    exporter: &'a QuadletExporter,
    application: &'a Application,
    target: &'a TargetProfile,
    podman_target: Option<PodmanTarget>,
    generated: Vec<(String, quadlet_lens::render::GeneratedQuadletDocument)>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
    next_source_id: u32,
    generation_failed: bool,
}

impl<'a> Mapping<'a> {
    const fn new(exporter: &'a QuadletExporter, application: &'a Application, target: &'a TargetProfile) -> Self {
        Self {
            exporter,
            application,
            target,
            podman_target: None,
            generated: Vec::new(),
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
            next_source_id: 1,
            generation_failed: false,
        }
    }

    fn validate_target(&mut self) -> bool {
        if self.target.implementation() != "podman" {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target",
                "Quadlet output requires the `podman` target implementation",
                "select target implementation `podman`",
                &[],
            );
            return false;
        }

        let requested = self.target.versions();
        let minimum = podman_version(requested.minimum());
        let maximum = requested.maximum().map(podman_version);
        let coverage = self.exporter.catalogue.coverage();
        let evaluated_maximum = maximum.unwrap_or(coverage.maximum());
        if minimum < coverage.minimum() || minimum > coverage.maximum() || evaluated_maximum > coverage.maximum() {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target.versions",
                "requested Podman range is outside the verified QuadletLens catalogue",
                &format!(
                    "requested {} through {}; verified coverage is {} through {}",
                    minimum,
                    maximum.map_or_else(|| "open ended".to_owned(), |version| version.to_string()),
                    coverage.minimum(),
                    coverage.maximum()
                ),
                &[],
            );
            return false;
        }
        let Ok(target) = PodmanTarget::new(minimum, maximum) else {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target.versions",
                "requested Podman range is invalid",
                "maximum version is before minimum version",
                &[],
            );
            return false;
        };
        if maximum.is_none() {
            self.diagnostics.push(
                Diagnostic::new(
                    self.exporter.codes.assumed_future.clone(),
                    Severity::Note,
                    "Podman maximum version is omitted; later compatibility remains an assumption",
                )
                .with_field(DiagnosticField::new(
                    "verified-through",
                    DiagnosticValue::plain(coverage.maximum().to_string()),
                )),
            );
        }
        self.podman_target = Some(target);
        true
    }

    fn map_application(&mut self) {
        let grouped = match self.exporter.grouping_policy {
            QuadletGroupingPolicy::SeparateContainers => false,
            QuadletGroupingPolicy::SinglePod => {
                if !self.validate_single_pod_grouping() {
                    self.generation_failed = true;
                    return;
                }
                true
            }
        };
        for network in self.application.networks() {
            self.map_network(network);
        }
        for volume in self.application.volumes() {
            self.map_volume(volume);
        }
        if grouped {
            self.map_pod();
        }
        for service in self.application.services() {
            self.map_service(service, grouped);
        }
    }

    fn validate_single_pod_grouping(&mut self) -> bool {
        let services = self.application.services();
        let origins = service_origins(services);
        let Some(first) = services.first() else {
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "single-pod grouping requires at least one service",
                "the neutral application contains no services",
                &origins,
            );
            return false;
        };

        for service in services {
            if !same_network_attachments(service.value().networks(), first.value().networks()) {
                self.invalid(
                    self.exporter.codes.grouping.clone(),
                    "application.grouping",
                    "single-pod grouping would change declared service networking",
                    "all services must have the same ordered network attachments and no aliases",
                    &origins,
                );
                return false;
            }
            if service
                .value()
                .networks()
                .iter()
                .any(|network| !network.value().aliases().is_empty())
            {
                self.invalid(
                    self.exporter.codes.grouping.clone(),
                    "application.grouping",
                    "single-pod grouping cannot preserve per-service network aliases",
                    "remove aliases or retain separate containers",
                    &origins,
                );
                return false;
            }
        }

        let mut container_ports = BTreeSet::new();
        let mut published_ports = BTreeSet::new();
        for service in services {
            let mut service_container_ports = BTreeSet::new();
            let mut service_published_ports = BTreeSet::new();
            for port in service.value().ports() {
                let Some(protocol) = protocol_name(port.value().protocol()) else {
                    self.invalid(
                        self.exporter.codes.grouping.clone(),
                        "application.grouping",
                        "single-pod grouping cannot validate an unknown port protocol",
                        "retain separate containers or use TCP, UDP, or SCTP",
                        &origins,
                    );
                    return false;
                };
                let container_port = (port.value().container(), protocol);
                if !service_container_ports.contains(&container_port) && container_ports.contains(&container_port) {
                    self.invalid(
                        self.exporter.codes.grouping.clone(),
                        "application.grouping",
                        "single-pod grouping has overlapping declared container ports",
                        "services in one pod share a network namespace; retain separate containers or remove the overlap",
                        &origins,
                    );
                    return false;
                }
                service_container_ports.insert(container_port);
                if let Some(published) = port.value().published() {
                    if !service_published_ports.contains(&(published, protocol))
                        && published_ports.contains(&(published, protocol))
                    {
                        self.invalid(
                            self.exporter.codes.grouping.clone(),
                            "application.grouping",
                            "single-pod grouping has overlapping published host ports",
                            "retain separate containers or assign distinct host ports",
                            &origins,
                        );
                        return false;
                    }
                    service_published_ports.insert((published, protocol));
                }
            }
            container_ports.extend(service_container_ports);
            published_ports.extend(service_published_ports);
        }
        true
    }

    fn map_network(&mut self, network: &Sourced<boxferry_model::Network>) {
        let subject = format!("networks.{}", network.value().name().as_str());
        match network.value().ownership() {
            ResourceOwnership::Application => {
                let Some(file_name) =
                    self.unit_file_name(&subject, network.value().name().as_str(), "network", network.origins())
                else {
                    return;
                };
                if !self.capability("quadlet.unit-type.network", &subject, network.origins())
                    || !self.capability("quadlet.network.name", &subject, network.origins())
                {
                    return;
                }
                let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Network);
                if self.push_network(
                    &mut builder,
                    NetworkKey::NetworkName,
                    network.value().name().as_str(),
                    &subject,
                    network.origins(),
                ) {
                    self.finish_document(file_name, &builder, &subject, network.origins());
                }
            }
            ResourceOwnership::External => self.exact(subject, network.origins()),
            ResourceOwnership::Implicit => self.unsupported(
                &subject,
                "implicit source network lifecycle cannot yet be reproduced safely",
                network.origins(),
            ),
            _ => self.unsupported(&subject, "unknown network ownership", network.origins()),
        }
    }

    fn map_volume(&mut self, volume: &Sourced<boxferry_model::Volume>) {
        let subject = format!("volumes.{}", volume.value().name().as_str());
        match volume.value().ownership() {
            ResourceOwnership::Application => {
                let Some(file_name) =
                    self.unit_file_name(&subject, volume.value().name().as_str(), "volume", volume.origins())
                else {
                    return;
                };
                if !self.capability("quadlet.unit-type.volume", &subject, volume.origins())
                    || !self.capability("quadlet.volume.name", &subject, volume.origins())
                {
                    return;
                }
                let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Volume);
                if self.push_volume(
                    &mut builder,
                    VolumeKey::VolumeName,
                    volume.value().name().as_str(),
                    &subject,
                    volume.origins(),
                ) {
                    self.finish_document(file_name, &builder, &subject, volume.origins());
                }
            }
            ResourceOwnership::External => self.exact(subject, volume.origins()),
            ResourceOwnership::Implicit => self.unsupported(
                &subject,
                "implicit source volume lifecycle cannot yet be reproduced safely",
                volume.origins(),
            ),
            _ => self.unsupported(&subject, "unknown volume ownership", volume.origins()),
        }
    }

    fn map_pod(&mut self) {
        let subject = "application.pod";
        let name = self.application.name().as_str();
        let origins = service_origins(self.application.services());
        let Some(file_name) = self.unit_file_name(subject, name, "pod", &origins) else {
            return;
        };
        if !self.capability("quadlet.unit-type.pod", subject, &origins)
            || !self.capability("quadlet.pod.name", subject, &origins)
        {
            return;
        }

        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Pod);
        if !self.push_pod(&mut builder, PodKey::PodName, name, subject, &origins) {
            return;
        }

        for service in self.application.services() {
            let service_subject = format!("services.{}", service.value().name().as_str());
            for (index, port) in service.value().ports().iter().enumerate() {
                self.map_pod_port(&service_subject, index, port, &mut builder);
            }
        }
        if let Some(service) = self.application.services().first() {
            for network in service.value().networks() {
                self.map_pod_network_attachment(network, &mut builder);
            }
        }

        self.approximate(
            "application.grouping",
            "caller-selected single-pod grouping shares one network namespace across source services",
            &origins,
        );
        self.finish_document(file_name, &builder, subject, &origins);
    }

    fn map_service(&mut self, service: &Sourced<Service>, grouped: bool) {
        let name = service.value().name().as_str();
        let subject = format!("services.{name}");
        let Some(file_name) = self.unit_file_name(&subject, name, "container", service.origins()) else {
            return;
        };
        if !self.capability("quadlet.unit-type.container", &subject, service.origins()) {
            return;
        }
        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Container);
        let Some(image) = service.value().image() else {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{subject}.image"),
                "Quadlet container generation requires an image",
                "neutral service has no image",
                service.origins(),
            );
            return;
        };
        if !is_safe_word(image.value().as_str(), false) {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{subject}.image"),
                "image reference cannot be emitted as an exact one-line Quadlet value",
                "image spelling contains unsupported whitespace or control syntax",
                image.origins(),
            );
            return;
        }
        if !self.capability("quadlet.container.image", &format!("{subject}.image"), image.origins())
            || !self.push_container(
                &mut builder,
                ContainerKey::Image,
                image.value().as_str(),
                &format!("{subject}.image"),
                image.origins(),
            )
        {
            return;
        }
        self.exact(format!("{subject}.image"), image.origins());

        if let Some(command) = service.value().command() {
            self.map_command(&subject, command, &mut builder);
        }
        for environment in service.value().environment() {
            self.map_environment(&subject, environment, &mut builder);
        }
        if !grouped {
            for (index, port) in service.value().ports().iter().enumerate() {
                self.map_port(&subject, index, port, &mut builder);
            }
        }
        for (index, mount) in service.value().mounts().iter().enumerate() {
            self.map_mount(&subject, index, mount, &mut builder);
        }
        if grouped {
            let pod_subject = format!("{subject}.pod");
            let pod_reference = format!("{}.pod", self.application.name().as_str());
            if self.capability("quadlet.container.pod", &pod_subject, service.origins())
                && self.push_container(
                    &mut builder,
                    ContainerKey::Pod,
                    pod_reference,
                    &pod_subject,
                    service.origins(),
                )
            {
                self.exact(pod_subject, service.origins());
            }
        } else {
            for network in service.value().networks() {
                self.map_network_attachment(&subject, network, &mut builder);
            }
        }

        self.finish_document(file_name, &builder, &subject, service.origins());
    }

    fn map_command(&mut self, service_subject: &str, command: &Sourced<Command>, builder: &mut QuadletDocumentBuilder) {
        let subject = format!("{service_subject}.command");
        let value = match command.value() {
            Command::Exec(arguments)
                if !arguments.is_empty() && arguments.iter().all(|argument| is_safe_word(argument.expose(), false)) =>
            {
                Some(
                    arguments
                        .iter()
                        .map(boxferry_model::ProtectedString::expose)
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            }
            Command::Exec(_) => {
                self.unsupported(
                    &subject,
                    "command arguments requiring systemd quoting are not yet encoded",
                    command.origins(),
                );
                None
            }
            Command::Shell(_) => {
                self.unsupported(
                    &subject,
                    "shell-form command semantics are not yet encoded for Quadlet Exec",
                    command.origins(),
                );
                None
            }
            Command::Empty => {
                self.unsupported(
                    &subject,
                    "explicit command clearing is not yet represented by the Quadlet adapter",
                    command.origins(),
                );
                None
            }
            _ => {
                self.unsupported(&subject, "unknown command form", command.origins());
                None
            }
        };
        if let Some(value) = value {
            if self.capability("quadlet.container.exec", &subject, command.origins())
                && self.push_container(builder, ContainerKey::Exec, value, &subject, command.origins())
            {
                self.exact(subject, command.origins());
            }
        }
    }

    fn map_environment(
        &mut self,
        service_subject: &str,
        environment: &Sourced<boxferry_model::EnvironmentVariable>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let name = environment.value().name().as_str();
        let subject = format!("{service_subject}.environment.{name}");
        if !is_environment_name(name) {
            self.unsupported(
                &subject,
                "environment name is outside the safely encoded Quadlet subset",
                environment.origins(),
            );
            return;
        }
        let EnvironmentValue::Literal(value) = environment.value().value() else {
            let reason = match environment.value().value() {
                EnvironmentValue::Host => "host environment resolution requires an explicit value provider",
                EnvironmentValue::Unset => "ensuring an image variable is absent requires a target-specific fallback",
                _ => "unknown environment value form",
            };
            self.unsupported(&subject, reason, environment.origins());
            return;
        };
        if !is_safe_word(value.expose(), true) {
            self.unsupported(
                &subject,
                "environment value requires systemd quoting or specifier escaping not yet encoded",
                environment.origins(),
            );
            return;
        }
        let encoded = format!("{name}={}", value.expose());
        if self.capability("quadlet.container.environment", &subject, environment.origins())
            && self.push_container(
                builder,
                ContainerKey::Environment,
                encoded,
                &subject,
                environment.origins(),
            )
        {
            self.exact(subject, environment.origins());
        }
    }

    fn map_port(
        &mut self,
        service_subject: &str,
        index: usize,
        port: &Sourced<Port>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.ports[{index}]");
        let Some(encoded) = self.encode_port(&subject, port) else {
            return;
        };
        if self.capability("quadlet.container.publish-port", &subject, port.origins())
            && self.push_container(builder, ContainerKey::PublishPort, encoded, &subject, port.origins())
        {
            self.exact(subject, port.origins());
        }
    }

    fn map_pod_port(
        &mut self,
        service_subject: &str,
        index: usize,
        port: &Sourced<Port>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.ports[{index}]");
        let Some(encoded) = self.encode_port(&subject, port) else {
            return;
        };
        if self.capability("quadlet.pod.publish-port", &subject, port.origins())
            && self.push_pod(builder, PodKey::PublishPort, encoded, &subject, port.origins())
        {
            self.exact(subject, port.origins());
        }
    }

    fn encode_port(&mut self, subject: &str, port: &Sourced<Port>) -> Option<String> {
        let Some(published) = port.value().published() else {
            self.unsupported(
                subject,
                "container-only exposure without host publication is not in the current Quadlet subset",
                port.origins(),
            );
            return None;
        };
        let Some(protocol) = protocol_name(port.value().protocol()) else {
            self.unsupported(subject, "unknown port protocol", port.origins());
            return None;
        };
        let prefix = match port.value().host_address() {
            None => String::new(),
            Some(address) if is_ipv4_spelling(address) => format!("{address}:"),
            Some(_) => {
                self.unsupported(
                    subject,
                    "host address requires a Quadlet port form not yet encoded",
                    port.origins(),
                );
                return None;
            }
        };
        Some(format!("{prefix}{published}:{}/{protocol}", port.value().container()))
    }

    fn map_mount(
        &mut self,
        service_subject: &str,
        index: usize,
        mount: &Sourced<Mount>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.mounts[{index}]");
        if !is_safe_mount_part(mount.value().target()) || !mount.value().target().starts_with('/') {
            self.unsupported(
                &subject,
                "container mount target is not a safely encoded absolute path",
                mount.origins(),
            );
            return;
        }
        let source = match mount.value().source() {
            MountSource::Volume(name) => self.volume_source(&subject, name.as_str(), mount.origins()),
            MountSource::HostPath(path) => match classify_path(path) {
                PathForm::AbsoluteLiteral | PathForm::SystemdSpecifier if is_safe_mount_part(path) => {
                    Some(path.clone())
                }
                PathForm::UnitRelativeLiteral => {
                    if let Some(root) = self.exporter.relative_bind_root.as_deref() {
                        if let Some(resolved) = resolve_relative_path(root, path) {
                            Some(resolved)
                        } else {
                            self.unsupported(
                                &subject,
                                "relative bind source traverses above the filesystem root",
                                mount.origins(),
                            );
                            None
                        }
                    } else {
                        self.unsupported(
                            &subject,
                            "relative bind source needs an explicit Compose project root",
                            mount.origins(),
                        );
                        None
                    }
                }
                PathForm::RelativeLiteral => {
                    self.unsupported(
                        &subject,
                        "relative bind form is not resolved by the configured POSIX path policy",
                        mount.origins(),
                    );
                    None
                }
                _ => {
                    self.unsupported(
                        &subject,
                        "bind source is outside the safely encoded Quadlet path subset",
                        mount.origins(),
                    );
                    None
                }
            },
            MountSource::Anonymous => Some(String::new()),
            _ => {
                self.unsupported(&subject, "unknown mount source", mount.origins());
                None
            }
        };
        let Some(source) = source else {
            return;
        };
        let mut encoded = if source.is_empty() {
            mount.value().target().to_owned()
        } else {
            format!("{source}:{}", mount.value().target())
        };
        let mut options = Vec::new();
        if mount.value().read_only() {
            options.push("ro");
        }
        match mount.value().selinux_relabel() {
            Some(SelinuxRelabel::Shared) => options.push("z"),
            Some(SelinuxRelabel::Private) => options.push("Z"),
            None => {}
            Some(_) => {
                self.unsupported(&subject, "unknown SELinux relabel mode", mount.origins());
                return;
            }
        }
        if !options.is_empty() {
            encoded.push(':');
            encoded.push_str(&options.join(","));
        }
        if self.capability("quadlet.container.volume", &subject, mount.origins())
            && self.push_container(builder, ContainerKey::Volume, encoded, &subject, mount.origins())
        {
            self.exact(subject, mount.origins());
        }
    }

    fn map_network_attachment(
        &mut self,
        service_subject: &str,
        network: &Sourced<NetworkAttachment>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let name = network.value().network().as_str();
        let subject = format!("{service_subject}.networks.{name}");
        let source = self.network_source(&subject, name, network.origins());
        if let Some(source) = source {
            if self.capability("quadlet.container.network", &subject, network.origins())
                && self.push_container(builder, ContainerKey::Network, source, &subject, network.origins())
            {
                self.exact(&subject, network.origins());
            }
        }
        if !network.value().aliases().is_empty() {
            self.unsupported(
                &format!("{subject}.aliases"),
                "per-network aliases are not in QuadletLens's Podman 5.4 generation subset",
                network.origins(),
            );
        }
    }

    fn map_pod_network_attachment(
        &mut self,
        network: &Sourced<NetworkAttachment>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let name = network.value().network().as_str();
        let subject = format!("application.pod.networks.{name}");
        let source = self.network_source(&subject, name, network.origins());
        if let Some(source) = source {
            if self.capability("quadlet.pod.network", &subject, network.origins())
                && self.push_pod(builder, PodKey::Network, source, &subject, network.origins())
            {
                self.exact(subject, network.origins());
            }
        }
    }

    fn network_source(&mut self, subject: &str, name: &str, origins: &[Provenance]) -> Option<String> {
        if !is_native_atom(name) {
            self.unsupported(subject, "network name requires target-specific escaping", origins);
            return None;
        }
        let declaration = self
            .application
            .networks()
            .iter()
            .find(|network| network.value().name().as_str() == name);
        match declaration.map(|network| network.value().ownership()) {
            Some(ResourceOwnership::Application) => Some(format!("{name}.network")),
            Some(ResourceOwnership::External) => Some(name.to_owned()),
            Some(ResourceOwnership::Implicit) | None => {
                self.unsupported(subject, "implicit network cannot yet be reproduced safely", origins);
                None
            }
            Some(_) => {
                self.unsupported(subject, "unknown network ownership", origins);
                None
            }
        }
    }

    fn volume_source(&mut self, subject: &str, name: &str, origins: &[Provenance]) -> Option<String> {
        if !is_native_atom(name) {
            self.unsupported(subject, "volume name requires target-specific escaping", origins);
            return None;
        }
        let declaration = self
            .application
            .volumes()
            .iter()
            .find(|volume| volume.value().name().as_str() == name);
        match declaration.map(|volume| volume.value().ownership()) {
            Some(ResourceOwnership::Application) => Some(format!("{name}.volume")),
            Some(ResourceOwnership::External) => Some(name.to_owned()),
            Some(ResourceOwnership::Implicit) | None => {
                self.unsupported(subject, "implicit volume cannot yet be reproduced safely", origins);
                None
            }
            Some(_) => {
                self.unsupported(subject, "unknown volume ownership", origins);
                None
            }
        }
    }

    fn unit_file_name(&mut self, subject: &str, stem: &str, extension: &str, origins: &[Provenance]) -> Option<String> {
        if is_native_atom(stem) {
            Some(format!("{stem}.{extension}"))
        } else {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                subject,
                "neutral identifier cannot be used safely as a Quadlet unit filename",
                "use an ASCII alphanumeric, dot, underscore, or hyphen name beginning and ending with an alphanumeric character",
                origins,
            );
            None
        }
    }

    fn capability(&mut self, capability: &str, subject: &str, origins: &[Provenance]) -> bool {
        let Some(target) = self.podman_target else {
            return false;
        };
        let evaluation = self.exporter.catalogue.evaluate(capability, target);
        match evaluation.classification() {
            SupportClassification::Native => true,
            SupportClassification::Deprecated => {
                self.diagnostics.push(
                    Diagnostic::new(
                        self.exporter.codes.capability.clone(),
                        Severity::Note,
                        "required Quadlet capability is deprecated for part of the target range",
                    )
                    .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                    .with_field(DiagnosticField::new("capability", DiagnosticValue::plain(capability))),
                );
                true
            }
            classification => {
                self.loss(
                    self.exporter.codes.capability.clone(),
                    subject,
                    ConversionKind::Unsupported,
                    "required Quadlet capability does not cover the complete target range",
                    &format!("{capability}: {classification:?}"),
                    origins,
                );
                false
            }
        }
    }

    fn push_container(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: ContainerKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_container(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn push_pod(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: PodKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_pod(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn push_network(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: NetworkKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_network(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn push_volume(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: VolumeKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_volume(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn finish_document(
        &mut self,
        file_name: String,
        builder: &QuadletDocumentBuilder,
        subject: &str,
        origins: &[Provenance],
    ) {
        let source_id = SourceId::new(self.next_source_id);
        let Some(next) = self.next_source_id.checked_add(1) else {
            self.invalid(
                self.exporter.codes.generation.clone(),
                subject,
                "generated Quadlet document count exceeds the supported source identity space",
                "reduce the number of generated unit files",
                origins,
            );
            self.generation_failed = true;
            return;
        };
        self.next_source_id = next;
        match builder.build(source_id) {
            Ok(document) => {
                self.generated.push((file_name, document));
                self.exact(subject, origins);
            }
            Err(error) => self.generation_error(subject, &error, origins),
        }
    }

    fn generation_error(&mut self, subject: &str, error: &RenderError, origins: &[Provenance]) {
        self.generation_failed = true;
        self.invalid(
            self.exporter.codes.generation.clone(),
            subject,
            "QuadletLens rejected generated native output",
            &error.to_string(),
            origins,
        );
    }

    fn exact(&mut self, subject: impl Into<String>, origins: &[Provenance]) {
        let mut outcome = ConversionOutcome::exact(subject);
        for origin in origins {
            outcome = outcome.with_origin(origin.clone());
        }
        self.outcomes.push(outcome);
    }

    fn unsupported(&mut self, subject: &str, reason: &str, origins: &[Provenance]) {
        self.loss(
            self.exporter.codes.unsupported.clone(),
            subject,
            ConversionKind::Unsupported,
            "neutral application intent is not represented by the current Quadlet subset",
            reason,
            origins,
        );
    }

    fn approximate(&mut self, subject: &str, reason: &str, origins: &[Provenance]) {
        self.loss(
            self.exporter.codes.grouping.clone(),
            subject,
            ConversionKind::Approximate,
            "generated Quadlet topology intentionally approximates source service isolation",
            reason,
            origins,
        );
    }

    fn invalid(&mut self, code: DiagnosticCode, subject: &str, summary: &str, reason: &str, origins: &[Provenance]) {
        self.loss(code, subject, ConversionKind::Invalid, summary, reason, origins);
    }

    fn loss(
        &mut self,
        code: DiagnosticCode,
        subject: &str,
        kind: ConversionKind,
        summary: &str,
        reason: &str,
        origins: &[Provenance],
    ) {
        let severity = if kind == ConversionKind::Invalid {
            Severity::Error
        } else {
            Severity::Warning
        };
        self.diagnostics.push(
            Diagnostic::new(code.clone(), severity, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason))),
        );
        if let Ok(mut outcome) = ConversionOutcome::loss(subject, kind, code) {
            for origin in origins {
                outcome = outcome.with_origin(origin.clone());
            }
            self.outcomes.push(outcome);
        }
    }

    fn finish(mut self) -> (Option<QuadletOutput>, Vec<ConversionOutcome>, Vec<Diagnostic>) {
        let candidate = if self.podman_target.is_some() && !self.generation_failed {
            let generated = std::mem::take(&mut self.generated);
            match QuadletOutput::from_generated(generated) {
                Ok(output) if output.document_set().is_valid() => Some(output),
                Ok(_) => {
                    self.invalid(
                        self.exporter.codes.generation.clone(),
                        "application",
                        "generated Quadlet document set has unresolved or ambiguous references",
                        "inspect native document-set diagnostics",
                        &[],
                    );
                    None
                }
                Err(error) => {
                    self.invalid(
                        self.exporter.codes.generation.clone(),
                        "application",
                        "generated Quadlet document metadata is invalid",
                        &error.to_string(),
                        &[],
                    );
                    None
                }
            }
        } else {
            None
        };
        (candidate, self.outcomes, self.diagnostics)
    }
}

fn podman_version(version: boxferry_engine::PlatformVersion) -> PodmanVersion {
    PodmanVersion::new(version.major(), version.minor(), version.patch())
}

fn protocol_name(protocol: &Protocol) -> Option<&'static str> {
    match protocol {
        Protocol::Tcp => Some("tcp"),
        Protocol::Udp => Some("udp"),
        Protocol::Sctp => Some("sctp"),
        _ => None,
    }
}

fn same_network_attachments(left: &[Sourced<NetworkAttachment>], right: &[Sourced<NetworkAttachment>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.value() == right.value())
}

fn service_origins(services: &[Sourced<Service>]) -> Vec<Provenance> {
    services
        .iter()
        .flat_map(|service| service.origins().iter().cloned())
        .collect()
}

fn is_native_atom(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_environment_name(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_safe_word(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',')
        })
}

fn is_safe_mount_part(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@' | b'+' | b'=' | b'%')
        })
}

fn is_ipv4_spelling(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.')
}

fn normalize_absolute_path(value: &str) -> Option<String> {
    if classify_path(value) != PathForm::AbsoluteLiteral || !is_safe_mount_part(value) {
        return None;
    }
    let mut components = Vec::new();
    for component in value.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component.to_owned()),
        }
    }
    Some(format!("/{}", components.join("/")))
}

fn resolve_relative_path(root: &str, relative: &str) -> Option<String> {
    let mut components = root
        .trim_start_matches('/')
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for component in relative.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component if component != "~" => components.push(component.to_owned()),
            _ => return None,
        }
    }
    Some(format!("/{}", components.join("/")))
}
