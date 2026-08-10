//! Neutral-model-to-Quadlet planning.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::{self, Write as _},
};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, Config, Device, EnvironmentFile, EnvironmentFileFormat, EnvironmentValue, Healthcheck,
    HealthcheckCommand, HostAddressKind, HostMapping, Mount, MountSource, NetworkAttachment, Port, Protocol,
    Provenance, ResourceGrant, ResourceOwnership, RestartPolicy, Secret, SecurityOption, SelinuxRelabel, Service,
    ServiceDependency, ServiceDependencyCondition, Sourced,
};
use quadlet_lens::{
    capability::{CapabilityCatalogue, CatalogueError, PodmanTarget, PodmanVersion, SupportClassification},
    model::{ContainerKey, NetworkKey, PodKey, QuadletUnitType, VolumeKey},
    path::{PathForm, classify_path},
    render::{EntryValue, PidsLimit, QuadletDocumentBuilder, RenderError, ShmSize, SystemdSection, SystemdUnitKey},
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
    /// Preserve one application-owned neutral service group as its correspondingly named `.pod`.
    ///
    /// The group must contain every application service exactly once. This policy does not infer
    /// how multiple or partial groups should map to Quadlet units.
    PreserveSingleGroup,
}

/// Native target adapter for validated Podman Quadlet output.
#[derive(Clone, Debug)]
pub struct QuadletExporter {
    catalogue: CapabilityCatalogue,
    codes: Codes,
    relative_bind_root: Option<String>,
    bind_source_mappings: BTreeMap<String, String>,
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
                dependency: DiagnosticCode::new("BFQ0008")?,
                restart: DiagnosticCode::new("BFQ0009")?,
                environment_file: DiagnosticCode::new("BFQ0010")?,
            },
            relative_bind_root: None,
            bind_source_mappings: BTreeMap::new(),
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

    /// Resolves Compose-relative host paths against an explicit absolute project root.
    ///
    /// This is the general form of [`Self::with_relative_bind_root`]. It applies to bind sources
    /// and environment-file declarations. Resolution is lexical and performs no filesystem access.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletExporterError::InvalidRelativeBindRoot`] when the root is not an absolute,
    /// safely encodable POSIX path or lexically traverses above `/`.
    pub fn with_relative_host_path_root(self, root: impl Into<String>) -> Result<Self, QuadletExporterError> {
        self.with_relative_bind_root(root)
    }

    /// Returns the normalized caller-selected Compose project root, when configured.
    #[must_use]
    pub fn relative_bind_root(&self) -> Option<&str> {
        self.relative_bind_root.as_deref()
    }

    /// Returns the normalized caller-selected Compose project root, when configured.
    #[must_use]
    pub fn relative_host_path_root(&self) -> Option<&str> {
        self.relative_bind_root()
    }

    /// Adds one explicit authored bind-source to target-path mapping.
    ///
    /// This is the opt-in boundary for source-machine-specific forms such as `~/data`, Windows
    /// paths, or environment-derived spellings. The source is matched exactly. The target must be
    /// either an absolute safely encodable POSIX path or a systemd-specifier path such as
    /// `%h/data`. Explicit mappings take precedence over relative-project-root resolution.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletExporterError::InvalidBindSourceMapping`] for an empty/control-bearing
    /// source or unsafe target. Returns [`QuadletExporterError::ConflictingBindSourceMapping`] if
    /// the same source was already assigned a different target.
    pub fn with_bind_source_mapping(
        mut self,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Result<Self, QuadletExporterError> {
        let source = source.into();
        let target = target.into();
        if !is_valid_mapping_source(&source) || !is_valid_quadlet_bind_source(&target) {
            return Err(QuadletExporterError::InvalidBindSourceMapping { source, target });
        }
        if let Some(existing) = self.bind_source_mappings.get(&source) {
            if existing != &target {
                return Err(QuadletExporterError::ConflictingBindSourceMapping {
                    source,
                    existing: existing.clone(),
                    replacement: target,
                });
            }
            return Ok(self);
        }
        self.bind_source_mappings.insert(source, target);
        Ok(self)
    }

    /// Returns the exact target selected for an authored bind-source spelling.
    #[must_use]
    pub fn bind_source_mapping(&self, source: &str) -> Option<&str> {
        self.bind_source_mappings.get(source).map(String::as_str)
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
    /// An explicit bind-source mapping contained an invalid source or target spelling.
    InvalidBindSourceMapping {
        /// Exact source spelling supplied by the caller.
        source: String,
        /// Requested Quadlet target spelling.
        target: String,
    },
    /// One source spelling was assigned two different target paths.
    ConflictingBindSourceMapping {
        /// Exact source spelling supplied by the caller.
        source: String,
        /// Previously configured target spelling.
        existing: String,
        /// Conflicting replacement target spelling.
        replacement: String,
    },
}

impl fmt::Display for QuadletExporterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalogue(error) => write!(formatter, "cannot load Quadlet capability catalogue: {error}"),
            Self::DiagnosticCode(error) => write!(formatter, "invalid Quadlet adapter diagnostic code: {error}"),
            Self::InvalidRelativeBindRoot(root) => {
                write!(formatter, "invalid absolute relative-bind root `{root}`")
            }
            Self::InvalidBindSourceMapping { source, target } => {
                write!(
                    formatter,
                    "invalid explicit bind-source mapping `{source}` to `{target}`"
                )
            }
            Self::ConflictingBindSourceMapping {
                source,
                existing,
                replacement,
            } => write!(
                formatter,
                "conflicting bind-source mapping for `{source}`: `{existing}` versus `{replacement}`"
            ),
        }
    }
}

impl Error for QuadletExporterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Catalogue(error) => Some(error),
            Self::DiagnosticCode(error) => Some(error),
            Self::InvalidRelativeBindRoot(_)
            | Self::InvalidBindSourceMapping { .. }
            | Self::ConflictingBindSourceMapping { .. } => None,
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
    dependency: DiagnosticCode,
    restart: DiagnosticCode,
    environment_file: DiagnosticCode,
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

struct PodPlan {
    name: String,
    subject: String,
    origins: Vec<Provenance>,
    consumed_group: Option<String>,
    approximation: &'static str,
}

#[derive(Clone, Copy)]
enum HealthcheckScalarKind {
    Duration,
    RetryCount,
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
        if !self.validate_dependency_graph() {
            self.generation_failed = true;
            return;
        }
        let pod_plan = match self.exporter.grouping_policy {
            QuadletGroupingPolicy::SeparateContainers => None,
            QuadletGroupingPolicy::SinglePod => {
                if !self.validate_single_pod_grouping() {
                    self.generation_failed = true;
                    return;
                }
                Some(PodPlan {
                    name: self.application.name().as_str().to_owned(),
                    subject: "application.pod".to_owned(),
                    origins: service_origins(self.application.services()),
                    consumed_group: None,
                    approximation: "caller-selected single-pod grouping shares one network namespace across source services",
                })
            }
            QuadletGroupingPolicy::PreserveSingleGroup => {
                let Some(plan) = self.preserve_single_group_plan() else {
                    self.generation_failed = true;
                    return;
                };
                Some(plan)
            }
        };
        for group in self.application.service_groups() {
            if pod_plan.as_ref().and_then(|plan| plan.consumed_group.as_deref()) != Some(group.value().name().as_str())
            {
                self.map_service_group(group);
            }
        }
        for network in self.application.networks() {
            self.map_network(network);
        }
        for volume in self.application.volumes() {
            self.map_volume(volume);
        }
        for config in self.application.configs() {
            self.map_config(config);
        }
        for secret in self.application.secrets() {
            self.map_secret(secret);
        }
        if let Some(plan) = &pod_plan {
            self.map_pod(plan);
        }
        for service in self.application.services() {
            self.map_service(service, pod_plan.as_ref().map(|plan| plan.name.as_str()));
        }
    }

    fn preserve_single_group_plan(&mut self) -> Option<PodPlan> {
        let groups = self.application.service_groups();
        if groups.len() != 1 {
            let origins = groups
                .iter()
                .flat_map(|group| group.origins().iter().cloned())
                .collect::<Vec<_>>();
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "preserving a neutral service group requires exactly one group",
                if groups.is_empty() {
                    "the application contains no service group"
                } else {
                    "the application contains multiple service groups"
                },
                &origins,
            );
            return None;
        }

        let group = &groups[0];
        if group.value().ownership() != ResourceOwnership::Application {
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "only an application-owned service group can be generated as a Quadlet pod",
                "resolve the group lifecycle as application-owned before selecting preserve-single-group",
                group.origins(),
            );
            return None;
        }

        let service_names = self
            .application
            .services()
            .iter()
            .map(|service| service.value().name().as_str())
            .collect::<BTreeSet<_>>();
        let member_names = group
            .value()
            .members()
            .iter()
            .map(|member| member.value().as_str())
            .collect::<BTreeSet<_>>();
        if service_names != member_names || group.value().members().len() != self.application.services().len() {
            let origins = service_group_origins(group, self.application.services());
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "preserved Quadlet pod membership must cover the complete application",
                "the single service group does not contain every application service exactly once",
                &origins,
            );
            return None;
        }
        if !self.validate_single_pod_grouping() {
            return None;
        }

        Some(PodPlan {
            name: group.value().name().as_str().to_owned(),
            subject: format!("service_groups.{}", group.value().name().as_str()),
            origins: service_group_origins(group, self.application.services()),
            consumed_group: Some(group.value().name().as_str().to_owned()),
            approximation: "caller-selected preservation maps structural membership to one shared Podman pod namespace",
        })
    }

    fn validate_dependency_graph(&mut self) -> bool {
        let service_names = self
            .application
            .services()
            .iter()
            .map(|service| service.value().name().as_str().to_owned())
            .collect::<BTreeSet<_>>();
        let mut valid = true;
        let mut indegree = service_names
            .iter()
            .map(|name| (name.clone(), 0_usize))
            .collect::<BTreeMap<_, _>>();
        let mut outgoing = BTreeMap::<String, Vec<String>>::new();

        for service in self.application.services() {
            let dependent = service.value().name().as_str();
            for (index, dependency) in service.value().dependencies().iter().enumerate() {
                let target = dependency.value().service().as_str();
                if !service_names.contains(target) {
                    if dependency.value().is_required() {
                        self.invalid(
                            self.exporter.codes.dependency.clone(),
                            &format!("services.{dependent}.dependencies[{index}]"),
                            "required service dependency does not exist in the application",
                            &format!("referenced service `{target}` is missing"),
                            dependency.origins(),
                        );
                        valid = false;
                    }
                    continue;
                }
                if let Some(count) = indegree.get_mut(dependent) {
                    *count = count.saturating_add(1);
                }
                outgoing
                    .entry(target.to_owned())
                    .or_default()
                    .push(dependent.to_owned());
            }
        }
        if !valid {
            return false;
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(name, count)| (*count == 0).then_some(name.clone()))
            .collect::<Vec<_>>();
        let mut visited = 0_usize;
        while let Some(name) = ready.pop() {
            visited = visited.saturating_add(1);
            if let Some(dependents) = outgoing.get(&name) {
                for dependent in dependents {
                    if let Some(count) = indegree.get_mut(dependent) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            ready.push(dependent.clone());
                        }
                    }
                }
            }
        }
        if visited == service_names.len() {
            return true;
        }

        let cycle_services = indegree
            .iter()
            .filter_map(|(name, count)| (*count > 0).then_some(name.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let origins = self
            .application
            .services()
            .iter()
            .flat_map(|service| service.value().dependencies())
            .flat_map(Sourced::origins)
            .cloned()
            .collect::<Vec<_>>();
        self.invalid(
            self.exporter.codes.dependency.clone(),
            "application.dependencies",
            "service dependency graph contains an ordering cycle",
            &format!("cycle includes: {cycle_services}"),
            &origins,
        );
        false
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
            if !same_host_mappings(service.value().host_mappings(), first.value().host_mappings()) {
                self.invalid(
                    self.exporter.codes.grouping.clone(),
                    "application.grouping",
                    "single-pod grouping cannot preserve different per-service host mappings",
                    "all services must have the same ordered host mappings or retain separate containers",
                    &origins,
                );
                return false;
            }
        }

        if !self.validate_grouped_user_namespace(services, &origins) {
            return false;
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

    fn validate_grouped_user_namespace(&mut self, services: &[Sourced<Service>], origins: &[Provenance]) -> bool {
        let first_user_namespace = services[0].value().user_namespace().map(|value| value.value().expose());
        if services
            .iter()
            .any(|service| service.value().user_namespace().map(|value| value.value().expose()) != first_user_namespace)
        {
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "single-pod grouping cannot preserve different per-service user namespaces",
                "all services must declare the same user namespace, or every service must leave it implicit",
                origins,
            );
            return false;
        }
        if first_user_namespace.is_some_and(|value| !is_safe_word(value, false)) {
            self.invalid(
                self.exporter.codes.grouping.clone(),
                "application.grouping",
                "single-pod grouping cannot encode the shared user namespace safely",
                "the shared user namespace contains unsupported whitespace or control syntax",
                origins,
            );
            return false;
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

    fn map_config(&mut self, config: &Sourced<Config>) {
        let subject = format!("configs.{}", config.value().name().as_str());
        self.unsupported(
            &subject,
            "Quadlet has no native configuration-resource lifecycle; create and mount this config through an explicit target policy",
            config.origins(),
        );
    }

    fn map_service_group(&mut self, group: &Sourced<boxferry_model::ServiceGroup>) {
        let subject = format!("service_groups.{}", group.value().name().as_str());
        self.unsupported(
            &subject,
            "neutral service-group membership requires an explicit Quadlet grouping and lifecycle resolution policy",
            group.origins(),
        );
    }

    fn map_secret(&mut self, secret: &Sourced<Secret>) {
        let subject = format!("secrets.{}", secret.value().name().as_str());
        match secret.value().ownership() {
            ResourceOwnership::External if secret.value().material().is_none() => self.exact(subject, secret.origins()),
            ResourceOwnership::External => self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &subject,
                "external secret cannot also request application-managed materialization",
                "remove the material source or make the secret application-owned",
                secret.origins(),
            ),
            ResourceOwnership::Application => self.unsupported(
                &subject,
                "Quadlet container units can consume Podman secrets but cannot create or update their material",
                secret.origins(),
            ),
            ResourceOwnership::Implicit => self.unsupported(
                &subject,
                "implicit source secret lifecycle cannot be reproduced safely",
                secret.origins(),
            ),
            _ => self.unsupported(&subject, "unknown secret ownership", secret.origins()),
        }
    }

    fn map_pod(&mut self, plan: &PodPlan) {
        let subject = plan.subject.as_str();
        let name = plan.name.as_str();
        let origins = &plan.origins;
        let Some(file_name) = self.unit_file_name(subject, name, "pod", origins) else {
            return;
        };
        if !self.capability("quadlet.unit-type.pod", subject, origins)
            || !self.capability("quadlet.pod.name", subject, origins)
        {
            return;
        }

        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Pod);
        if !self.push_pod(&mut builder, PodKey::PodName, name, subject, origins) {
            return;
        }

        if let Some(user_namespace) = self
            .application
            .services()
            .first()
            .and_then(|service| service.value().user_namespace())
        {
            let namespace_subject = "application.pod.user_namespace";
            let namespace_origins = shared_user_namespace_origins(self.application.services());
            if self.capability("quadlet.pod.userns", namespace_subject, &namespace_origins)
                && self.push_pod(
                    &mut builder,
                    PodKey::UserNS,
                    user_namespace.value().expose(),
                    namespace_subject,
                    &namespace_origins,
                )
            {
                self.exact(namespace_subject, &namespace_origins);
            }
        }

        if let Some(service) = self.application.services().first() {
            for (index, mapping) in service.value().host_mappings().iter().enumerate() {
                let mapping_origins = shared_host_mapping_origins(self.application.services(), index);
                self.map_pod_host_mapping(index, mapping.value(), &mapping_origins, &mut builder);
            }
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

        self.approximate("application.grouping", plan.approximation, origins);
        self.finish_document(file_name, &builder, subject, origins);
    }

    fn map_service(&mut self, service: &Sourced<Service>, pod_name: Option<&str>) {
        let grouped = pod_name.is_some();
        let name = service.value().name().as_str();
        let subject = format!("services.{name}");
        let Some(file_name) = self.unit_file_name(&subject, name, "container", service.origins()) else {
            return;
        };
        if !self.capability("quadlet.unit-type.container", &subject, service.origins()) {
            return;
        }
        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Container);
        self.map_service_dependencies(service, &mut builder);
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

        self.map_container_name(&subject, service.value(), &mut builder);
        if let Some(command) = service.value().command() {
            self.map_command(&subject, command, &mut builder);
        }
        self.map_execution_context(&subject, service.value(), grouped, &mut builder);
        self.map_released_container_settings(&subject, service.value(), &mut builder);
        self.report_grouped_dns(&subject, service.value(), grouped);
        if let Some(restart_policy) = service.value().restart_policy() {
            self.map_restart_policy(&subject, restart_policy, &mut builder);
        }
        if let Some(healthcheck) = service.value().healthcheck() {
            self.map_healthcheck(&subject, healthcheck, &mut builder);
        }
        self.map_healthy_readiness(service, &mut builder);
        for environment in service.value().environment() {
            self.map_environment(&subject, environment, &mut builder);
        }
        for (index, environment_file) in service.value().environment_files().iter().enumerate() {
            self.map_environment_file(&subject, index, environment_file, &mut builder);
        }
        self.map_metadata_labels(&subject, service.value(), &mut builder);
        for (index, grant) in service.value().config_grants().iter().enumerate() {
            self.unsupported(
                &format!("{subject}.configs[{index}]"),
                "Quadlet has no native Compose-compatible config grant; define an explicit bind or target-specific materialization policy",
                grant.origins(),
            );
        }
        for (index, grant) in service.value().secret_grants().iter().enumerate() {
            self.map_secret_grant(&subject, index, grant, &mut builder);
        }
        if !grouped {
            for (index, mapping) in service.value().host_mappings().iter().enumerate() {
                self.map_container_host_mapping(&subject, index, mapping, &mut builder);
            }
        }
        if !grouped {
            for (index, port) in service.value().ports().iter().enumerate() {
                self.map_port(&subject, index, port, &mut builder);
            }
        }
        for (index, mount) in service.value().mounts().iter().enumerate() {
            self.map_mount(&subject, index, mount, &mut builder);
        }
        self.map_pod_or_networks(&subject, service, pod_name, &mut builder);

        self.finish_document(file_name, &builder, &subject, service.origins());
    }

    fn map_container_name(&mut self, subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        let Some(runtime_name) = service.runtime_name() else {
            return;
        };
        let runtime_subject = format!("{subject}.container_name");
        if !valid_podman_container_name(runtime_name.value().expose()) {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &runtime_subject,
                "explicit container name does not satisfy Podman's runtime name grammar",
                "container names must match `[a-zA-Z0-9][a-zA-Z0-9_.-]*`",
                runtime_name.origins(),
            );
        } else if self.capability(
            "quadlet.container.container-name",
            &runtime_subject,
            runtime_name.origins(),
        ) && self.push_container(
            builder,
            ContainerKey::ContainerName,
            runtime_name.value().expose(),
            &runtime_subject,
            runtime_name.origins(),
        ) {
            self.exact(runtime_subject, runtime_name.origins());
        }
    }

    fn map_restart_policy(
        &mut self,
        service_subject: &str,
        restart_policy: &Sourced<RestartPolicy>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.restart_policy");
        let (value, approximation) = match restart_policy.value() {
            RestartPolicy::Never => ("no", None),
            RestartPolicy::Always => (
                "always",
                Some("systemd does not reproduce every runtime-specific activation gate and daemon-restart rule"),
            ),
            RestartPolicy::OnFailure {
                maximum_retries: Some(_),
            } => {
                self.unsupported(
                    &subject,
                    "a finite container restart count has no equivalent in Restart=; systemd start-rate limits use different time-window semantics",
                    restart_policy.origins(),
                );
                return;
            }
            RestartPolicy::OnFailure { maximum_retries: None } => (
                "on-failure",
                Some(
                    "systemd on-failure also covers selected signals, timeouts, watchdog failures, and OOM termination",
                ),
            ),
            RestartPolicy::UnlessStopped => (
                "always",
                Some(
                    "systemd manual-stop handling cannot retain source-runtime unless-stopped state across runtime or host restarts",
                ),
            ),
            _ => {
                self.unsupported(
                    &subject,
                    "unknown container restart-policy form",
                    restart_policy.origins(),
                );
                return;
            }
        };
        if !self.capability("systemd.service.restart", &subject, restart_policy.origins())
            || !self.push_systemd(
                builder,
                SystemdSection::Service,
                "Restart",
                value,
                &subject,
                restart_policy.origins(),
            )
        {
            return;
        }
        if let Some(reason) = approximation {
            self.restart_approximation(&subject, reason, restart_policy.origins());
        } else {
            self.exact(subject, restart_policy.origins());
        }
    }

    fn map_metadata_labels(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        for label in service.labels() {
            let name = label.value().name().as_str();
            let subject = format!("{service_subject}.labels.{name}");
            if is_compose_managed_label(name) {
                self.unsupported(
                    &subject,
                    "Compose-managed labels cannot be safely re-authored as application metadata",
                    label.origins(),
                );
                continue;
            }
            let Some(encoded) = encode_quadlet_label(name, label.value().value().expose()) else {
                self.unsupported(
                    &subject,
                    "label names and values containing NUL cannot be represented by Quadlet or systemd",
                    label.origins(),
                );
                continue;
            };
            if self.capability("quadlet.container.label", &subject, label.origins())
                && self.push_container(builder, ContainerKey::Label, encoded, &subject, label.origins())
            {
                self.exact(subject, label.origins());
            }
        }
    }

    fn map_pod_or_networks(
        &mut self,
        service_subject: &str,
        service: &Sourced<Service>,
        pod_name: Option<&str>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let Some(pod_name) = pod_name else {
            for network in service.value().networks() {
                self.map_network_attachment(service_subject, network, builder);
            }
            return;
        };
        let subject = format!("{service_subject}.pod");
        if self.capability("quadlet.container.pod", &subject, service.origins())
            && self.push_container(
                builder,
                ContainerKey::Pod,
                format!("{pod_name}.pod"),
                &subject,
                service.origins(),
            )
        {
            self.exact(subject, service.origins());
        }
    }

    fn map_secret_grant(
        &mut self,
        service_subject: &str,
        index: usize,
        grant: &Sourced<ResourceGrant>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.secrets[{index}]");
        if grant.value().source().is_sensitive() {
            self.unsupported(
                &subject,
                "a sensitive interpolated secret identifier cannot be emitted into a unit file safely",
                grant.origins(),
            );
            return;
        }
        let source = grant.value().source().expose();
        let Some(secret) = self
            .application
            .secrets()
            .iter()
            .find(|secret| secret.value().name().as_str() == source)
        else {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &subject,
                "secret grant references a missing application resource",
                "declare the referenced secret before converting the service grant",
                grant.origins(),
            );
            return;
        };
        if secret.value().ownership() != ResourceOwnership::External || secret.value().material().is_some() {
            self.unsupported(
                &subject,
                "only pre-existing external Podman secrets can be consumed without an out-of-band creation step",
                &secret_grant_origins(grant, secret),
            );
            return;
        }

        let runtime_name = secret
            .value()
            .runtime_name()
            .map_or(secret.value().name().as_str(), |name| name.value().expose());
        if secret
            .value()
            .runtime_name()
            .is_some_and(|name| name.value().is_sensitive())
            || !is_safe_secret_component(runtime_name)
        {
            self.unsupported(
                &subject,
                "external secret runtime name cannot be encoded in the reviewed Quadlet Secret grammar",
                &secret_grant_origins(grant, secret),
            );
            return;
        }

        let options = match podman_secret_options(grant.value(), runtime_name, source) {
            Ok(options) => options,
            Err(reason) => {
                self.unsupported(&subject, reason, &secret_grant_origins(grant, secret));
                return;
            }
        };

        let value = if options.is_empty() {
            runtime_name.to_owned()
        } else {
            format!("{runtime_name},{}", options.join(","))
        };
        let origins = secret_grant_origins(grant, secret);
        if self.capability("quadlet.container.secret", &subject, &origins)
            && self.push_container(builder, ContainerKey::Secret, value, &subject, &origins)
        {
            self.exact(subject, &origins);
        }
    }

    fn map_execution_context(
        &mut self,
        service_subject: &str,
        service: &Service,
        grouped: bool,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if let Some(user) = service.user() {
            self.map_protected_container_value(
                service_subject,
                "user",
                user,
                "quadlet.container.user",
                ContainerKey::User,
                builder,
            );
        }
        if let Some(group) = service.group() {
            let subject = format!("{service_subject}.group");
            if group.value().expose().bytes().all(|byte| byte.is_ascii_digit()) && !group.value().expose().is_empty() {
                self.map_protected_container_value(
                    service_subject,
                    "group",
                    group,
                    "quadlet.container.group",
                    ContainerKey::Group,
                    builder,
                );
            } else {
                self.unsupported(
                    &subject,
                    "Quadlet Group requires a numeric GID in the supported native contract",
                    group.origins(),
                );
            }
        }
        if let Some(user_namespace) = service.user_namespace() {
            if !grouped {
                self.map_protected_container_value(
                    service_subject,
                    "user_namespace",
                    user_namespace,
                    "quadlet.container.userns",
                    ContainerKey::UserNS,
                    builder,
                );
            }
        }
        for (index, group) in service.supplementary_groups().iter().enumerate() {
            self.map_protected_container_value(
                service_subject,
                &format!("supplementary_groups[{index}]"),
                group,
                "quadlet.container.group-add",
                ContainerKey::GroupAdd,
                builder,
            );
        }
        if let Some(working_directory) = service.working_directory() {
            self.map_protected_container_value(
                service_subject,
                "working_directory",
                working_directory,
                "quadlet.container.working-dir",
                ContainerKey::WorkingDir,
                builder,
            );
        }
        if let Some(read_only) = service.read_only_root_filesystem() {
            let subject = format!("{service_subject}.read_only_root_filesystem");
            let value = if *read_only.value() { "true" } else { "false" };
            if self.capability("quadlet.container.read-only", &subject, read_only.origins())
                && self.push_container(builder, ContainerKey::ReadOnly, value, &subject, read_only.origins())
            {
                self.exact(subject, read_only.origins());
            }
        }
    }

    fn map_released_container_settings(
        &mut self,
        service_subject: &str,
        service: &Service,
        builder: &mut QuadletDocumentBuilder,
    ) {
        self.map_released_scalars(service_subject, service, builder);
        self.map_dns(service_subject, service, builder);
        self.map_capabilities(service_subject, service, builder);
        self.map_tmpfs(service_subject, service, builder);
        self.map_sysctls(service_subject, service, builder);
        self.map_ulimits(service_subject, service, builder);
        self.map_devices(service_subject, service, builder);
        self.map_stop_signal(service_subject, service, builder);
        self.map_security_options(service_subject, service, builder);
    }

    fn map_security_options(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        let Some(options) = service.security_options() else {
            return;
        };
        let collection_subject = format!("{service_subject}.security_options");
        if options.is_empty() {
            self.unsupported(
                &collection_subject,
                "an explicit empty security-option collection has no safe Quadlet reset encoding",
                service.security_options_origins(),
            );
            return;
        }

        let mut singleton_counts = BTreeMap::new();
        for option in options {
            if let Some(name) = security_option_singleton_name(option.value()) {
                *singleton_counts.entry(name).or_insert(0_usize) += 1;
            }
        }
        let all_origins = collection_all_origins(options, service.security_options_origins());
        for (name, count) in &singleton_counts {
            if *count > 1 {
                self.invalid(
                    self.exporter.codes.invalid_value.clone(),
                    &collection_subject,
                    "security-option singleton is declared more than once",
                    &format!("{name} occurs {count} times; retain one explicit value before generating Quadlet"),
                    &all_origins,
                );
            }
        }

        let disable_labels = options
            .iter()
            .any(|option| matches!(option.value(), SecurityOption::SecurityLabelDisable(true)));
        let other_labels = options
            .iter()
            .any(|option| security_option_is_selinux_label(option.value()));
        if disable_labels && other_labels {
            self.unsupported(
                &collection_subject,
                "SecurityLabelDisable=true conflicts with additional SELinux label settings; all native keys are retained but require explicit partial authorization",
                &all_origins,
            );
        }

        for (index, option) in options.iter().enumerate() {
            let subject = format!("{collection_subject}[{index}]");
            let Some((capability, key, value)) = security_option_output(option.value()) else {
                self.unsupported(&subject, "unknown security-option variant", option.origins());
                continue;
            };
            if security_option_singleton_name(option.value())
                .is_some_and(|name| singleton_counts.get(name).copied().unwrap_or_default() > 1)
            {
                continue;
            }
            if !is_safe_security_option_value(value) {
                self.unsupported(
                    &subject,
                    "security-option value requires quoting, contains a systemd specifier, or is not one safe physical line",
                    option.origins(),
                );
                continue;
            }
            if self.capability(capability, &subject, option.origins())
                && self.push_container(builder, key, value, &subject, option.origins())
            {
                self.exact(subject, option.origins());
            }
        }
    }

    fn map_released_scalars(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(hostname) = service.hostname() {
            self.map_raw_container_value(
                &format!("{service_subject}.hostname"),
                hostname,
                hostname.origins(),
                "quadlet.container.hostname",
                ContainerKey::HostName,
                builder,
                is_safe_hostname,
                "hostname must be a resolved systemd-safe token and must not rely on host UTS mode",
            );
        }
        if let Some(limit) = service.pids_limit() {
            let subject = format!("{service_subject}.pids_limit");
            let value = limit.value().expose();
            let valid = if value == "-1" {
                Some(PidsLimit::unlimited().as_str().to_owned())
            } else {
                PidsLimit::finite(value).ok().map(|value| value.as_str().to_owned())
            };
            self.map_validated_container_value(
                &subject,
                limit,
                valid,
                "quadlet.container.pids-limit",
                ContainerKey::PidsLimit,
                builder,
                "PID limit must be -1 or a positive ASCII decimal without normalization",
            );
        }
        if let Some(size) = service.shm_size() {
            let subject = format!("{service_subject}.shm_size");
            let valid = ShmSize::new(size.value().expose())
                .ok()
                .filter(|size| !size.is_unlimited())
                .map(|size| size.as_str().to_owned());
            self.map_validated_container_value(
                &subject,
                size,
                valid,
                "quadlet.container.shm-size",
                ContainerKey::ShmSize,
                builder,
                "shared-memory size must be a positive ASCII decimal with optional b, k, m, or g",
            );
        }
    }

    fn map_dns(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        self.map_dns_values(
            service_subject,
            "dns",
            service.dns_servers(),
            service.dns_servers_origins(),
            ContainerKey::DNS,
            builder,
        );
        self.map_dns_values(
            service_subject,
            "dns_opt",
            service.dns_options(),
            service.dns_options_origins(),
            ContainerKey::DNSOption,
            builder,
        );
        self.map_dns_values(
            service_subject,
            "dns_search",
            service.dns_search_domains(),
            service.dns_search_domains_origins(),
            ContainerKey::DNSSearch,
            builder,
        );
    }

    fn report_grouped_dns(&mut self, service_subject: &str, service: &Service, grouped: bool) {
        if !grouped {
            return;
        }
        for (name, values, collection_origins) in [
            ("dns", service.dns_servers(), service.dns_servers_origins()),
            ("dns_opt", service.dns_options(), service.dns_options_origins()),
            (
                "dns_search",
                service.dns_search_domains(),
                service.dns_search_domains_origins(),
            ),
        ] {
            if let Some(values) = values {
                let origins = collection_all_origins(values, collection_origins);
                self.unsupported(
                    &format!("{service_subject}.{name}"),
                    "generated Pod grouping can change effective shared-pod resolver configuration",
                    &origins,
                );
            }
        }
    }

    fn map_dns_values(
        &mut self,
        service_subject: &str,
        name: &str,
        values: Option<&[Sourced<boxferry_model::ProtectedString>]>,
        collection_origins: &[Provenance],
        key: ContainerKey,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let Some(values) = values else {
            return;
        };
        let subject = format!("{service_subject}.{name}");
        if values.is_empty() {
            self.unsupported(
                &subject,
                "an empty Quadlet DNS assignment resets native state and cannot represent an explicit empty collection",
                collection_origins,
            );
            return;
        }
        if key == ContainerKey::DNSOption {
            let mut seen = BTreeSet::new();
            if values.iter().any(|value| !seen.insert(value.value().expose())) {
                let origins = collection_all_origins(values, collection_origins);
                self.invalid(
                    self.exporter.codes.invalid_value.clone(),
                    &subject,
                    "DNS resolver options must be unique; duplicates cannot be emitted safely",
                    "remove duplicate dns_opt entries",
                    &origins,
                );
                return;
            }
        }
        for (index, value) in values.iter().enumerate() {
            let item_subject = format!("{subject}[{index}]");
            let raw = value.value().expose();
            let origins = collection_item_origins(collection_origins, value);
            if raw.is_empty() || raw.contains(['\0', '\r', '\n', '%', '$']) {
                self.unsupported(
                    &item_subject,
                    "DNS values must be non-empty resolved single physical lines without systemd specifiers",
                    &origins,
                );
                continue;
            }
            if (key == ContainerKey::DNS && raw == "none") || (key == ContainerKey::DNSSearch && raw == ".") {
                self.unsupported(
                    &item_subject,
                    "special DNS values have target-specific resolver semantics",
                    &origins,
                );
            }
            let emitted = self.capability(
                &format!(
                    "quadlet.container.{}",
                    match key {
                        ContainerKey::DNS => "dns",
                        ContainerKey::DNSOption => "dns-option",
                        _ => "dns-search",
                    }
                ),
                &item_subject,
                &origins,
            ) && self.push_container(builder, key, raw, &item_subject, &origins);
            if emitted
                && !((key == ContainerKey::DNS && raw == "none") || (key == ContainerKey::DNSSearch && raw == "."))
            {
                self.exact(item_subject, &origins);
            }
        }
    }

    fn map_tmpfs(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(tmpfs) = service.tmpfs() {
            if tmpfs.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.tmpfs"),
                    "an explicit empty tmpfs collection is retained but has no safe repeatable Quadlet reset encoding",
                    service.tmpfs_origins(),
                );
            }
            for (index, value) in tmpfs.iter().enumerate() {
                let origins = collection_item_origins(service.tmpfs_origins(), value);
                self.map_raw_container_value(
                    &format!("{service_subject}.tmpfs[{index}]"),
                    value,
                    &origins,
                    "quadlet.container.tmpfs",
                    ContainerKey::Tmpfs,
                    builder,
                    is_safe_tmpfs,
                    "tmpfs must be a resolved non-empty systemd-safe declaration",
                );
            }
        }
    }

    fn map_sysctls(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(sysctls) = service.sysctls() {
            if sysctls.is_empty() {
                self.unsupported(&format!("{service_subject}.sysctls"), "an explicit empty sysctls collection is retained but has no safe repeatable Quadlet reset encoding", service.sysctls_origins());
            }
            for (index, sysctl) in sysctls.iter().enumerate() {
                let subject = format!("{service_subject}.sysctls[{index}]");
                let origins = collection_item_origins(service.sysctls_origins(), sysctl);
                let name = sysctl.value().name().expose();
                let value = sysctl.value().value().expose();
                if !is_safe_sysctl(name, value) {
                    self.unsupported(
                        &subject,
                        "sysctl must use an unambiguous systemd-safe name=value spelling",
                        &origins,
                    );
                } else if self.capability("quadlet.container.sysctl", &subject, &origins)
                    && self.push_container(
                        builder,
                        ContainerKey::Sysctl,
                        format!("{name}={value}"),
                        &subject,
                        &origins,
                    )
                {
                    self.exact(subject, &origins);
                }
            }
        }
    }

    fn map_ulimits(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(ulimits) = service.ulimits() {
            if ulimits.is_empty() {
                self.unsupported(&format!("{service_subject}.ulimits"), "an explicit empty ulimits collection is retained but has no safe repeatable Quadlet reset encoding", service.ulimits_origins());
            }
            for (index, limit) in ulimits.iter().enumerate() {
                let subject = format!("{service_subject}.ulimits[{index}]");
                let origins = resource_limit_origins(service.ulimits_origins(), limit);
                let Some(soft) = limit.value().soft() else {
                    self.unsupported(
                        &subject,
                        "ulimit lacks a complete soft/hard scalar representation",
                        &origins,
                    );
                    continue;
                };
                let Some(hard) = limit.value().hard() else {
                    self.unsupported(
                        &subject,
                        "ulimit lacks a complete soft/hard scalar representation",
                        &origins,
                    );
                    continue;
                };
                let name = limit.value().name().expose();
                let soft_value = soft.value().expose();
                let hard_value = hard.value().expose();
                if !is_safe_ulimit(name, soft_value, hard_value) {
                    self.unsupported(
                        &subject,
                        "ulimit requires a lowercase name and -1 or non-negative ASCII decimal soft/hard values",
                        &origins,
                    );
                } else if self.capability("quadlet.container.ulimit", &subject, &origins)
                    && self.push_container(
                        builder,
                        ContainerKey::Ulimit,
                        format!("{name}={soft_value}:{hard_value}"),
                        &subject,
                        &origins,
                    )
                {
                    self.exact(subject, &origins);
                }
            }
        }
    }

    fn map_devices(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(devices) = service.devices() {
            if devices.is_empty() {
                self.unsupported(&format!("{service_subject}.devices"), "an explicit empty devices collection is retained but has no safe repeatable Quadlet reset encoding", service.devices_origins());
            }
            for (index, device) in devices.iter().enumerate() {
                let subject = format!("{service_subject}.devices[{index}]");
                let origins = device_origins(service.devices_origins(), device);
                let Some(value) = reviewed_device_value(device.value()) else {
                    self.unsupported(
                        &subject,
                        "device is CDI, opaque, deferred, or not a reviewed host-device spelling",
                        &origins,
                    );
                    continue;
                };
                if self.capability("quadlet.container.add-device", &subject, &origins)
                    && self.push_container(builder, ContainerKey::AddDevice, value, &subject, &origins)
                {
                    self.exact(subject, &origins);
                }
            }
        }
    }

    fn map_stop_signal(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        if let Some(signal) = service.stop_signal() {
            self.map_raw_container_value(
                &format!("{service_subject}.stop_signal"),
                signal,
                signal.origins(),
                "quadlet.container.stop-signal",
                ContainerKey::StopSignal,
                builder,
                is_safe_stop_signal,
                "stop signal must be a non-empty systemd-safe token or number without grammar normalization",
            );
        }
    }

    fn map_capabilities(&mut self, service_subject: &str, service: &Service, builder: &mut QuadletDocumentBuilder) {
        for (field, values, collection_origins, capability, key) in [
            (
                "cap_drop",
                service.cap_drop(),
                service.cap_drop_origins(),
                "quadlet.container.drop-capability",
                ContainerKey::DropCapability,
            ),
            (
                "cap_add",
                service.cap_add(),
                service.cap_add_origins(),
                "quadlet.container.add-capability",
                ContainerKey::AddCapability,
            ),
        ] {
            let Some(values) = values else {
                continue;
            };
            if values.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.{field}"),
                    "an explicit empty capability collection is retained but has no safe repeatable Quadlet reset encoding",
                    collection_origins,
                );
            }
            for (index, value) in values.iter().enumerate() {
                let origins = collection_item_origins(collection_origins, value);
                self.map_raw_container_value(
                    &format!("{service_subject}.{field}[{index}]"),
                    value,
                    &origins,
                    capability,
                    key,
                    builder,
                    |value| is_safe_word(value, false),
                    "capability names must be non-empty systemd-safe values; values are not normalized or reconciled",
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_raw_container_value(
        &mut self,
        subject: &str,
        value: &Sourced<boxferry_model::ProtectedString>,
        origins: &[Provenance],
        capability: &str,
        key: ContainerKey,
        builder: &mut QuadletDocumentBuilder,
        validate: impl FnOnce(&str) -> bool,
        reason: &str,
    ) {
        if !validate(value.value().expose()) {
            self.unsupported(subject, reason, origins);
        } else if self.capability(capability, subject, origins)
            && self.push_container(builder, key, value.value().expose(), subject, origins)
        {
            self.exact(subject, origins);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_validated_container_value(
        &mut self,
        subject: &str,
        value: &Sourced<boxferry_model::ProtectedString>,
        encoded: Option<String>,
        capability: &str,
        key: ContainerKey,
        builder: &mut QuadletDocumentBuilder,
        reason: &str,
    ) {
        let Some(encoded) = encoded else {
            self.unsupported(subject, reason, value.origins());
            return;
        };
        if self.capability(capability, subject, value.origins())
            && self.push_container(builder, key, encoded, subject, value.origins())
        {
            self.exact(subject, value.origins());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_protected_container_value(
        &mut self,
        service_subject: &str,
        field: &str,
        value: &Sourced<boxferry_model::ProtectedString>,
        capability: &str,
        key: ContainerKey,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.{field}");
        if !is_safe_word(value.value().expose(), false) {
            self.unsupported(
                &subject,
                "execution-context value requires systemd quoting or target-specific validation not yet encoded",
                value.origins(),
            );
            return;
        }
        if self.capability(capability, &subject, value.origins())
            && self.push_container(builder, key, value.value().expose(), &subject, value.origins())
        {
            self.exact(subject, value.origins());
        }
    }

    fn map_service_dependencies(&mut self, service: &Sourced<Service>, builder: &mut QuadletDocumentBuilder) {
        let service_name = service.value().name().as_str();
        for (index, dependency) in service.value().dependencies().iter().enumerate() {
            self.map_service_dependency(service_name, index, dependency, builder);
        }
    }

    fn map_service_dependency(
        &mut self,
        service_name: &str,
        index: usize,
        dependency: &Sourced<ServiceDependency>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("services.{service_name}.dependencies[{index}]");
        let target_name = dependency.value().service().as_str();
        let origins = dependency_mapping_origins(dependency);
        let Some(target) = self
            .application
            .services()
            .iter()
            .find(|candidate| candidate.value().name().as_str() == target_name)
        else {
            self.loss(
                self.exporter.codes.dependency.clone(),
                &subject,
                ConversionKind::Unsupported,
                "optional service dependency is absent from the application",
                &format!("optional service `{target_name}` cannot be ordered or activated"),
                &origins,
            );
            return;
        };
        if !self.dependency_condition_supported(&subject, dependency, target.value(), &origins) {
            return;
        }

        let target_unit = format!("{target_name}.service");
        let activation = if dependency.value().is_required() {
            ("systemd.unit.requires", SystemdUnitKey::Requires)
        } else {
            ("systemd.unit.wants", SystemdUnitKey::Wants)
        };
        let activation_supported = self.capability(activation.0, &subject, &origins);
        let ordering_supported = self.capability("systemd.unit.after", &subject, &origins);
        if activation_supported
            && ordering_supported
            && self.push_systemd_unit(builder, activation.1, &target_unit, &subject, &origins)
            && self.push_systemd_unit(builder, SystemdUnitKey::After, target_unit, &subject, &origins)
        {
            self.exact(&subject, &origins);
        }
        self.map_dependency_restart(&subject, dependency);
    }

    fn dependency_condition_supported(
        &mut self,
        subject: &str,
        dependency: &Sourced<ServiceDependency>,
        target: &Service,
        origins: &[Provenance],
    ) -> bool {
        match dependency
            .value()
            .condition()
            .map_or(&ServiceDependencyCondition::Started, Sourced::value)
        {
            ServiceDependencyCondition::Started => true,
            ServiceDependencyCondition::Healthy if service_has_renderable_healthcheck(target) => true,
            ServiceDependencyCondition::Healthy => {
                self.loss(
                    self.exporter.codes.dependency.clone(),
                    subject,
                    ConversionKind::Unsupported,
                    "healthy service dependency cannot be established for the target",
                    "referenced service has no explicit, enabled, safely encodable health command",
                    origins,
                );
                false
            }
            ServiceDependencyCondition::CompletedSuccessfully => {
                self.loss(
                    self.exporter.codes.dependency.clone(),
                    subject,
                    ConversionKind::Unsupported,
                    "successful-completion dependency has no verified Quadlet mapping",
                    "ordinary Quadlet container services do not provide the required one-shot completion contract",
                    origins,
                );
                false
            }
            ServiceDependencyCondition::Other(_) => {
                self.loss(
                    self.exporter.codes.dependency.clone(),
                    subject,
                    ConversionKind::Unsupported,
                    "source-specific dependency condition has no verified Quadlet mapping",
                    "retain or replace the provider-specific condition manually",
                    origins,
                );
                false
            }
            _ => {
                self.loss(
                    self.exporter.codes.dependency.clone(),
                    subject,
                    ConversionKind::Unsupported,
                    "unknown dependency condition has no verified Quadlet mapping",
                    "upgrade BoxFerry or select a supported condition",
                    origins,
                );
                false
            }
        }
    }

    fn map_dependency_restart(&mut self, subject: &str, dependency: &Sourced<ServiceDependency>) {
        let Some(restart) = dependency.value().restart() else {
            return;
        };
        let restart_subject = format!("{subject}.restart");
        if *restart.value() {
            self.loss(
                self.exporter.codes.dependency.clone(),
                &restart_subject,
                ConversionKind::Unsupported,
                "Compose-controlled dependency restart propagation has no verified Quadlet mapping",
                "systemd runtime restart relationships are not equivalent to explicit Compose update operations",
                restart.origins(),
            );
        } else {
            self.exact(restart_subject, restart.origins());
        }
    }

    fn map_healthy_readiness(&mut self, service: &Sourced<Service>, builder: &mut QuadletDocumentBuilder) {
        let name = service.value().name().as_str();
        if !service_has_renderable_healthcheck(service.value()) {
            return;
        }
        let mut origins = self
            .application
            .services()
            .iter()
            .flat_map(|dependent| dependent.value().dependencies())
            .filter(|dependency| {
                dependency.value().service().as_str() == name
                    && matches!(
                        dependency.value().condition().map(Sourced::value),
                        Some(ServiceDependencyCondition::Healthy)
                    )
            })
            .flat_map(dependency_mapping_origins)
            .collect::<Vec<_>>();
        if origins.is_empty() {
            return;
        }
        if let Some(healthcheck) = service.value().healthcheck() {
            for origin in healthcheck.origins() {
                if !origins.contains(origin) {
                    origins.push(origin.clone());
                }
            }
        }
        let subject = format!("services.{name}.readiness");
        if self.capability("quadlet.container.notify-healthy", &subject, &origins)
            && self.push_container(builder, ContainerKey::Notify, "healthy", &subject, &origins)
        {
            self.exact(subject, &origins);
        }
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

    fn map_healthcheck(
        &mut self,
        service_subject: &str,
        healthcheck: &Sourced<Healthcheck>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let disabled = healthcheck.value().disabled().is_some_and(|value| *value.value());
        if disabled && healthcheck.value().command().is_some() {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{service_subject}.healthcheck"),
                "health check cannot be both disabled and assigned a command",
                "resolve the conflicting neutral health-check intent before target planning",
                healthcheck.origins(),
            );
            return;
        }

        if let Some(disable) = healthcheck.value().disabled() {
            let subject = format!("{service_subject}.healthcheck.disable");
            if *disable.value() {
                if self.capability("quadlet.container.health-command", &subject, disable.origins())
                    && self.push_container(builder, ContainerKey::HealthCmd, "none", &subject, disable.origins())
                {
                    self.exact(subject, disable.origins());
                }
            } else {
                self.exact(subject, disable.origins());
            }
        }

        if !disabled {
            if let Some(command) = healthcheck.value().command() {
                self.map_healthcheck_command(service_subject, command, builder);
            }
        }
        if let Some(interval) = healthcheck.value().interval() {
            self.map_healthcheck_scalar(
                service_subject,
                "interval",
                interval.value().as_str(),
                interval.origins(),
                "quadlet.container.health-interval",
                ContainerKey::HealthInterval,
                HealthcheckScalarKind::Duration,
                builder,
            );
        }
        if let Some(timeout) = healthcheck.value().timeout() {
            self.map_healthcheck_scalar(
                service_subject,
                "timeout",
                timeout.value().as_str(),
                timeout.origins(),
                "quadlet.container.health-timeout",
                ContainerKey::HealthTimeout,
                HealthcheckScalarKind::Duration,
                builder,
            );
        }
        if let Some(retries) = healthcheck.value().retries() {
            self.map_healthcheck_scalar(
                service_subject,
                "retries",
                retries.value().as_str(),
                retries.origins(),
                "quadlet.container.health-retries",
                ContainerKey::HealthRetries,
                HealthcheckScalarKind::RetryCount,
                builder,
            );
        }
        if let Some(start_period) = healthcheck.value().start_period() {
            self.map_healthcheck_scalar(
                service_subject,
                "start_period",
                start_period.value().as_str(),
                start_period.origins(),
                "quadlet.container.health-start-period",
                ContainerKey::HealthStartPeriod,
                HealthcheckScalarKind::Duration,
                builder,
            );
        }
        if let Some(start_interval) = healthcheck.value().start_interval() {
            self.unsupported(
                &format!("{service_subject}.healthcheck.start_interval"),
                "Quadlet has no native HealthStartInterval key and no verified fallback is enabled",
                start_interval.origins(),
            );
        }
    }

    fn map_healthcheck_command(
        &mut self,
        service_subject: &str,
        command: &Sourced<HealthcheckCommand>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.healthcheck.test");
        let arguments = match command.value() {
            HealthcheckCommand::Exec(arguments) if !arguments.is_empty() => {
                let mut values = Vec::with_capacity(arguments.len() + 1);
                values.push("CMD");
                values.extend(arguments.iter().map(boxferry_model::ProtectedString::expose));
                values
            }
            HealthcheckCommand::Shell(value) if !value.expose().is_empty() => {
                vec!["CMD-SHELL", value.expose()]
            }
            HealthcheckCommand::Exec(_) => {
                self.invalid(
                    self.exporter.codes.invalid_value.clone(),
                    &subject,
                    "health-check exec command requires at least one argument",
                    "supply a command or explicitly disable the health check",
                    command.origins(),
                );
                return;
            }
            HealthcheckCommand::Shell(_) => {
                self.invalid(
                    self.exporter.codes.invalid_value.clone(),
                    &subject,
                    "health-check shell command must not be empty",
                    "supply shell command text or explicitly disable the health check",
                    command.origins(),
                );
                return;
            }
            _ => {
                self.unsupported(&subject, "unknown health-check command form", command.origins());
                return;
            }
        };
        if arguments
            .iter()
            .any(|argument| argument.contains('\0') || argument.contains('%'))
        {
            self.unsupported(
                &subject,
                "health command contains a NUL byte or unresolved systemd percent specifier",
                command.origins(),
            );
            return;
        }
        let encoded = encode_json_array(&arguments);
        if self.capability("quadlet.container.health-command", &subject, command.origins())
            && self.push_container(builder, ContainerKey::HealthCmd, encoded, &subject, command.origins())
        {
            self.exact(subject, command.origins());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_healthcheck_scalar(
        &mut self,
        service_subject: &str,
        field: &str,
        value: &str,
        origins: &[Provenance],
        capability: &str,
        key: ContainerKey,
        kind: HealthcheckScalarKind,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.healthcheck.{field}");
        let valid = is_safe_health_scalar(value)
            && match kind {
                HealthcheckScalarKind::Duration => is_podman_health_duration(value),
                HealthcheckScalarKind::RetryCount => value.bytes().all(|byte| byte.is_ascii_digit()),
            };
        if !valid {
            self.unsupported(
                &subject,
                "health-check scalar is not valid for the selected Podman target",
                origins,
            );
            return;
        }
        if self.capability(capability, &subject, origins) && self.push_container(builder, key, value, &subject, origins)
        {
            self.exact(subject, origins);
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

    fn map_environment_file(
        &mut self,
        service_subject: &str,
        index: usize,
        environment_file: &Sourced<EnvironmentFile>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.env_file[{index}]");
        let origins = environment_file_origins(environment_file);
        if !environment_file.value().is_required() {
            self.unsupported(
                &subject,
                "Quadlet EnvironmentFile has no documented Compose-compatible optional-file setting",
                &origins,
            );
            return;
        }

        let authored_path = environment_file.value().path().expose();
        let path = match classify_path(authored_path) {
            PathForm::AbsoluteLiteral => normalize_absolute_path(authored_path),
            PathForm::UnitRelativeLiteral | PathForm::RelativeLiteral
                if is_safe_mount_part(authored_path) && !authored_path.starts_with('~') =>
            {
                self.exporter
                    .relative_bind_root
                    .as_deref()
                    .and_then(|root| resolve_relative_path(root, authored_path))
            }
            _ => None,
        };
        let Some(path) = path else {
            let reason = match classify_path(authored_path) {
                PathForm::UnitRelativeLiteral | PathForm::RelativeLiteral
                    if self.exporter.relative_bind_root.is_none() =>
                {
                    "relative environment-file path needs an explicit Compose project root"
                }
                PathForm::SystemdSpecifier => {
                    "Compose environment-file paths cannot acquire systemd specifier semantics implicitly"
                }
                _ => "environment-file path is not a safely encodable POSIX path",
            };
            self.unsupported(&subject, reason, &origins);
            return;
        };

        if self.capability("quadlet.container.environment-file", &subject, &origins)
            && self.push_container(builder, ContainerKey::EnvironmentFile, path, &subject, &origins)
        {
            let reason = match environment_file.value().format().map(Sourced::value) {
                Some(EnvironmentFileFormat::Raw) => {
                    "Compose raw env-file parsing has no explicit Quadlet selector; Podman parser parity is not yet proven"
                }
                Some(_) => "the requested env-file parser mode has no proven Quadlet equivalent",
                None => "Compose default env-file parsing and Podman env-file parsing are not yet proven equivalent",
            };
            self.environment_file_approximation(&subject, reason, &origins);
        }
    }

    fn map_container_host_mapping(
        &mut self,
        service_subject: &str,
        index: usize,
        mapping: &Sourced<HostMapping>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.host_mappings[{index}]");
        let Some(encoded) = self.encode_host_mapping(&subject, mapping.value(), mapping.origins()) else {
            return;
        };
        if self.capability("quadlet.container.add-host", &subject, mapping.origins())
            && self.push_container(builder, ContainerKey::AddHost, encoded, &subject, mapping.origins())
        {
            self.exact(subject, mapping.origins());
        }
    }

    fn map_pod_host_mapping(
        &mut self,
        index: usize,
        mapping: &HostMapping,
        origins: &[Provenance],
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("application.pod.host_mappings[{index}]");
        let Some(encoded) = self.encode_host_mapping(&subject, mapping, origins) else {
            return;
        };
        if self.capability("quadlet.pod.add-host", &subject, origins)
            && self.push_pod(builder, PodKey::AddHost, encoded, &subject, origins)
        {
            self.exact(subject, origins);
        }
    }

    fn encode_host_mapping(&mut self, subject: &str, mapping: &HostMapping, origins: &[Provenance]) -> Option<String> {
        let hostname = mapping.hostname().as_str();
        if !is_native_atom(hostname) {
            self.unsupported(subject, "host mapping name requires target-specific encoding", origins);
            return None;
        }
        let address = match mapping.address().kind() {
            HostAddressKind::Ipv4 | HostAddressKind::HostGateway | HostAddressKind::Ipv6 { bracketed: true } => {
                mapping.address().raw().to_owned()
            }
            HostAddressKind::Ipv6 { bracketed: false } => format!("[{}]", mapping.address().raw()),
            HostAddressKind::Other => {
                self.unsupported(
                    subject,
                    "host mapping address is deferred or implementation-specific",
                    origins,
                );
                return None;
            }
            _ => {
                self.unsupported(subject, "unknown host mapping address kind", origins);
                return None;
            }
        };
        Some(format!("{hostname}:{address}"))
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
            MountSource::HostPath(path) => {
                if let Some(mapped) = self.exporter.bind_source_mappings.get(path) {
                    Some(mapped.clone())
                } else {
                    match classify_path(path) {
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
                                "relative bind form needs an explicit source-to-target mapping",
                                mount.origins(),
                            );
                            None
                        }
                        _ => {
                            self.unsupported(
                                &subject,
                                "bind source needs an explicit source-to-target mapping for Quadlet",
                                mount.origins(),
                            );
                            None
                        }
                    }
                }
            }
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

    fn push_systemd_unit(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: SystemdUnitKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_systemd_unit(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn push_systemd(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        section: SystemdSection,
        key: &str,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_systemd(section, key, value)) {
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

    fn restart_approximation(&mut self, subject: &str, reason: &str, origins: &[Provenance]) {
        self.loss(
            self.exporter.codes.restart.clone(),
            subject,
            ConversionKind::Approximate,
            "container restart behavior is approximated by the systemd service manager",
            reason,
            origins,
        );
    }

    fn environment_file_approximation(&mut self, subject: &str, reason: &str, origins: &[Provenance]) {
        self.loss(
            self.exporter.codes.environment_file.clone(),
            subject,
            ConversionKind::Approximate,
            "environment-file parsing is delegated to Podman",
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

fn service_has_renderable_healthcheck(service: &Service) -> bool {
    let Some(healthcheck) = service.healthcheck() else {
        return false;
    };
    if healthcheck.value().disabled().is_some_and(|disabled| *disabled.value()) {
        return false;
    }
    match healthcheck.value().command().map(Sourced::value) {
        Some(HealthcheckCommand::Exec(arguments)) => {
            !arguments.is_empty()
                && arguments
                    .iter()
                    .all(|argument| !argument.expose().contains(['\0', '%']))
        }
        Some(HealthcheckCommand::Shell(value)) => !value.expose().is_empty() && !value.expose().contains(['\0', '%']),
        _ => false,
    }
}

fn dependency_mapping_origins(dependency: &Sourced<ServiceDependency>) -> Vec<Provenance> {
    let mut origins = dependency.origins().to_vec();
    if let Some(condition) = dependency.value().condition() {
        for origin in condition.origins() {
            if !origins.contains(origin) {
                origins.push(origin.clone());
            }
        }
    }
    if let Some(required) = dependency.value().required() {
        for origin in required.origins() {
            if !origins.contains(origin) {
                origins.push(origin.clone());
            }
        }
    }
    origins
}

fn collection_item_origins<T>(collection_origins: &[Provenance], item: &Sourced<T>) -> Vec<Provenance> {
    let mut origins = collection_origins.to_vec();
    extend_origins(&mut origins, item.origins());
    origins
}

fn collection_all_origins<T>(values: &[Sourced<T>], collection_origins: &[Provenance]) -> Vec<Provenance> {
    let mut origins = collection_origins.to_vec();
    for value in values {
        for origin in value.origins() {
            if !origins.contains(origin) {
                origins.push(origin.clone());
            }
        }
    }
    origins
}

fn resource_limit_origins(
    collection_origins: &[Provenance],
    limit: &Sourced<boxferry_model::ResourceLimit>,
) -> Vec<Provenance> {
    let mut origins = collection_item_origins(collection_origins, limit);
    for value in [limit.value().soft(), limit.value().hard()].into_iter().flatten() {
        extend_origins(&mut origins, value.origins());
    }
    origins
}

fn device_origins(collection_origins: &[Provenance], device: &Sourced<Device>) -> Vec<Provenance> {
    let mut origins = collection_item_origins(collection_origins, device);
    if let Device::Long {
        source,
        target,
        permissions,
    } = device.value()
    {
        for value in [source.as_ref(), target.as_ref(), permissions.as_ref()]
            .into_iter()
            .flatten()
        {
            extend_origins(&mut origins, value.origins());
        }
    }
    origins
}

fn extend_origins(origins: &mut Vec<Provenance>, additional: &[Provenance]) {
    for origin in additional {
        if !origins.contains(origin) {
            origins.push(origin.clone());
        }
    }
}

fn same_host_mappings(left: &[Sourced<HostMapping>], right: &[Sourced<HostMapping>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.value() == right.value())
}

fn shared_host_mapping_origins(services: &[Sourced<Service>], index: usize) -> Vec<Provenance> {
    let mut origins = Vec::new();
    for origin in services
        .iter()
        .filter_map(|service| service.value().host_mappings().get(index))
        .flat_map(Sourced::origins)
    {
        if !origins.contains(origin) {
            origins.push(origin.clone());
        }
    }
    origins
}

fn shared_user_namespace_origins(services: &[Sourced<Service>]) -> Vec<Provenance> {
    services
        .iter()
        .filter_map(|service| service.value().user_namespace())
        .flat_map(Sourced::origins)
        .cloned()
        .collect()
}

fn service_origins(services: &[Sourced<Service>]) -> Vec<Provenance> {
    services
        .iter()
        .flat_map(|service| service.origins().iter().cloned())
        .collect()
}

fn service_group_origins(
    group: &Sourced<boxferry_model::ServiceGroup>,
    services: &[Sourced<Service>],
) -> Vec<Provenance> {
    let mut origins = group.origins().to_vec();
    for origin in group
        .value()
        .members()
        .iter()
        .flat_map(Sourced::origins)
        .chain(services.iter().flat_map(Sourced::origins))
    {
        if !origins.contains(origin) {
            origins.push(origin.clone());
        }
    }
    origins
}

fn secret_grant_origins(grant: &Sourced<ResourceGrant>, secret: &Sourced<Secret>) -> Vec<Provenance> {
    let mut origins = grant.origins().to_vec();
    for sourced in [
        grant.value().target(),
        grant.value().uid(),
        grant.value().gid(),
        grant.value().mode(),
        secret.value().runtime_name(),
    ]
    .into_iter()
    .flatten()
    {
        for origin in sourced.origins() {
            if !origins.contains(origin) {
                origins.push(origin.clone());
            }
        }
    }
    for origin in secret.origins() {
        if !origins.contains(origin) {
            origins.push(origin.clone());
        }
    }
    origins
}

fn environment_file_origins(environment_file: &Sourced<EnvironmentFile>) -> Vec<Provenance> {
    let mut origins = environment_file.origins().to_vec();
    for sourced in [
        environment_file.value().required().map(Sourced::origins),
        environment_file.value().format().map(Sourced::origins),
    ]
    .into_iter()
    .flatten()
    .flatten()
    {
        if !origins.contains(sourced) {
            origins.push(sourced.clone());
        }
    }
    origins
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

fn encode_json_array(values: &[&str]) -> String {
    let mut encoded = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            encoded.push(',');
        }
        encoded.push('"');
        for character in value.chars() {
            match character {
                '"' => encoded.push_str("\\\""),
                '\\' => encoded.push_str("\\\\"),
                '\u{08}' => encoded.push_str("\\b"),
                '\u{0c}' => encoded.push_str("\\f"),
                '\n' => encoded.push_str("\\n"),
                '\r' => encoded.push_str("\\r"),
                '\t' => encoded.push_str("\\t"),
                character if character <= '\u{1f}' => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    let value = usize::try_from(u32::from(character)).unwrap_or_default();
                    encoded.push_str("\\u00");
                    encoded.push(char::from(HEX[(value >> 4) & 0x0f]));
                    encoded.push(char::from(HEX[value & 0x0f]));
                }
                character => encoded.push(character),
            }
        }
        encoded.push('"');
    }
    encoded.push(']');
    encoded
}

fn is_safe_health_scalar(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.bytes().any(|byte| matches!(byte, b'\0' | b'\n' | b'\r' | b'%'))
}

fn is_podman_health_duration(mut value: &str) -> bool {
    if value == "0" {
        return true;
    }
    let mut found = false;
    while !value.is_empty() {
        let number_end = value
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
            .map(|(index, character)| index + character.len_utf8())
            .last()
            .unwrap_or(0);
        if number_end == 0 {
            return false;
        }
        let number = &value[..number_end];
        if number.matches('.').count() > 1 || number == "." {
            return false;
        }
        value = &value[number_end..];
        let Some(unit) = ["ns", "us", "µs", "μs", "ms", "s", "m", "h"]
            .into_iter()
            .find(|unit| value.starts_with(unit))
        else {
            return false;
        };
        value = &value[unit.len()..];
        found = true;
    }
    found
}

fn is_safe_word(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',')
        })
}

fn is_safe_security_option_value(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',' | b'*'
                )
        })
}

fn security_option_singleton_name(option: &SecurityOption) -> Option<&'static str> {
    match option {
        SecurityOption::AppArmor(_) => Some("AppArmor"),
        SecurityOption::NoNewPrivileges(_) => Some("NoNewPrivileges"),
        SecurityOption::SeccompProfile(_) => Some("SeccompProfile"),
        SecurityOption::SecurityLabelDisable(_) => Some("SecurityLabelDisable"),
        SecurityOption::SecurityLabelFileType(_) => Some("SecurityLabelFileType"),
        SecurityOption::SecurityLabelLevel(_) => Some("SecurityLabelLevel"),
        SecurityOption::SecurityLabelNested(_) => Some("SecurityLabelNested"),
        SecurityOption::SecurityLabelType(_) => Some("SecurityLabelType"),
        _ => None,
    }
}

fn security_option_is_selinux_label(option: &SecurityOption) -> bool {
    matches!(
        option,
        SecurityOption::SecurityLabelFileType(_)
            | SecurityOption::SecurityLabelLevel(_)
            | SecurityOption::SecurityLabelNested(_)
            | SecurityOption::SecurityLabelType(_)
    )
}

fn security_option_output(option: &SecurityOption) -> Option<(&'static str, ContainerKey, &str)> {
    match option {
        SecurityOption::AppArmor(value) => Some(("quadlet.container.apparmor", ContainerKey::AppArmor, value.expose())),
        SecurityOption::NoNewPrivileges(value) => Some((
            "quadlet.container.no-new-privileges",
            ContainerKey::NoNewPrivileges,
            if *value { "true" } else { "false" },
        )),
        SecurityOption::SeccompProfile(value) => Some((
            "quadlet.container.seccomp-profile",
            ContainerKey::SeccompProfile,
            value.expose(),
        )),
        SecurityOption::SecurityLabelDisable(value) => Some((
            "quadlet.container.security-label-disable",
            ContainerKey::SecurityLabelDisable,
            if *value { "true" } else { "false" },
        )),
        SecurityOption::SecurityLabelFileType(value) => Some((
            "quadlet.container.security-label-file-type",
            ContainerKey::SecurityLabelFileType,
            value.expose(),
        )),
        SecurityOption::SecurityLabelLevel(value) => Some((
            "quadlet.container.security-label-level",
            ContainerKey::SecurityLabelLevel,
            value.expose(),
        )),
        SecurityOption::SecurityLabelNested(value) => Some((
            "quadlet.container.security-label-nested",
            ContainerKey::SecurityLabelNested,
            if *value { "true" } else { "false" },
        )),
        SecurityOption::SecurityLabelType(value) => Some((
            "quadlet.container.security-label-type",
            ContainerKey::SecurityLabelType,
            value.expose(),
        )),
        SecurityOption::Mask(value) => Some(("quadlet.container.mask", ContainerKey::Mask, value.expose())),
        SecurityOption::Unmask(value) => Some(("quadlet.container.unmask", ContainerKey::Unmask, value.expose())),
        _ => None,
    }
}

fn is_safe_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.is_ascii()
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.bytes().next().is_some_and(|byte| byte.is_ascii_alphanumeric())
                && label.bytes().last().is_some_and(|byte| byte.is_ascii_alphanumeric())
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_safe_tmpfs(value: &str) -> bool {
    is_safe_word(value, false)
}

fn is_safe_sysctl(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains('=') && is_safe_word(name, false) && is_safe_word(value, true)
}

fn is_safe_limit_value(value: &str) -> bool {
    value == "-1" || (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_safe_ulimit(name: &str, soft: &str, hard: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| byte.is_ascii_lowercase())
        && is_safe_limit_value(soft)
        && is_safe_limit_value(hard)
}

fn is_safe_stop_signal(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn reviewed_device_value(device: &Device) -> Option<String> {
    match device {
        Device::Short(value) => reviewed_device_spelling(value.expose()),
        Device::Long {
            source,
            target,
            permissions,
        } => {
            let source = source.as_ref()?.value().expose();
            let target = target.as_ref().map(|value| value.value().expose());
            let permissions = permissions.as_ref().map(|value| value.value().expose());
            if !is_reviewed_device_component(source)
                || target.is_some_and(|target| !is_reviewed_device_component(target))
                || permissions.is_some_and(|permissions| !is_reviewed_device_permissions(permissions))
            {
                return None;
            }
            let mut rendered = source.to_owned();
            if let Some(target) = target {
                rendered.push(':');
                rendered.push_str(target);
            }
            if let Some(permissions) = permissions {
                target?;
                rendered.push(':');
                rendered.push_str(permissions);
            }
            Some(rendered)
        }
        _ => None,
    }
}

fn reviewed_device_spelling(value: &str) -> Option<String> {
    let mut parts = value.split(':');
    let source = parts.next()?;
    let target = parts.next();
    let permissions = parts.next();
    if parts.next().is_some()
        || !is_reviewed_device_component(source)
        || target.is_some_and(|target| !is_reviewed_device_component(target))
        || permissions.is_some_and(|permissions| !is_reviewed_device_permissions(permissions))
        || permissions.is_some() && target.is_none()
    {
        return None;
    }
    Some(value.to_owned())
}

fn is_reviewed_device_component(value: &str) -> bool {
    value.starts_with("/dev/")
        && value.len() > 5
        && value.is_ascii()
        && !value.contains(['$', '%', ':', '\0', '\r', '\n'])
        && !value.chars().any(char::is_whitespace)
        && value.split('/').all(|segment| !matches!(segment, "." | ".."))
}

fn is_reviewed_device_permissions(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| matches!(byte, b'r' | b'w' | b'm'))
}

fn valid_podman_container_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn encode_quadlet_label(name: &str, value: &str) -> Option<String> {
    if name.contains('\0') || value.contains('\0') {
        return None;
    }
    let mut encoded = String::with_capacity(name.len() + value.len() + 3);
    encoded.push('"');
    for character in name.chars().chain(std::iter::once('=')).chain(value.chars()) {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '%' => encoded.push_str("%%"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                write!(&mut encoded, "\\u{:04x}", u32::from(character)).ok()?;
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    Some(encoded)
}

fn is_compose_managed_label(name: &str) -> bool {
    name.starts_with("com.docker.compose.")
}

fn is_safe_secret_component(value: &str) -> bool {
    is_safe_word(value, false) && !value.contains([',', '='])
}

fn podman_secret_options(grant: &ResourceGrant, runtime_name: &str, source: &str) -> Result<Vec<String>, &'static str> {
    let mut options = Vec::new();
    match grant.target() {
        Some(target) if is_safe_secret_component(target.value().expose()) => {
            options.push(format!("target={}", target.value().expose()));
        }
        Some(_) => return Err("secret target cannot be encoded in the reviewed Quadlet Secret grammar"),
        None if runtime_name != source => options.push(format!("target={source}")),
        None => {}
    }
    for (name, value) in [("uid", grant.uid()), ("gid", grant.gid())] {
        if let Some(value) = value {
            if value.value().expose().is_empty() || !value.value().expose().bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("secret UID and GID options require non-negative decimal integers");
            }
            options.push(format!("{name}={}", value.value().expose()));
        }
    }
    if let Some(mode) = grant.mode() {
        let Some(mode) = normalize_secret_mode(mode.value().expose()) else {
            return Err("secret mode must be a one-to-four-digit octal value without writable bits");
        };
        options.push(format!("mode={mode}"));
    }
    Ok(options)
}

fn normalize_secret_mode(value: &str) -> Option<String> {
    let digits = value.strip_prefix("0o").unwrap_or(value);
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return None;
    }
    let mode = u16::from_str_radix(digits, 8).ok()?;
    if mode > 0o777 || mode & 0o222 != 0 {
        return None;
    }
    Some(format!("{mode:04o}"))
}

fn is_safe_mount_part(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@' | b'+' | b'=' | b'%')
        })
}

fn is_valid_mapping_source(value: &str) -> bool {
    !value.is_empty() && !value.bytes().any(|byte| matches!(byte, b'\0' | b'\n' | b'\r'))
}

fn is_valid_quadlet_bind_source(value: &str) -> bool {
    matches!(
        classify_path(value),
        PathForm::AbsoluteLiteral | PathForm::SystemdSpecifier
    ) && is_safe_mount_part(value)
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
