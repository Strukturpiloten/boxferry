//! Neutral-model-to-Quadlet planning.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::{self, Write as _},
};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, RuleId, Severity, TargetProfile,
};
use boxferry_model::{
    Annotation, Application, ArtifactDependency, ArtifactDependencyNode, BuildSettingValues, BuildSourceDeclaration,
    Command, Config, Device, Entrypoint, EnvironmentFile, EnvironmentFileFormat, EnvironmentValue, ExposedPort,
    GroupExitPolicy, Healthcheck, HealthcheckCommand, HostAddressKind, HostMapping, ImageAcquisition,
    ImageAcquisitionSetting, ImageArtifactAssignment, ImageBuild, ImageBuildSetting, MetadataLabel, Mount, MountSource,
    NetworkAttachment, NetworkDriverOption, Port, ProtectedString, Protocol, Provenance, PullPolicy, ReloadAction,
    ResourceGrant, ResourceOwnership, RestartPolicy, Secret, SecurityOption, SelinuxRelabel, Service,
    ServiceDependency, ServiceDependencyCondition, ServiceGroupRuntime, SourceBuildSetting, Sourced,
    StartupNotification, VolumeImageSource,
};
use quadlet_lens::{
    capability::{CapabilityCatalogue, CatalogueError, PodmanTarget, PodmanVersion, SupportClassification},
    model::{BuildKey, ContainerKey, ImageKey, NetworkKey, PodKey, QuadletUnitType, VolumeKey},
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
    pod_name: Option<String>,
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
                invalid_target: RuleId::QuadletTargetInvalid.definition().diagnostic_code()?,
                assumed_future: RuleId::QuadletOpenEndedTarget.definition().diagnostic_code()?,
                unsupported: RuleId::QuadletOutputUnsupported.definition().diagnostic_code()?,
                invalid_value: RuleId::QuadletValueInvalid.definition().diagnostic_code()?,
                generation: RuleId::QuadletGenerationFailed.definition().diagnostic_code()?,
                capability_unavailable: RuleId::QuadletCapabilityUnavailable.definition().diagnostic_code()?,
                grouping_approximation: RuleId::QuadletGroupingApproximation.definition().diagnostic_code()?,
                dependency_unsupported: RuleId::QuadletDependencyUnsupported.definition().diagnostic_code()?,
                restart: RuleId::QuadletRestartApproximation.definition().diagnostic_code()?,
                environment_file: RuleId::QuadletEnvironmentFileApproximation
                    .definition()
                    .diagnostic_code()?,
                grouping_invalid: RuleId::QuadletGroupingInvalid.definition().diagnostic_code()?,
                dependency_invalid: RuleId::QuadletDependencyInvalid.definition().diagnostic_code()?,
                capability_deprecated: RuleId::QuadletCapabilityDeprecated.definition().diagnostic_code()?,
            },
            relative_bind_root: None,
            bind_source_mappings: BTreeMap::new(),
            grouping_policy: QuadletGroupingPolicy::SeparateContainers,
            pod_name: None,
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

    /// Selects the native name for a caller-requested single Podman pod.
    ///
    /// The override applies only to [`QuadletGroupingPolicy::SinglePod`]. Callers selecting
    /// another grouping policy retain its established naming behavior.
    #[must_use]
    pub fn with_pod_name(mut self, name: impl Into<String>) -> Self {
        self.pod_name = Some(name.into());
        self
    }

    /// Returns the caller-selected native pod name, when one was supplied.
    #[must_use]
    pub fn pod_name(&self) -> Option<&str> {
        self.pod_name.as_deref()
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
    capability_unavailable: DiagnosticCode,
    grouping_approximation: DiagnosticCode,
    dependency_unsupported: DiagnosticCode,
    restart: DiagnosticCode,
    environment_file: DiagnosticCode,
    grouping_invalid: DiagnosticCode,
    dependency_invalid: DiagnosticCode,
    capability_deprecated: DiagnosticCode,
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
    runtime: Option<Sourced<ServiceGroupRuntime>>,
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
        let artifact_dependencies = self.artifact_dependencies();
        if let Err(error) = self
            .application
            .validate_image_artifact_dependencies(&artifact_dependencies)
        {
            self.invalid(
                self.exporter.codes.dependency_invalid.clone(),
                "application.artifact_dependencies",
                "image artifact references must form a complete acyclic graph",
                &error.to_string(),
                &[],
            );
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
                    name: self
                        .exporter
                        .pod_name
                        .clone()
                        .unwrap_or_else(|| self.application.name().as_str().to_owned()),
                    subject: "application.pod".to_owned(),
                    origins: service_origins(self.application.services()),
                    consumed_group: None,
                    approximation: "caller-selected single-pod grouping shares one network namespace across source services",
                    runtime: None,
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
        for acquisition in self.application.image_acquisitions() {
            self.map_image_acquisition(acquisition);
        }
        for build in self.application.image_builds() {
            self.map_image_build(build);
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

    fn artifact_dependencies(&self) -> Vec<Sourced<ArtifactDependency>> {
        let mut dependencies = Vec::new();
        for volume in self.application.volumes() {
            let source = ArtifactDependencyNode::Volume(volume.value().name().clone());
            let Some(image) = volume.value().image_source() else {
                continue;
            };
            let target = match image.value() {
                VolumeImageSource::ImageAcquisition(name) => ArtifactDependencyNode::ImageAcquisition(name.clone()),
                VolumeImageSource::ImageBuild(name) => ArtifactDependencyNode::ImageBuild(name.clone()),
                VolumeImageSource::Literal(_) | _ => continue,
            };
            let Some(origin) = image.origins().first() else {
                continue;
            };
            dependencies.push(Sourced::from_source(
                ArtifactDependency::new(
                    Sourced::from_source(source, origin.clone()),
                    Sourced::from_source(target, origin.clone()),
                ),
                origin.clone(),
            ));
        }
        dependencies
    }

    fn preserve_single_group_plan(&mut self) -> Option<PodPlan> {
        let groups = self.application.service_groups();
        if groups.len() != 1 {
            let origins = groups
                .iter()
                .flat_map(|group| group.origins().iter().cloned())
                .collect::<Vec<_>>();
            self.invalid(
                self.exporter.codes.grouping_invalid.clone(),
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
                self.exporter.codes.grouping_invalid.clone(),
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
                self.exporter.codes.grouping_invalid.clone(),
                "application.grouping",
                "preserved Quadlet pod membership must cover the complete application",
                "the single service group does not contain every application service exactly once",
                &origins,
            );
            return None;
        }
        // Native pod settings are authoritative.  Do not combine them with values inferred from
        // the member containers: those values can intentionally disagree in a converted graph.
        if group.value().runtime().is_none() && !self.validate_single_pod_grouping() {
            return None;
        }

        Some(PodPlan {
            name: group.value().name().as_str().to_owned(),
            subject: format!("service_groups.{}", group.value().name().as_str()),
            origins: service_group_origins(group, self.application.services()),
            consumed_group: Some(group.value().name().as_str().to_owned()),
            approximation: "caller-selected preservation maps structural membership to one shared Podman pod namespace",
            runtime: group.value().runtime().cloned(),
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
                            self.exporter.codes.dependency_invalid.clone(),
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
            self.exporter.codes.dependency_invalid.clone(),
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
                self.exporter.codes.grouping_invalid.clone(),
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
                    self.exporter.codes.grouping_invalid.clone(),
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
                    self.exporter.codes.grouping_invalid.clone(),
                    "application.grouping",
                    "single-pod grouping cannot preserve per-service network aliases",
                    "remove aliases or retain separate containers",
                    &origins,
                );
                return false;
            }
            if !same_host_mappings(service.value().host_mappings(), first.value().host_mappings()) {
                self.invalid(
                    self.exporter.codes.grouping_invalid.clone(),
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
                        self.exporter.codes.grouping_invalid.clone(),
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
                        self.exporter.codes.grouping_invalid.clone(),
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
                            self.exporter.codes.grouping_invalid.clone(),
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
                self.exporter.codes.grouping_invalid.clone(),
                "application.grouping",
                "single-pod grouping cannot preserve different per-service user namespaces",
                "all services must declare the same user namespace, or every service must leave it implicit",
                origins,
            );
            return false;
        }
        if first_user_namespace.is_some_and(|value| !is_safe_word(value, false)) {
            self.invalid(
                self.exporter.codes.grouping_invalid.clone(),
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
                if !self.capability("quadlet.unit-type.network", &subject, network.origins()) {
                    return;
                }
                let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Network);
                let runtime_name_subject = format!("{subject}.runtime_name");
                let (runtime_name, runtime_name_origins) = network.value().runtime_name().map_or_else(
                    || (network.value().name().as_str(), network.origins()),
                    |name| (name.value().expose(), name.origins()),
                );
                if !is_safe_network_scalar(runtime_name, false) {
                    self.unsupported(
                        &runtime_name_subject,
                        "NetworkName must be an unquoted systemd-safe network name",
                        runtime_name_origins,
                    );
                    return;
                }
                if self.capability("quadlet.network.name", &runtime_name_subject, runtime_name_origins)
                    && self.push_network(
                        &mut builder,
                        NetworkKey::NetworkName,
                        runtime_name,
                        &runtime_name_subject,
                        runtime_name_origins,
                    )
                {
                    self.exact(&runtime_name_subject, runtime_name_origins);
                    self.map_network_settings(&subject, network.value(), &mut builder);
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

    #[expect(
        clippy::too_many_lines,
        reason = "the ten related typed network keys share one generated document and ordered IPAM contract"
    )]
    fn map_network_settings(
        &mut self,
        network_subject: &str,
        network: &boxferry_model::Network,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if let Some(driver) = network.driver() {
            self.map_raw_network_value(
                &format!("{network_subject}.driver"),
                driver,
                "quadlet.network.driver",
                NetworkKey::Driver,
                builder,
                "network driver must be an unquoted systemd-safe scalar",
            );
        }
        if let Some(options) = network.driver_options() {
            if options.is_empty() {
                self.unsupported(
                    &format!("{network_subject}.driver_options"),
                    "an explicit empty network option collection has no safe Quadlet reset encoding",
                    network.driver_options_origins(),
                );
            }
            for (index, option) in options.iter().enumerate() {
                let subject = format!("{network_subject}.driver_options[{index}]");
                let origins = network_option_origins(network.driver_options_origins(), option);
                let name = option.value().name().value().as_str();
                let value = option.value().value().value().expose();
                if is_safe_network_assignment(name, value) {
                    self.emit_network(
                        builder,
                        NetworkKey::Options,
                        "quadlet.network.options",
                        &format!("{name}={value}"),
                        &subject,
                        &origins,
                    );
                } else {
                    self.unsupported(
                        &subject,
                        "network option must use an unambiguous systemd-safe NAME=VALUE spelling",
                        &origins,
                    );
                }
            }
        }
        if let Some(labels) = network.labels() {
            if labels.is_empty() {
                self.unsupported(
                    &format!("{network_subject}.labels"),
                    "an explicit empty network label collection has no safe Quadlet reset encoding",
                    network.labels_origins(),
                );
            }
            for (index, label) in labels.iter().enumerate() {
                let subject = format!("{network_subject}.labels[{index}]");
                let origins = network_label_origins(network.labels_origins(), label);
                let name = label.value().name().as_str();
                let value = label.value().value().expose();
                if is_label_name(name) && is_safe_network_scalar(value, true) {
                    self.emit_network(
                        builder,
                        NetworkKey::Label,
                        "quadlet.network.label",
                        &format!("{name}={value}"),
                        &subject,
                        &origins,
                    );
                } else {
                    self.unsupported(
                        &subject,
                        "network label must use an unambiguous systemd-safe NAME=VALUE spelling",
                        &origins,
                    );
                }
            }
        }
        for (field, value, key, capability) in [
            (
                "internal",
                network.internal(),
                NetworkKey::Internal,
                "quadlet.network.internal",
            ),
            ("ipv6", network.ipv6(), NetworkKey::IPv6, "quadlet.network.ipv6"),
        ] {
            if let Some(value) = value {
                let subject = format!("{network_subject}.{field}");
                self.emit_network(
                    builder,
                    key,
                    capability,
                    if *value.value() { "true" } else { "false" },
                    &subject,
                    value.origins(),
                );
            }
        }
        if let Some(driver) = network.ipam_driver() {
            self.map_raw_network_value(
                &format!("{network_subject}.ipam_driver"),
                driver,
                "quadlet.network.ipam-driver",
                NetworkKey::IPAMDriver,
                builder,
                "IPAM driver must be an unquoted systemd-safe scalar",
            );
        }
        if let Some(rows) = network.ipam_configs() {
            if rows.is_empty() {
                self.unsupported(
                    &format!("{network_subject}.ipam_configs"),
                    "an explicit empty IPAM collection has no safe Quadlet reset encoding",
                    network.ipam_configs_origins(),
                );
            }
            for (index, row) in rows.iter().enumerate() {
                let subject = format!("{network_subject}.ipam_configs[{index}]");
                let origins = network_ipam_origins(network.ipam_configs_origins(), row);
                self.emit_network_ipam_value(
                    builder,
                    NetworkKey::Subnet,
                    "quadlet.network.subnet",
                    row.value().subnet(),
                    &format!("{subject}.subnet"),
                    &origins,
                );
                if let Some(gateway) = row.value().gateway() {
                    self.emit_network_ipam_value(
                        builder,
                        NetworkKey::Gateway,
                        "quadlet.network.gateway",
                        gateway,
                        &format!("{subject}.gateway"),
                        &origins,
                    );
                }
                if let Some(range) = row.value().ip_range() {
                    self.emit_network_ipam_value(
                        builder,
                        NetworkKey::IPRange,
                        "quadlet.network.ip-range",
                        range,
                        &format!("{subject}.ip_range"),
                        &origins,
                    );
                }
            }
        }
    }

    fn map_raw_network_value(
        &mut self,
        subject: &str,
        value: &Sourced<ProtectedString>,
        capability: &str,
        key: NetworkKey,
        builder: &mut QuadletDocumentBuilder,
        unsupported: &str,
    ) {
        if is_safe_network_scalar(value.value().expose(), false) {
            self.emit_network(
                builder,
                key,
                capability,
                value.value().expose(),
                subject,
                value.origins(),
            );
        } else {
            self.unsupported(subject, unsupported, value.origins());
        }
    }

    fn emit_network_ipam_value(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: NetworkKey,
        capability: &str,
        value: &Sourced<ProtectedString>,
        subject: &str,
        row_origins: &[Provenance],
    ) {
        let origins = if value.origins().is_empty() {
            row_origins
        } else {
            value.origins()
        };
        if is_safe_network_scalar(value.value().expose(), false) {
            self.emit_network(builder, key, capability, value.value().expose(), subject, origins);
        } else {
            self.unsupported(subject, "IPAM values must be unquoted systemd-safe scalars", origins);
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
                let runtime_name = volume
                    .value()
                    .runtime_name()
                    .map_or_else(|| volume.value().name().as_str(), |name| name.value().expose());
                if self.push_volume(
                    &mut builder,
                    VolumeKey::VolumeName,
                    runtime_name,
                    &subject,
                    volume.origins(),
                ) {
                    self.map_volume_settings(&mut builder, volume, &subject);
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

    #[expect(
        clippy::too_many_lines,
        reason = "the typed volume keys share one generated document contract"
    )]
    fn map_volume_settings(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        volume: &Sourced<boxferry_model::Volume>,
        subject: &str,
    ) {
        let volume = volume.value();
        for (field, value, key, capability) in [
            ("driver", volume.driver(), VolumeKey::Driver, "quadlet.volume.driver"),
            ("device", volume.device(), VolumeKey::Device, "quadlet.volume.device"),
            ("type", volume.volume_type(), VolumeKey::Type, "quadlet.volume.type"),
            ("user", volume.user(), VolumeKey::User, "quadlet.volume.user"),
            ("group", volume.group(), VolumeKey::Group, "quadlet.volume.group"),
            (
                "service_name",
                volume.service_name(),
                VolumeKey::ServiceName,
                "quadlet.volume.service-name",
            ),
        ] {
            if let Some(value) = value {
                let item = format!("{subject}.{field}");
                if is_safe_network_scalar(value.value().expose(), false)
                    && self.capability(capability, &item, value.origins())
                    && self.push_volume(builder, key, value.value().expose(), &item, value.origins())
                {
                    self.exact(item, value.origins());
                } else {
                    self.unsupported(
                        &item,
                        "volume value requires an unquoted systemd-safe scalar",
                        value.origins(),
                    );
                }
            }
        }
        if let Some(value) = volume.options() {
            let item = format!("{subject}.options");
            if volume.device().is_none()
                && self
                    .podman_target
                    .is_some_and(|target| target.minimum() < PodmanVersion::new(6, 0, 0))
            {
                self.unsupported(
                    &item,
                    "Options without Device is only evidenced for target ranges beginning at Podman 6.0",
                    value.origins(),
                );
            } else if is_safe_network_scalar(value.value().expose(), false)
                && self.capability("quadlet.volume.options", &item, value.origins())
                && self.push_volume(
                    builder,
                    VolumeKey::Options,
                    value.value().expose(),
                    &item,
                    value.origins(),
                )
            {
                self.exact(item, value.origins());
            } else {
                self.unsupported(
                    &item,
                    "volume value requires an unquoted systemd-safe scalar",
                    value.origins(),
                );
            }
        }
        for (field, value, key, capability) in [
            ("uid", volume.uid(), VolumeKey::UID, "quadlet.volume.uid"),
            ("gid", volume.gid(), VolumeKey::GID, "quadlet.volume.gid"),
        ] {
            if let Some(value) = value {
                let item = format!("{subject}.{field}");
                if !value.value().expose().is_empty()
                    && value.value().expose().bytes().all(|b| b.is_ascii_digit())
                    && self.capability(capability, &item, value.origins())
                    && self.push_volume(builder, key, value.value().expose(), &item, value.origins())
                {
                    self.exact(item, value.origins());
                } else {
                    self.unsupported(
                        &item,
                        "only reviewed canonical numeric identity spellings can be emitted",
                        value.origins(),
                    );
                }
            }
        }
        if volume.volume_type().is_some() && volume.device().is_none() {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{subject}.type"),
                "Volume Type requires Device",
                "set Device with Type",
                &[],
            );
        }
        if let Some(copy) = volume.copy() {
            let item = format!("{subject}.copy");
            if self.capability("quadlet.volume.copy", &item, copy.origins())
                && self.push_volume(
                    builder,
                    VolumeKey::Copy,
                    if *copy.value() { "true" } else { "false" },
                    &item,
                    copy.origins(),
                )
            {
                self.exact(item, copy.origins());
            }
        }
        if let Some(labels) = volume.labels() {
            if labels.is_empty() {
                self.unsupported(
                    &format!("{subject}.labels"),
                    "an explicit empty volume label collection has no safe Quadlet reset encoding",
                    volume.labels_origins(),
                );
            }
            for (index, label) in labels.iter().enumerate() {
                let item = format!("{subject}.labels[{index}]");
                let name = label.value().name().as_str();
                let text = label.value().value().expose();
                if is_label_name(name)
                    && is_safe_network_scalar(text, true)
                    && self.capability("quadlet.volume.label", &item, label.origins())
                    && self.push_volume(
                        builder,
                        VolumeKey::Label,
                        format!("{name}={text}"),
                        &item,
                        label.origins(),
                    )
                {
                    self.exact(item, label.origins());
                } else {
                    self.unsupported(
                        &item,
                        "volume label must use a safe NAME=VALUE assignment",
                        label.origins(),
                    );
                }
            }
        }
        for (field, values, key, capability) in [
            (
                "containers_conf_modules",
                volume.containers_conf_modules(),
                VolumeKey::ContainersConfModule,
                "quadlet.volume.containers-conf-module",
            ),
            (
                "global_args",
                volume.global_args(),
                VolumeKey::GlobalArgs,
                "quadlet.volume.global-args",
            ),
            (
                "podman_args",
                volume.podman_args(),
                VolumeKey::PodmanArgs,
                "quadlet.volume.podman-args",
            ),
        ] {
            if let Some(values) = values {
                if values.is_empty() {
                    let origins = match field {
                        "containers_conf_modules" => volume.containers_conf_modules_origins(),
                        "global_args" => volume.global_args_origins(),
                        _ => volume.podman_args_origins(),
                    };
                    self.unsupported(
                        &format!("{subject}.{field}"),
                        "an explicit empty collection has no safe Quadlet reset encoding",
                        origins,
                    );
                }
                for (index, value) in values.iter().enumerate() {
                    let item = format!("{subject}.{field}[{index}]");
                    if is_safe_network_scalar(value.value().expose(), false)
                        && self.capability(capability, &item, value.origins())
                        && self.push_volume(builder, key, value.value().expose(), &item, value.origins())
                    {
                        self.exact(item, value.origins());
                    } else {
                        self.unsupported(
                            &item,
                            "protected raw volume arguments are retained but only safe physical forms can be regenerated",
                            value.origins(),
                        );
                    }
                }
            }
        }
        if let Some(image) = volume.image_source() {
            let item = format!("{subject}.image");
            let text = match image.value() {
                VolumeImageSource::Literal(value) => value.expose().to_owned(),
                VolumeImageSource::ImageAcquisition(name) => format!("{}.image", name.as_str()),
                VolumeImageSource::ImageBuild(name) => format!("{}.build", name.as_str()),
                _ => {
                    self.unsupported(&item, "unknown volume image source cannot be emitted", image.origins());
                    return;
                }
            };
            if volume.copy().is_some() {
                self.unsupported(
                    &item,
                    "Image ignores Copy; the conflict is retained rather than claimed exact",
                    image.origins(),
                );
            }
            if self.capability("quadlet.volume.image", &item, image.origins())
                && self.push_volume(builder, VolumeKey::Image, text, &item, image.origins())
            {
                self.exact(item, image.origins());
            }
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

    fn map_image_acquisition(&mut self, acquisition: &Sourced<ImageAcquisition>) {
        let name = acquisition.value().name().as_str();
        let subject = format!("image_acquisitions.{name}");
        let Some(file_name) = self.unit_file_name(&subject, name, "image", acquisition.origins()) else {
            return;
        };
        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Image);
        let mut has_image = false;
        for (index, setting) in acquisition.value().settings().unwrap_or_default().iter().enumerate() {
            let item = format!("{subject}.settings[{index}]");
            has_image |= self.emit_image_acquisition_setting(&mut builder, setting, &item);
        }
        if !has_image {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &subject,
                "Quadlet image generation requires Image",
                "neutral acquisition has no Image setting",
                acquisition.origins(),
            );
            return;
        }
        self.finish_document(file_name, &builder, &subject, acquisition.origins());
    }

    fn emit_image_acquisition_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageAcquisitionSetting>,
        subject: &str,
    ) -> bool {
        let has_image = matches!(setting.value(), ImageAcquisitionSetting::Image(value) if !value.expose().is_empty());
        match setting.value() {
            ImageAcquisitionSetting::Image(value) => {
                self.emit_image(
                    builder,
                    ImageKey::Image,
                    "quadlet.image.image",
                    value.expose(),
                    subject,
                    setting.origins(),
                );
            }
            ImageAcquisitionSetting::ImageTags(values) => {
                self.emit_image_values(builder, ImageKey::ImageTag, "quadlet.image.image-tag", values, subject);
            }
            ImageAcquisitionSetting::ContainersConfigModules(values) => self.emit_image_values(
                builder,
                ImageKey::ContainersConfModule,
                "quadlet.image.containers-conf-module",
                values,
                subject,
            ),
            ImageAcquisitionSetting::GlobalArguments(values) => self.emit_image_values(
                builder,
                ImageKey::GlobalArgs,
                "quadlet.image.global-args",
                values,
                subject,
            ),
            ImageAcquisitionSetting::ServiceName(value) => self.emit_image(
                builder,
                ImageKey::ServiceName,
                "quadlet.image.service-name",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::AllTags(value) => self.emit_image(
                builder,
                ImageKey::AllTags,
                "quadlet.image.all-tags",
                bool_text(*value),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::Architecture(value) => self.emit_image(
                builder,
                ImageKey::Arch,
                "quadlet.image.arch",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::AuthFile(value) => self.emit_image(
                builder,
                ImageKey::AuthFile,
                "quadlet.image.auth-file",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::CertificateDirectory(value) => self.emit_image(
                builder,
                ImageKey::CertDir,
                "quadlet.image.cert-dir",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::Credentials(value) => self.emit_image(
                builder,
                ImageKey::Creds,
                "quadlet.image.creds",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::DecryptionKey(value) => self.emit_image(
                builder,
                ImageKey::DecryptionKey,
                "quadlet.image.decryption-key",
                value.expose(),
                subject,
                setting.origins(),
            ),
            ImageAcquisitionSetting::OperatingSystem(value) => self.emit_image(
                builder,
                ImageKey::OS,
                "quadlet.image.os",
                value.expose(),
                subject,
                setting.origins(),
            ),
            _ => self.unsupported(subject, "unrecognized image-acquisition setting", setting.origins()),
        }
        has_image
    }

    fn map_image_build(&mut self, build: &Sourced<ImageBuild>) {
        let name = build.value().name().as_str();
        let subject = format!("image_builds.{name}");
        let Some(file_name) = self.unit_file_name(&subject, name, "build", build.origins()) else {
            return;
        };
        if !self.capability("quadlet.unit-type.build", &subject, build.origins()) {
            return;
        }
        self.report_build_source_declaration(build, &subject);
        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Build);
        let mut has_tag = false;
        let mut has_context = false;
        for (index, setting) in build.value().settings().unwrap_or_default().iter().enumerate() {
            let item = format!("{subject}.settings[{index}]");
            if self.map_image_build_required_setting(&mut builder, setting, &item, &mut has_tag, &mut has_context)
                || self.map_image_build_text_setting(&mut builder, setting, &item)
                || self.map_image_build_boolean_setting(&mut builder, setting, &item)
                || self.map_image_build_assignment_setting(&mut builder, setting, &item)
                || self.map_image_build_value_setting(&mut builder, setting, &item)
            {
                continue;
            }
            match setting.value() {
                ImageBuildSetting::RuntimeArguments(_) => self.unsupported(
                    &item,
                    "PodmanArgs is retained as native evidence and is never synthesized",
                    setting.origins(),
                ),
                _ => self.unsupported(&item, "unrecognized image-build setting", setting.origins()),
            }
        }
        if !has_tag || !has_context {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &subject,
                "Quadlet build generation requires ImageTag and File or SetWorkingDirectory",
                "neutral build is missing a required build tag or context",
                build.origins(),
            );
            return;
        }
        self.finish_document(file_name, &builder, &subject, build.origins());
    }

    fn map_image_build_required_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageBuildSetting>,
        item: &str,
        has_tag: &mut bool,
        has_context: &mut bool,
    ) -> bool {
        match setting.value() {
            ImageBuildSetting::ImageTags(values) => {
                *has_tag |= values.values().iter().any(|value| !value.value().expose().is_empty());
                self.emit_build_values(builder, BuildKey::ImageTag, "quadlet.build.image-tag", values, item);
            }
            ImageBuildSetting::SetWorkingDirectory(value) => {
                *has_context |= !value.expose().is_empty();
                self.emit_build(
                    builder,
                    BuildKey::SetWorkingDirectory,
                    "quadlet.build.set-working-directory",
                    value.expose(),
                    item,
                    setting.origins(),
                );
            }
            ImageBuildSetting::RecipeFile(value) => {
                *has_context |= !value.expose().is_empty();
                self.emit_build(
                    builder,
                    BuildKey::File,
                    "quadlet.build.file",
                    value.expose(),
                    item,
                    setting.origins(),
                );
            }
            _ => return false,
        }
        true
    }

    fn map_image_build_text_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageBuildSetting>,
        item: &str,
    ) -> bool {
        let (key, capability, value) = match setting.value() {
            ImageBuildSetting::Target(value) => (BuildKey::Target, "quadlet.build.target", value),
            ImageBuildSetting::Network(value) => (BuildKey::Network, "quadlet.build.network", value),
            ImageBuildSetting::Architecture(value) => (BuildKey::Arch, "quadlet.build.arch", value),
            ImageBuildSetting::Variant(value) => (BuildKey::Variant, "quadlet.build.variant", value),
            ImageBuildSetting::PullPolicy(value) => (BuildKey::Pull, "quadlet.build.pull", value),
            ImageBuildSetting::Retry(value) => (BuildKey::Retry, "quadlet.build.retry", value),
            ImageBuildSetting::RetryDelay(value) => (BuildKey::RetryDelay, "quadlet.build.retry-delay", value),
            ImageBuildSetting::AuthFile(value) => (BuildKey::AuthFile, "quadlet.build.auth-file", value),
            ImageBuildSetting::IgnoreFile(value) => (BuildKey::IgnoreFile, "quadlet.build.ignore-file", value),
            ImageBuildSetting::ServiceName(value) => (BuildKey::ServiceName, "quadlet.build.service-name", value),
            _ => return false,
        };
        self.emit_build(builder, key, capability, value.expose(), item, setting.origins());
        true
    }

    fn map_image_build_boolean_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageBuildSetting>,
        item: &str,
    ) -> bool {
        let (key, capability, value) = match setting.value() {
            ImageBuildSetting::TlsVerify(value) => (BuildKey::TLSVerify, "quadlet.build.tls-verify", value),
            ImageBuildSetting::ForceRemove(value) => (BuildKey::ForceRM, "quadlet.build.force-rm", value),
            _ => return false,
        };
        self.emit_build(builder, key, capability, bool_text(*value), item, setting.origins());
        true
    }

    fn map_image_build_assignment_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageBuildSetting>,
        item: &str,
    ) -> bool {
        let (key, capability, values) = match setting.value() {
            ImageBuildSetting::Labels(values) => (BuildKey::Label, "quadlet.build.label", values),
            ImageBuildSetting::BuildArguments(values) => (BuildKey::BuildArg, "quadlet.build.build-arg", values),
            ImageBuildSetting::Annotations(values) => (BuildKey::Annotation, "quadlet.build.annotation", values),
            ImageBuildSetting::Environment(values) => (BuildKey::Environment, "quadlet.build.environment", values),
            _ => return false,
        };
        self.emit_build_assignments(builder, key, capability, values, item);
        true
    }

    fn map_image_build_value_setting(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        setting: &Sourced<ImageBuildSetting>,
        item: &str,
    ) -> bool {
        let (key, capability, values) = match setting.value() {
            ImageBuildSetting::Secrets(values) => (BuildKey::Secret, "quadlet.build.secret", values),
            ImageBuildSetting::GroupAdd(values) => (BuildKey::GroupAdd, "quadlet.build.group-add", values),
            ImageBuildSetting::DnsServers(values) => (BuildKey::DNS, "quadlet.build.dns", values),
            ImageBuildSetting::DnsOptions(values) => (BuildKey::DNSOption, "quadlet.build.dns-option", values),
            ImageBuildSetting::DnsSearchDomains(values) => (BuildKey::DNSSearch, "quadlet.build.dns-search", values),
            ImageBuildSetting::ContainersConfigModules(values) => (
                BuildKey::ContainersConfModule,
                "quadlet.build.containers-conf-module",
                values,
            ),
            ImageBuildSetting::GlobalArguments(values) => (BuildKey::GlobalArgs, "quadlet.build.global-args", values),
            ImageBuildSetting::Volumes(values) => (BuildKey::Volume, "quadlet.build.volume", values),
            _ => return false,
        };
        self.emit_build_values(builder, key, capability, values, item);
        true
    }

    fn report_build_source_declaration(&mut self, build: &Sourced<ImageBuild>, subject: &str) {
        let Some(declaration) = build.value().source_declaration() else {
            return;
        };
        match declaration.value() {
            BuildSourceDeclaration::Scalar(_) => self.unsupported(
                &format!("{subject}.source_declaration"),
                "a scalar source build context is retained separately and is not silently converted into a Quadlet build context",
                declaration.origins(),
            ),
            BuildSourceDeclaration::Structured(settings) => {
                for (index, setting) in settings.iter().enumerate() {
                    let item = format!("{subject}.source_declaration.settings[{index}]");
                    let covered = match setting.value() {
                        SourceBuildSetting::RecipeFile(_) => build_has_setting(build.value(), |value| matches!(value, ImageBuildSetting::RecipeFile(_))),
                        SourceBuildSetting::Target(_) => build_has_setting(build.value(), |value| matches!(value, ImageBuildSetting::Target(_))),
                        SourceBuildSetting::Tags(_) => build_has_setting(build.value(), |value| matches!(value, ImageBuildSetting::ImageTags(_))),
                        SourceBuildSetting::Labels(_) => build_has_setting(build.value(), |value| matches!(value, ImageBuildSetting::Labels(_))),
                        SourceBuildSetting::Arguments(arguments) => {
                            build_has_setting(build.value(), |value| matches!(value, ImageBuildSetting::BuildArguments(_)))
                                && arguments.values().iter().all(|argument| argument.value().value().is_some())
                        }
                        _ => false,
                    };
                    if !covered {
                        self.unsupported(&item, "source build declaration is not represented by the emitted Quadlet build settings", setting.origins());
                    }
                }
            }
            _ => self.unsupported(&format!("{subject}.source_declaration"), "unrecognized source build declaration", declaration.origins()),
        }
    }

    fn map_pod(&mut self, plan: &PodPlan) {
        let subject = plan.subject.as_str();
        let name = plan.name.as_str();
        let origins = &plan.origins;
        let Some(file_name) = self.unit_file_name(subject, name, "pod", origins) else {
            return;
        };
        if !self.capability("quadlet.unit-type.pod", subject, origins) {
            return;
        }

        let mut builder = QuadletDocumentBuilder::new(QuadletUnitType::Pod);
        if let Some(runtime) = plan.runtime.as_ref() {
            if let Some(runtime_name) = runtime.value().runtime_name() {
                if !self.capability("quadlet.pod.name", subject, runtime_name.origins())
                    || !self.push_pod(
                        &mut builder,
                        PodKey::PodName,
                        runtime_name.value().expose(),
                        subject,
                        runtime_name.origins(),
                    )
                {
                    return;
                }
            }
            self.map_group_runtime(subject, runtime, &mut builder);
            self.exact(subject, origins);
            self.finish_document(file_name, &builder, subject, origins);
            return;
        }
        if !self.capability("quadlet.pod.name", subject, origins)
            || !self.push_pod(&mut builder, PodKey::PodName, name, subject, origins)
        {
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

    #[allow(clippy::too_many_lines)] // Each native pod key remains capability-gated at its mapping site.
    fn map_group_runtime(
        &mut self,
        group_subject: &str,
        runtime: &Sourced<ServiceGroupRuntime>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let value = runtime.value();
        if let Some(service_name) = value.service_name() {
            let subject = format!("{group_subject}.runtime.service_name");
            let name = service_name.value().expose();
            if name.ends_with(".service") || !is_native_atom(name) {
                self.unsupported(
                    &subject,
                    "pod ServiceName must be a safe unsuffixed service-name stem",
                    service_name.origins(),
                );
            } else if self.capability("quadlet.pod.service-name", &subject, service_name.origins())
                && self.push_pod(builder, PodKey::ServiceName, name, &subject, service_name.origins())
            {
                self.exact(subject, service_name.origins());
            }
        }
        if let Some(user_namespace) = value.user_namespace() {
            let subject = format!("{group_subject}.runtime.user_namespace");
            if is_safe_word(user_namespace.value().expose(), false)
                && self.capability("quadlet.pod.userns", &subject, user_namespace.origins())
                && self.push_pod(
                    builder,
                    PodKey::UserNS,
                    user_namespace.value().expose(),
                    &subject,
                    user_namespace.origins(),
                )
            {
                self.exact(subject, user_namespace.origins());
            } else {
                self.unsupported(
                    &subject,
                    "pod UserNS requires a safe one-line native token",
                    user_namespace.origins(),
                );
            }
        }
        if let Some(shm_size) = value.shm_size() {
            let subject = format!("{group_subject}.runtime.shm_size");
            if ShmSize::new(shm_size.value().expose()).is_ok()
                && self.capability("quadlet.pod.shm-size", &subject, shm_size.origins())
                && self.push_pod(
                    builder,
                    PodKey::ShmSize,
                    shm_size.value().expose(),
                    &subject,
                    shm_size.origins(),
                )
            {
                self.exact(subject, shm_size.origins());
            } else {
                self.unsupported(
                    &subject,
                    "pod ShmSize must use the reviewed native grammar",
                    shm_size.origins(),
                );
            }
        }
        if let Some(exit_policy) = value.exit_policy() {
            let subject = format!("{group_subject}.runtime.exit_policy");
            let output = match exit_policy.value() {
                GroupExitPolicy::Stop => Some("stop"),
                GroupExitPolicy::Continue => Some("continue"),
                _ => None,
            };
            if let Some(output) = output {
                if self.pod_capability(
                    "quadlet.pod.exit-policy",
                    PodmanVersion::new(5, 6, 0),
                    &subject,
                    exit_policy.origins(),
                ) && self.push_pod(builder, PodKey::ExitPolicy, output, &subject, exit_policy.origins())
                {
                    self.exact(subject, exit_policy.origins());
                }
            } else {
                self.unsupported(
                    &subject,
                    "raw pod ExitPolicy is retained as native evidence and is not synthesized",
                    exit_policy.origins(),
                );
            }
        }
        if let Some(timeout) = value.stop_timeout() {
            let subject = format!("{group_subject}.runtime.stop_timeout");
            if is_canonical_nonnegative_seconds(timeout.value().as_str())
                && self.pod_capability(
                    "quadlet.pod.stop-timeout",
                    PodmanVersion::new(5, 7, 0),
                    &subject,
                    timeout.origins(),
                )
                && self.push_pod(
                    builder,
                    PodKey::StopTimeout,
                    timeout.value().as_str(),
                    &subject,
                    timeout.origins(),
                )
            {
                self.exact(subject, timeout.origins());
            } else {
                self.unsupported(
                    &subject,
                    "pod StopTimeout requires canonical integral seconds",
                    timeout.origins(),
                );
            }
        }
        if let Some(host_mappings) = value.host_mappings() {
            if host_mappings.is_empty() {
                self.unsupported(
                    &format!("{group_subject}.runtime.host_mappings"),
                    "an explicit empty pod AddHost reset has no generated Quadlet representation",
                    value.host_mappings_origins(),
                );
            }
            for (index, mapping) in host_mappings.iter().enumerate() {
                self.map_pod_host_mapping(index, mapping.value(), mapping.origins(), builder);
            }
        }
        let host_network = value.networks().is_some_and(|networks| {
            networks
                .iter()
                .any(|network| network.value().network().as_str() == "host")
        });
        if host_network && value.ports().is_some_and(|ports| !ports.is_empty()) {
            let mut origins = value
                .ports()
                .map_or_else(Vec::new, |ports| collection_all_origins(ports, value.ports_origins()));
            if let Some(networks) = value.networks() {
                for origin in collection_all_origins(networks, value.networks_origins()) {
                    if !origins.contains(&origin) {
                        origins.push(origin);
                    }
                }
            }
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{group_subject}.runtime.ports"),
                "Network=host conflicts with PublishPort",
                "remove published ports or choose a non-host pod network",
                &origins,
            );
        }
        if let Some(ports) = value.ports() {
            if ports.is_empty() {
                self.unsupported(
                    &format!("{group_subject}.runtime.ports"),
                    "an explicit empty pod PublishPort reset has no generated Quadlet representation",
                    value.ports_origins(),
                );
            }
            for (index, port) in ports.iter().enumerate() {
                self.map_pod_port(group_subject, index, port, builder);
            }
        }
        if let Some(networks) = value.networks() {
            if networks.is_empty() {
                self.unsupported(
                    &format!("{group_subject}.runtime.networks"),
                    "an explicit empty pod Network reset has no generated Quadlet representation",
                    value.networks_origins(),
                );
            }
            for network in networks {
                self.map_pod_network_attachment(network, builder);
            }
        }
        if let Some(mounts) = value.mounts() {
            if mounts.is_empty() {
                self.unsupported(
                    &format!("{group_subject}.runtime.mounts"),
                    "an explicit empty pod Volume reset has no generated Quadlet representation",
                    value.mounts_origins(),
                );
            }
            for (index, mount) in mounts.iter().enumerate() {
                self.map_pod_mount(group_subject, index, mount, builder);
            }
        }
    }

    #[allow(clippy::too_many_lines)] // Image-or-rootfs selection and typed service mapping share one document builder.
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
        if let Some(rootfs) = service.value().rootfs() {
            let rootfs_subject = format!("{subject}.rootfs");
            if service.value().image().is_some()
                || service.value().image_acquisition().is_some()
                || service.value().image_build().is_some()
            {
                self.invalid(
                    self.exporter.codes.invalid_value.clone(),
                    &rootfs_subject,
                    "Rootfs conflicts with every image source",
                    "remove the image, image acquisition, or image build reference before emitting Rootfs",
                    rootfs.origins(),
                );
                return;
            }
            if !is_safe_absolute_container_path(rootfs.value().expose()) {
                self.unsupported(
                    &rootfs_subject,
                    "Rootfs requires a safe absolute literal path",
                    rootfs.origins(),
                );
                return;
            }
            if self.capability("quadlet.container.rootfs", &rootfs_subject, rootfs.origins())
                && self.push_container(
                    &mut builder,
                    ContainerKey::Rootfs,
                    rootfs.value().expose(),
                    &rootfs_subject,
                    rootfs.origins(),
                )
            {
                self.exact(rootfs_subject, rootfs.origins());
            }
            self.map_service_content(&subject, service, grouped, pod_name, &mut builder);
            self.finish_document(file_name, &builder, &subject, service.origins());
            return;
        }
        let direct_image = service.value().image();
        let acquisition = service.value().image_acquisition();
        let build = service.value().image_build();
        let valid_direct_build_pair = match (direct_image, build) {
            (Some(image), Some(build_reference)) if acquisition.is_none() => self
                .application
                .image_builds()
                .iter()
                .find(|candidate| candidate.value().name() == build_reference.value())
                .is_some_and(|candidate| build_has_tag(candidate.value(), image.value().as_str())),
            _ => false,
        };
        let image_sources =
            usize::from(direct_image.is_some()) + usize::from(acquisition.is_some()) + usize::from(build.is_some());
        if !(image_sources == 1 || valid_direct_build_pair) {
            self.invalid(
                self.exporter.codes.invalid_value.clone(),
                &format!("{subject}.image"),
                "Quadlet container generation requires exactly one image source",
                "a direct image may accompany only the image build that declares it as a nonempty ImageTag",
                service.origins(),
            );
            return;
        }
        let (value, origins) = if let Some(build) = build.filter(|_| valid_direct_build_pair) {
            (format!("{}.build", build.value().as_str()), build.origins())
        } else if let Some(image) = direct_image {
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
            (image.value().as_str().to_owned(), image.origins())
        } else if let Some(acquisition) = service.value().image_acquisition() {
            (format!("{}.image", acquisition.value().as_str()), acquisition.origins())
        } else if let Some(build) = service.value().image_build() {
            (format!("{}.build", build.value().as_str()), build.origins())
        } else {
            unreachable!("image source count is one")
        };
        if !self.capability("quadlet.container.image", &format!("{subject}.image"), origins)
            || !self.push_container(
                &mut builder,
                ContainerKey::Image,
                value,
                &format!("{subject}.image"),
                origins,
            )
        {
            return;
        }
        self.exact(format!("{subject}.image"), origins);

        self.map_service_content(&subject, service, grouped, pod_name, &mut builder);
        self.finish_document(file_name, &builder, &subject, service.origins());
    }

    fn map_service_content(
        &mut self,
        subject: &str,
        service: &Sourced<Service>,
        grouped: bool,
        pod_name: Option<&str>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        self.map_container_name(subject, service.value(), builder);
        if let Some(command) = service.value().command() {
            self.map_command(subject, command, builder);
        }
        self.map_execution_context(subject, service.value(), grouped, builder);
        self.map_released_container_settings(subject, service.value(), builder);
        self.map_extended_container_settings(subject, service.value(), grouped, builder);
        if let Some(notification) = service.value().startup_notification() {
            self.map_startup_notification(subject, notification, builder);
        }
        self.report_grouped_dns(subject, service.value(), grouped);
        if let Some(restart_policy) = service.value().restart_policy() {
            self.map_restart_policy(subject, restart_policy, builder);
        }
        if let Some(healthcheck) = service.value().healthcheck() {
            self.map_healthcheck(subject, healthcheck, builder);
        }
        if service.value().startup_notification().is_none() {
            self.map_healthy_readiness(service, builder);
        }
        for environment in service.value().environment() {
            self.map_environment(subject, environment, builder);
        }
        for (index, environment_file) in service.value().environment_files().iter().enumerate() {
            self.map_environment_file(subject, index, environment_file, builder);
        }
        self.map_metadata_labels(subject, service.value(), builder);
        for (index, grant) in service.value().config_grants().iter().enumerate() {
            self.unsupported(
                &format!("{subject}.configs[{index}]"),
                "Quadlet has no native Compose-compatible config grant; define an explicit bind or target-specific materialization policy",
                grant.origins(),
            );
        }
        for (index, grant) in service.value().secret_grants().iter().enumerate() {
            self.map_secret_grant(subject, index, grant, builder);
        }
        if !grouped {
            for (index, mapping) in service.value().host_mappings().iter().enumerate() {
                self.map_container_host_mapping(subject, index, mapping, builder);
            }
        }
        if !grouped {
            for (index, port) in service.value().ports().iter().enumerate() {
                self.map_port(subject, index, port, builder);
            }
        }
        for (index, mount) in service.value().mounts().iter().enumerate() {
            self.map_mount(subject, index, mount, builder);
        }
        self.map_pod_or_networks(subject, service, pod_name, builder);
        if let Some(arguments) = service.value().podman_args() {
            if arguments.is_empty() {
                let field = format!("{subject}.podman_args");
                if self.capability(
                    "quadlet.container.podman-args",
                    &field,
                    service.value().podman_args_origins(),
                ) && self.push_container(
                    builder,
                    ContainerKey::PodmanArgs,
                    "",
                    &field,
                    service.value().podman_args_origins(),
                ) {
                    self.exact(field, service.value().podman_args_origins());
                }
            }
            for (index, argument) in arguments.iter().enumerate() {
                let field = format!("{subject}.podman_args[{index}]");
                let raw = argument.value().expose();
                if raw.contains(['\0', '\r', '\n']) {
                    self.unsupported(
                        &field,
                        "authored PodmanArgs contains an unsafe physical-line control character",
                        argument.origins(),
                    );
                } else if self.capability("quadlet.container.podman-args", &field, argument.origins())
                    && self.push_container(builder, ContainerKey::PodmanArgs, raw, &field, argument.origins())
                {
                    self.exact(field, argument.origins());
                }
            }
        }
    }

    fn map_startup_notification(
        &mut self,
        service_subject: &str,
        notification: &Sourced<StartupNotification>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.startup_notification");
        match notification.value() {
            StartupNotification::Healthy => {
                if self.capability("quadlet.container.notify-healthy", &subject, notification.origins())
                    && self.push_container(
                        builder,
                        ContainerKey::Notify,
                        "healthy",
                        &subject,
                        notification.origins(),
                    )
                {
                    self.exact(subject, notification.origins());
                }
            }
            StartupNotification::Runtime | StartupNotification::Application => self.unsupported(
                &subject,
                "Notify=false and Notify=true require explicit capability evidence before they can be generated",
                notification.origins(),
            ),
            _ => self.unsupported(&subject, "unknown startup notification form", notification.origins()),
        }
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

    #[allow(clippy::too_many_lines)] // All new typed service keys are capability-gated together.
    fn map_extended_container_settings(
        &mut self,
        service_subject: &str,
        service: &Service,
        grouped: bool,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if let Some(entrypoint) = service.entrypoint() {
            let subject = format!("{service_subject}.entrypoint");
            match entrypoint.value() {
                Entrypoint::Exec(arguments)
                    if !arguments.is_empty() && arguments.iter().all(|arg| is_safe_word(arg.expose(), false)) =>
                {
                    let value = format!(
                        "[{}]",
                        arguments
                            .iter()
                            .map(|arg| format!("\"{}\"", arg.expose()))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    if self.capability("quadlet.container.entrypoint", &subject, entrypoint.origins())
                        && self.push_container(builder, ContainerKey::Entrypoint, value, &subject, entrypoint.origins())
                    {
                        self.exact(subject, entrypoint.origins());
                    }
                }
                _ => self.unsupported(
                    &subject,
                    "Entrypoint is exact only for a reviewed JSON exec array",
                    entrypoint.origins(),
                ),
            }
        }
        if let Some(run_init) = service.run_init() {
            let subject = format!("{service_subject}.run_init");
            if self.capability("quadlet.container.run-init", &subject, run_init.origins())
                && self.push_container(
                    builder,
                    ContainerKey::RunInit,
                    run_init.value().to_string(),
                    &subject,
                    run_init.origins(),
                )
            {
                self.approximate(
                    &subject,
                    "RunInit runtime equivalence remains reviewable",
                    run_init.origins(),
                );
            }
        }
        if let Some(timeout) = service.stop_timeout() {
            let subject = format!("{service_subject}.stop_timeout");
            if let Ok(seconds) = timeout.value().as_str().parse::<u64>() {
                if self.capability("quadlet.container.stop-timeout", &subject, timeout.origins())
                    && self.push_container(
                        builder,
                        ContainerKey::StopTimeout,
                        timeout.value().as_str(),
                        &subject,
                        timeout.origins(),
                    )
                {
                    if seconds == 0 {
                        self.approximate(
                            &subject,
                            "a zero stop timeout can differ from an omitted/default timeout and remains reviewable",
                            timeout.origins(),
                        );
                    } else {
                        self.exact(subject, timeout.origins());
                    }
                }
            } else {
                self.unsupported(
                    &subject,
                    "only nonnegative integral stop-timeout seconds are encodable; fractional native durations remain source-aware but require review",
                    timeout.origins(),
                );
            }
        }
        if let Some(policy) = service.pull_policy() {
            let subject = format!("{service_subject}.pull_policy");
            let value = match policy.value() {
                PullPolicy::Always => Some("always"),
                PullPolicy::Missing => Some("missing"),
                PullPolicy::Never => Some("never"),
                PullPolicy::Raw(raw) if raw.expose() == "newer" => Some("newer"),
                _ => None,
            };
            if let Some(value) = value {
                if self.capability("quadlet.container.pull", &subject, policy.origins())
                    && self.push_container(builder, ContainerKey::Pull, value, &subject, policy.origins())
                {
                    self.exact(subject, policy.origins());
                }
            } else {
                self.unsupported(
                    &subject,
                    "only reviewed native Pull values can be emitted; arbitrary raw policies remain unsupported",
                    policy.origins(),
                );
            }
        }
        if let Some(memory) = service.memory_limit() {
            let subject = format!("{service_subject}.memory_limit");
            if is_positive_memory(memory.value().expose()) {
                if self.capability("quadlet.container.memory", &subject, memory.origins())
                    && self.push_container(
                        builder,
                        ContainerKey::Memory,
                        memory.value().expose(),
                        &subject,
                        memory.origins(),
                    )
                {
                    self.exact(subject, memory.origins());
                }
            } else {
                self.unsupported(
                    &subject,
                    "memory must be a positive canonical byte quantity",
                    memory.origins(),
                );
            }
        }
        if let Some(ports) = service.exposed_ports() {
            if ports.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.exposed_ports"),
                    "an explicit empty exposed-port collection has no reviewed Quadlet reset encoding",
                    service.exposed_ports_origins(),
                );
            } else {
                for (index, port) in ports.iter().enumerate() {
                    self.map_exposed_port(service_subject, index, port, builder);
                }
            }
        }
        if let Some(annotations) = service.annotations() {
            if annotations.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.annotations"),
                    "an explicit empty annotation collection has no reviewed Quadlet reset encoding",
                    service.annotations_origins(),
                );
            } else {
                for annotation in annotations {
                    self.map_annotation(service_subject, annotation, builder);
                }
            }
        }
        if let Some(logging) = service.logging() {
            self.map_logging(service_subject, logging, builder);
        }
        if let Some(reload) = service.reload_action() {
            self.map_reload_action(service_subject, reload, builder);
        }
        self.map_network_addresses(service_subject, service, grouped, builder);
    }

    fn map_exposed_port(
        &mut self,
        service_subject: &str,
        index: usize,
        port: &Sourced<ExposedPort>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.exposed_ports[{index}]");
        let protocol = match port.value().protocol() {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
            _ => {
                self.unsupported(&subject, "ExposeHostPort supports only tcp or udp", port.origins());
                return;
            }
        };
        let value = if protocol == "tcp" {
            port.value().container().to_string()
        } else {
            format!("{}/udp", port.value().container())
        };
        if self.capability("quadlet.container.expose-host-port", &subject, port.origins())
            && self.push_container(builder, ContainerKey::ExposeHostPort, value, &subject, port.origins())
        {
            self.exact(subject, port.origins());
        }
    }

    fn map_annotation(
        &mut self,
        service_subject: &str,
        annotation: &Sourced<Annotation>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!(
            "{service_subject}.annotations.{}",
            annotation.value().name().value().as_str()
        );
        let Some(value) = encode_quadlet_label(
            annotation.value().name().value().as_str(),
            annotation.value().value().value().expose(),
        ) else {
            self.unsupported(
                &subject,
                "annotation contains NUL and cannot be represented",
                annotation.origins(),
            );
            return;
        };
        if self.capability("quadlet.container.annotation", &subject, annotation.origins())
            && self.push_container(builder, ContainerKey::Annotation, value, &subject, annotation.origins())
        {
            self.exact(subject, annotation.origins());
        }
    }

    fn map_logging(
        &mut self,
        service_subject: &str,
        logging: &Sourced<boxferry_model::Logging>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if let Some(driver) = logging.value().driver() {
            let subject = format!("{service_subject}.logging.driver");
            if !is_safe_word(driver.value().expose(), false) {
                self.unsupported(
                    &subject,
                    "LogDriver requires a non-empty systemd-safe token",
                    driver.origins(),
                );
            } else if self.capability("quadlet.container.log-driver", &subject, driver.origins())
                && self.push_container(
                    builder,
                    ContainerKey::LogDriver,
                    driver.value().expose(),
                    &subject,
                    driver.origins(),
                )
            {
                self.approximate(
                    &subject,
                    "provider logging remains a reviewable partial mapping",
                    driver.origins(),
                );
            }
        }
        if let Some(options) = logging.value().options() {
            if options.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.logging.options"),
                    "an explicit empty logging-option collection has no reviewed Quadlet reset encoding",
                    logging.value().options_origins(),
                );
            }
            for option in options {
                let subject = format!(
                    "{service_subject}.logging.options.{}",
                    option.value().name().value().as_str()
                );
                let Some(value) = encode_quadlet_label(
                    option.value().name().value().as_str(),
                    option.value().value().value().expose(),
                ) else {
                    self.unsupported(
                        &subject,
                        "LogOpt contains NUL and cannot be represented",
                        option.origins(),
                    );
                    continue;
                };
                if self.capability("quadlet.container.log-opt", &subject, option.origins())
                    && self.push_container(builder, ContainerKey::LogOpt, value, &subject, option.origins())
                {
                    self.approximate(
                        &subject,
                        "provider logging remains a reviewable partial mapping",
                        option.origins(),
                    );
                }
            }
        }
    }

    fn map_reload_action(
        &mut self,
        service_subject: &str,
        reload: &Sourced<ReloadAction>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{service_subject}.reload_action");
        match reload.value() {
            ReloadAction::Signal(signal) => {
                if is_safe_stop_signal(signal.expose())
                    && self.capability("quadlet.container.reload-signal", &subject, reload.origins())
                    && self.push_container(
                        builder,
                        ContainerKey::ReloadSignal,
                        signal.expose(),
                        &subject,
                        reload.origins(),
                    )
                {
                    self.exact(subject, reload.origins());
                } else {
                    self.unsupported(&subject, "reload signal is not safely encodable", reload.origins());
                }
            }
            ReloadAction::Command(Command::Exec(arguments))
                if !arguments.is_empty() && arguments.iter().all(|arg| is_safe_word(arg.expose(), false)) =>
            {
                let value = arguments
                    .iter()
                    .map(ProtectedString::expose)
                    .collect::<Vec<_>>()
                    .join(" ");
                if self.capability("quadlet.container.reload-cmd", &subject, reload.origins())
                    && self.push_container(builder, ContainerKey::ReloadCmd, value, &subject, reload.origins())
                {
                    self.exact(subject, reload.origins());
                }
            }
            _ => self.unsupported(
                &subject,
                "reload command requires unsupported systemd command-line encoding",
                reload.origins(),
            ),
        }
    }

    fn map_network_addresses(
        &mut self,
        service_subject: &str,
        service: &Service,
        grouped: bool,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let networks = service.networks();
        let has_values = networks.iter().any(|network| {
            network.value().ipv4_address().is_some()
                || network.value().ipv6_address().is_some()
                || !network.value().aliases().is_empty()
        });
        if !has_values {
            return;
        }
        if grouped {
            for network in networks {
                if network.value().ipv4_address().is_some()
                    || network.value().ipv6_address().is_some()
                    || !network.value().aliases().is_empty()
                {
                    self.unsupported(
                        &format!("{service_subject}.networks.{}", network.value().network().as_str()),
                        "IP, IP6, and NetworkAlias remain container-scoped and cannot be moved into a generated pod",
                        network.origins(),
                    );
                }
            }
            return;
        }
        if networks.len() != 1 {
            let origins = networks
                .iter()
                .flat_map(|network| {
                    network
                        .origins()
                        .iter()
                        .chain(network.value().ipv4_address().into_iter().flat_map(Sourced::origins))
                        .chain(network.value().ipv6_address().into_iter().flat_map(Sourced::origins))
                        .chain(network.value().alias_origins().iter().flatten())
                })
                .cloned()
                .collect::<Vec<_>>();
            self.unsupported(
                &format!("{service_subject}.networks"),
                "IP, IP6, and NetworkAlias require exactly one compatible network attachment",
                &origins,
            );
            return;
        }
        let network = networks[0].value();
        for (key, value, capability) in [
            (ContainerKey::IP, network.ipv4_address(), "quadlet.container.ip"),
            (ContainerKey::IP6, network.ipv6_address(), "quadlet.container.ip6"),
        ] {
            if let Some(value) = value {
                let subject = format!(
                    "{service_subject}.networks.{}",
                    if key == ContainerKey::IP {
                        "ipv4_address"
                    } else {
                        "ipv6_address"
                    }
                );
                if self.capability(capability, &subject, value.origins())
                    && self.push_container(builder, key, value.value().expose(), &subject, value.origins())
                {
                    self.exact(subject, value.origins());
                }
            }
        }
        for (index, alias) in network.aliases().iter().enumerate() {
            let subject = format!("{service_subject}.networks.aliases[{index}]");
            let origins = network.alias_origins().get(index).map_or(&[][..], Vec::as_slice);
            if self.capability("quadlet.container.network-alias", &subject, origins)
                && self.push_container(builder, ContainerKey::NetworkAlias, alias, &subject, origins)
            {
                self.exact(subject, origins);
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
        values: Option<&[Sourced<ProtectedString>]>,
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
                    is_safe_capability,
                    "capability names must use the reviewed uppercase Linux capability grammar",
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_raw_container_value(
        &mut self,
        subject: &str,
        value: &Sourced<ProtectedString>,
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
        value: &Sourced<ProtectedString>,
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
        value: &Sourced<ProtectedString>,
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
                self.exporter.codes.dependency_unsupported.clone(),
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
                    self.exporter.codes.dependency_unsupported.clone(),
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
                    self.exporter.codes.dependency_unsupported.clone(),
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
                    self.exporter.codes.dependency_unsupported.clone(),
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
                    self.exporter.codes.dependency_unsupported.clone(),
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
                self.exporter.codes.dependency_unsupported.clone(),
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
                        .map(ProtectedString::expose)
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
                values.extend(arguments.iter().map(ProtectedString::expose));
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
    }

    fn map_pod_mount(
        &mut self,
        group_subject: &str,
        index: usize,
        mount: &Sourced<Mount>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let subject = format!("{group_subject}.runtime.mounts[{index}]");
        let Some(encoded) = self.encode_pod_mount(&subject, mount) else {
            return;
        };
        if self.capability("quadlet.pod.volume", &subject, mount.origins())
            && self.push_pod(builder, PodKey::Volume, encoded, &subject, mount.origins())
        {
            self.exact(subject, mount.origins());
        }
    }

    fn encode_pod_mount(&mut self, subject: &str, mount: &Sourced<Mount>) -> Option<String> {
        if !is_safe_mount_part(mount.value().target()) || !mount.value().target().starts_with('/') {
            self.unsupported(
                subject,
                "pod mount target is not a safely encoded absolute path",
                mount.origins(),
            );
            return None;
        }
        let source = match mount.value().source() {
            MountSource::Volume(name) => self.volume_source(subject, name.as_str(), mount.origins()),
            MountSource::HostPath(path) => {
                if let Some(mapped) = self.exporter.bind_source_mappings.get(path) {
                    Some(mapped.clone())
                } else if matches!(
                    classify_path(path),
                    PathForm::AbsoluteLiteral | PathForm::SystemdSpecifier
                ) && is_safe_mount_part(path)
                {
                    Some(path.clone())
                } else {
                    self.unsupported(
                        subject,
                        "pod bind source needs an explicit safe source-to-target mapping",
                        mount.origins(),
                    );
                    None
                }
            }
            MountSource::Anonymous => Some(String::new()),
            _ => {
                self.unsupported(subject, "unknown pod mount source", mount.origins());
                None
            }
        }?;
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
                self.unsupported(subject, "unknown pod mount SELinux relabel mode", mount.origins());
                return None;
            }
        }
        if !options.is_empty() {
            encoded.push(':');
            encoded.push_str(&options.join(","));
        }
        Some(encoded)
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
        if name == "host" {
            return Some(name.to_owned());
        }
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
                        self.exporter.codes.capability_deprecated.clone(),
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
                    self.exporter.codes.capability_unavailable.clone(),
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

    /// Uses the Lens catalogue when available and retains the reviewed local floor for newly
    /// released Pod keys until the pinned Lens catalogue carries the corresponding evidence.
    fn pod_capability(
        &mut self,
        capability: &str,
        minimum: PodmanVersion,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        let Some(target) = self.podman_target else {
            return false;
        };
        if self.exporter.catalogue.capability(capability).is_some()
            && matches!(
                self.exporter.catalogue.evaluate(capability, target).classification(),
                SupportClassification::Native | SupportClassification::Deprecated
            )
        {
            return self.capability(capability, subject, origins);
        }
        if target.minimum() >= minimum {
            return true;
        }
        self.loss(
            self.exporter.codes.capability_unavailable.clone(),
            subject,
            ConversionKind::Unsupported,
            "required Quadlet capability does not cover the complete target range",
            &format!("{capability}: reviewed minimum is {minimum}"),
            origins,
        );
        false
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

    fn push_image(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: ImageKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_image(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn push_build(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: BuildKey,
        value: impl Into<String>,
        subject: &str,
        origins: &[Provenance],
    ) -> bool {
        match EntryValue::new(value).and_then(|value| builder.push_build(key, value)) {
            Ok(()) => true,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                false
            }
        }
    }

    fn emit_image(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: ImageKey,
        capability: &str,
        value: &str,
        subject: &str,
        origins: &[Provenance],
    ) {
        if self.capability(capability, subject, origins) && self.push_image(builder, key, value, subject, origins) {
            self.exact(subject, origins);
        }
    }

    fn emit_image_values(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: ImageKey,
        capability: &str,
        values: &BuildSettingValues<ProtectedString>,
        subject: &str,
    ) {
        if values.values().is_empty() {
            self.unsupported(
                subject,
                "an explicitly empty repeated Image setting has no physical Quadlet entry to emit",
                &[],
            );
            return;
        }
        for (index, value) in values.values().iter().enumerate() {
            self.emit_image(
                builder,
                key,
                capability,
                value.value().expose(),
                &format!("{subject}[{index}]"),
                value.origins(),
            );
        }
    }

    fn emit_build(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: BuildKey,
        capability: &str,
        value: &str,
        subject: &str,
        origins: &[Provenance],
    ) {
        if self.capability(capability, subject, origins) && self.push_build(builder, key, value, subject, origins) {
            self.exact(subject, origins);
        }
    }

    fn emit_build_values(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: BuildKey,
        capability: &str,
        values: &BuildSettingValues<ProtectedString>,
        subject: &str,
    ) {
        if values.values().is_empty() {
            self.unsupported(
                subject,
                "an explicitly empty repeated Build setting has no physical Quadlet entry to emit",
                &[],
            );
            return;
        }
        for (index, value) in values.values().iter().enumerate() {
            self.emit_build(
                builder,
                key,
                capability,
                value.value().expose(),
                &format!("{subject}[{index}]"),
                value.origins(),
            );
        }
    }

    fn emit_build_assignments(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: BuildKey,
        capability: &str,
        values: &BuildSettingValues<ImageArtifactAssignment>,
        subject: &str,
    ) {
        if values.values().is_empty() {
            self.unsupported(
                subject,
                "an explicitly empty repeated Build assignment setting has no physical Quadlet entry to emit",
                &[],
            );
            return;
        }
        for (index, assignment) in values.values().iter().enumerate() {
            let value = assignment.value().value().map_or_else(
                || assignment.value().name().expose().to_owned(),
                |value| format!("{}={}", assignment.value().name().expose(), value.expose()),
            );
            self.emit_build(
                builder,
                key,
                capability,
                &value,
                &format!("{subject}[{index}]"),
                assignment.origins(),
            );
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

    fn emit_network(
        &mut self,
        builder: &mut QuadletDocumentBuilder,
        key: NetworkKey,
        capability: &str,
        value: &str,
        subject: &str,
        origins: &[Provenance],
    ) {
        if self.capability(capability, subject, origins) && self.push_network(builder, key, value, subject, origins) {
            self.exact(subject, origins);
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
            self.exporter.codes.grouping_approximation.clone(),
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

fn network_option_origins(collection_origins: &[Provenance], option: &Sourced<NetworkDriverOption>) -> Vec<Provenance> {
    let mut origins = collection_item_origins(collection_origins, option);
    extend_origins(&mut origins, option.value().name().origins());
    extend_origins(&mut origins, option.value().value().origins());
    origins
}

fn network_label_origins(collection_origins: &[Provenance], label: &Sourced<MetadataLabel>) -> Vec<Provenance> {
    collection_item_origins(collection_origins, label)
}

fn network_ipam_origins(
    collection_origins: &[Provenance],
    row: &Sourced<boxferry_model::NetworkIpamConfig>,
) -> Vec<Provenance> {
    let mut origins = collection_item_origins(collection_origins, row);
    extend_origins(&mut origins, row.value().subnet().origins());
    if let Some(gateway) = row.value().gateway() {
        extend_origins(&mut origins, gateway.origins());
    }
    if let Some(range) = row.value().ip_range() {
        extend_origins(&mut origins, range.origins());
    }
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

fn is_safe_network_scalar(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.trim() == value
        && !value.bytes().any(|byte| {
            matches!(byte, b'\0' | b'\n' | b'\r' | b'%' | b'\'' | b'"' | b'\\') || byte.is_ascii_whitespace()
        })
}

fn is_safe_network_assignment(name: &str, value: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && is_safe_network_scalar(name, false)
        && is_safe_network_scalar(value, true)
}

fn is_label_name(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn is_positive_memory(value: &str) -> bool {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (amount, suffix) = value.split_at(split);
    !amount.is_empty()
        && !amount.starts_with('0')
        && amount.parse::<u64>().is_ok_and(|amount| amount > 0)
        && matches!(suffix, "" | "b" | "k" | "m" | "g")
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
    let (target, options) = value
        .split_once(':')
        .map_or((value, None), |(target, options)| (target, Some(options)));
    is_safe_absolute_container_path(target)
        && options.is_none_or(|options| {
            !options.is_empty()
                && options
                    .split(',')
                    .all(|option| is_safe_word(option, false) && !option.contains([':', '%']))
        })
}

fn is_safe_sysctl(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains('=') && is_safe_word(name, false) && is_safe_word(value, true)
}

fn is_safe_limit_value(value: &str) -> bool {
    value == "-1" || value == "0" || (!value.starts_with('0') && value.parse::<u64>().is_ok_and(|value| value > 0))
}

fn is_safe_ulimit(name: &str, soft: &str, hard: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| byte.is_ascii_lowercase())
        && is_safe_limit_value(soft)
        && is_safe_limit_value(hard)
}

fn is_safe_stop_signal(value: &str) -> bool {
    value
        .parse::<u8>()
        .is_ok_and(|number| (1..=64).contains(&number) && (value == "0" || !value.starts_with('0')))
        || value.strip_prefix("SIG").is_some_and(|name| {
            name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
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
    !value.is_empty()
        && value.bytes().all(|byte| matches!(byte, b'r' | b'w' | b'm'))
        && value.bytes().collect::<BTreeSet<_>>().len() == value.len()
}

fn is_safe_capability(value: &str) -> bool {
    let name = value.strip_prefix("CAP_").unwrap_or(value);
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_safe_absolute_container_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.contains('%')
        && value.split('/').all(|part| part != "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@' | b'+'))
}

fn is_canonical_nonnegative_seconds(value: &str) -> bool {
    value == "0" || (!value.is_empty() && !value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit()))
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

const fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn build_has_tag(build: &ImageBuild, image: &str) -> bool {
    build
        .settings()
        .unwrap_or_default()
        .iter()
        .any(|setting| match setting.value() {
            ImageBuildSetting::ImageTags(tags) => tags
                .values()
                .iter()
                .any(|tag| !tag.value().expose().is_empty() && tag.value().expose() == image),
            _ => false,
        })
}

fn build_has_setting(build: &ImageBuild, matches: impl Fn(&ImageBuildSetting) -> bool) -> bool {
    build
        .settings()
        .unwrap_or_default()
        .iter()
        .any(|setting| matches(setting.value()))
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
