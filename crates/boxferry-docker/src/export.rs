//! Reviewed standalone Docker target mapping and inert request output.
//!
//! Unresolved neutral image parameters cannot become native create intent. Resource-local
//! unsupported intent may leave an unrelated, unapproved partial preview, but a service
//! referencing an omitted resource blocks the candidate under every loss policy.
//! Bind-only `SELinux` relabel intent uses typed `DockerLens` mounts. Native planning
//! must independently admit its capability; mapping does not establish host effects.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;
use std::num::{NonZeroU16, NonZeroU32, NonZeroU64};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, PlatformVersion, RuleId, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, Entrypoint, EnvironmentValue, HealthcheckCommand, MountSource, Protocol, Provenance,
    PullPolicy, ResourceOwnership, RestartPolicy as NeutralRestartPolicy, SelinuxRelabel, Service,
    ServiceDependencyCondition, Sourced,
};
use docker_lens::observation::ResourceRef;
use docker_lens::target::{
    Argument, BindRelabel, ContainerHostname, ContainerIntent, ContainerLabel, ContainerSettings, ContainerToken,
    ContainerUser, DockerApiRenderer, DockerPlanner, EnvironmentAssignment, HealthTest, Healthcheck, HostBinding,
    ImageCommand, ImageReference, IntentError, MemoryLimit, Mount, NetworkAddress, NetworkAlias,
    NetworkAttachmentIntent, NetworkCreate, NetworkDriver, NetworkIntent, NetworkRole, NetworkSource, PidsLimit,
    Planner, PlanningError, PortHostIp, PortHostPort, PortPublication, Renderer, RestartPolicy,
    SecurityOption as NativeSecurityOption, TargetField, TargetIdentity, TargetIntent, TargetResource, VolumeLabel,
    WorkingDirectory,
};
use docker_lens::version::{Capability, EngineBuild, NativeEvidenceReference, TargetCapabilityCatalog};

/// The `BoxFerry` implementation identifier for the inert Docker target.
pub const DOCKER_TARGET: &str = "docker";

/// Independent authorization to put literal environment values in an output artifact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DockerEnvironmentPolicy {
    /// Do not emit any environment assignment; report each withheld field as a loss.
    #[default]
    Withhold,
    /// Explicitly permit literal values in the returned inert request bytes.
    IncludeLiteral,
}

/// Inert DockerLens-rendered requests bound to one exact reviewed native profile.
pub struct DockerOutput {
    profile: docker_lens::version::TargetProfile,
    evidence: NativeEvidenceReference,
    artifact: docker_lens::target::RenderedArtifact,
    dependency_decisions: Vec<DockerDependencyDecision>,
}

/// A BoxFerry-owned start/readiness decision, not an Engine create field or executor.
pub struct DockerDependencyDecision {
    service: String,
    dependency: String,
    service_identity: TargetIdentity,
    dependency_identity: TargetIdentity,
    condition: Option<ServiceDependencyCondition>,
    required: Option<bool>,
    restart: Option<bool>,
    reference_origins: Vec<Provenance>,
    condition_origins: Vec<Provenance>,
    required_origins: Vec<Provenance>,
    restart_origins: Vec<Provenance>,
    sensitive_identity: bool,
}

impl fmt::Debug for DockerDependencyDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DockerDependencyDecision([redacted identities and provenance])")
    }
}

impl DockerDependencyDecision {
    /// Neutral dependent service key.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }
    /// Neutral prerequisite service key.
    #[must_use]
    pub fn dependency(&self) -> &str {
        &self.dependency
    }
    /// Exact target container identity for the dependent; may be protected.
    #[must_use]
    pub fn service_identity(&self) -> &[u8] {
        self.service_identity.bytes()
    }
    /// Exact target container identity for the prerequisite; may be protected.
    #[must_use]
    pub fn dependency_identity(&self) -> &[u8] {
        self.dependency_identity.bytes()
    }
    /// Authored readiness condition, or `None` for the started default.
    #[must_use]
    pub const fn condition(&self) -> Option<&ServiceDependencyCondition> {
        self.condition.as_ref()
    }
    /// Authored required flag, or `None` for the required default.
    #[must_use]
    pub const fn required(&self) -> Option<bool> {
        self.required
    }
    /// Authored restart-propagation flag, or `None` for the disabled default.
    #[must_use]
    pub const fn restart(&self) -> Option<bool> {
        self.restart
    }
    /// Provenance of the dependency reference.
    #[must_use]
    pub fn reference_origins(&self) -> &[Provenance] {
        &self.reference_origins
    }
    /// Provenance of the readiness condition.
    #[must_use]
    pub fn condition_origins(&self) -> &[Provenance] {
        &self.condition_origins
    }
    /// Provenance of the required flag.
    #[must_use]
    pub fn required_origins(&self) -> &[Provenance] {
        &self.required_origins
    }
    /// Provenance of restart propagation.
    #[must_use]
    pub fn restart_origins(&self) -> &[Provenance] {
        &self.restart_origins
    }
    /// Whether an explicit runtime identity is protected and needs artifact authorization.
    #[must_use]
    pub const fn sensitive_identity(&self) -> bool {
        self.sensitive_identity
    }
}

impl DockerOutput {
    /// Returns the exact Engine build, release, advertised/acquisition/rendering APIs, mode, and evidence key.
    #[must_use]
    pub const fn native_profile(&self) -> &docker_lens::version::TargetProfile {
        &self.profile
    }

    /// Returns the immutable reviewed native evidence provenance for this exact profile.
    #[must_use]
    pub const fn evidence(&self) -> &NativeEvidenceReference {
        &self.evidence
    }

    /// Explicitly exposes request-only inert Engine bytes; prerequisites are not represented here.
    /// The bytes may contain protected values. Use `complete_bytes` for a reviewable artifact.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.artifact.bytes()
    }

    /// Returns `DockerLens`'s versioned complete inert artifact, including requests, prerequisites,
    /// and exact target context. The bytes may contain protected values.
    ///
    /// # Errors
    ///
    /// Returns a native error if the renderer did not retain validated provenance.
    pub fn complete_bytes(&self) -> Result<Vec<u8>, docker_lens::target::CompleteArtifactError> {
        self.artifact.complete_bytes()
    }

    /// Returns typed external-network requirements retained by the native renderer.
    #[must_use]
    pub fn network_prerequisites(&self) -> &[docker_lens::target::NetworkPrerequisite] {
        self.artifact.network_prerequisites()
    }

    /// Returns typed external-volume requirements retained by the native renderer.
    #[must_use]
    pub fn volume_prerequisites(&self) -> &[docker_lens::target::VolumePrerequisite] {
        self.artifact.volume_prerequisites()
    }

    /// Returns unresolved finite-rootless-PIDs destination obligations retained by `DockerLens`.
    ///
    /// Neither these obligations nor a metadata-compatible destination assessment prove
    /// controller delegation, limit enforcement, authenticated destination identity, or
    /// readiness. Assessment requires an explicitly selected destination snapshot; the
    /// source inventory and reviewed target profile do not satisfy these obligations.
    /// Use [`Self::complete_bytes`] to retain them in the complete inert review artifact;
    /// [`Self::bytes`] contains requests only. Existing native capability gates still apply.
    #[must_use]
    pub fn destination_pids_prerequisites(&self) -> &[docker_lens::target::DestinationPidsPrerequisite] {
        self.artifact.destination_pids_prerequisites()
    }

    /// BoxFerry-only dependency decisions retained separately from the native artifact.
    #[must_use]
    pub fn dependency_decisions(&self) -> &[DockerDependencyDecision] {
        &self.dependency_decisions
    }
}

impl fmt::Debug for DockerOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DockerOutput([redacted inert Engine requests])")
    }
}

/// Library-only Docker exporter requiring one caller-selected `DockerLens` offline profile.
#[derive(Clone, Debug)]
pub struct DockerExporter {
    native_profile: docker_lens::version::TargetProfile,
    environment_policy: DockerEnvironmentPolicy,
    invalid_target: DiagnosticCode,
    unsupported: DiagnosticCode,
    planning: DiagnosticCode,
}

impl DockerExporter {
    /// Creates an exporter; the profile is resolved against `DockerLens`'s reviewed catalogue at plan time.
    ///
    /// # Errors
    ///
    /// Returns an error only if a repository-owned diagnostic code is malformed.
    pub fn new(native_profile: docker_lens::version::TargetProfile) -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            native_profile,
            environment_policy: DockerEnvironmentPolicy::Withhold,
            invalid_target: RuleId::DockerTargetInvalid.definition().diagnostic_code()?,
            unsupported: RuleId::DockerOutputUnsupported.definition().diagnostic_code()?,
            planning: RuleId::DockerPlanningFailed.definition().diagnostic_code()?,
        })
    }

    /// Opts in to literal environment values independently of source promotion.
    #[must_use]
    pub const fn with_environment_policy(mut self, policy: DockerEnvironmentPolicy) -> Self {
        self.environment_policy = policy;
        self
    }

    /// Returns the selected exact native profile without claiming it has been reviewed.
    #[must_use]
    pub const fn native_profile(&self) -> &docker_lens::version::TargetProfile {
        &self.native_profile
    }
}

impl ExportAdapter for DockerExporter {
    type Output = DockerOutput;

    fn plan(
        &self,
        application: &Application,
        target: &TargetProfile,
    ) -> Result<ConversionPlan<Self::Output>, PlanError> {
        let (mut evidence_outcomes, mut evidence_diagnostics) =
            boxferry_engine::retained_native_evidence_losses(application, &self.unsupported)?;
        let (metadata_outcomes, metadata_diagnostics) =
            boxferry_engine::vendor_metadata_losses(application, &self.unsupported)?;
        evidence_outcomes.extend(metadata_outcomes);
        evidence_diagnostics.extend(metadata_diagnostics);
        let mut mapping = Mapping {
            exporter: self,
            application,
            resources: Vec::new(),
            networks: BTreeMap::new(),
            volumes: BTreeMap::new(),
            published_ports: BTreeMap::new(),
            service_refs: Vec::new(),
            dependency_decisions: Vec::new(),
            unsafe_service: false,
            outcomes: evidence_outcomes,
            diagnostics: evidence_diagnostics,
        };
        let catalogue = TargetCapabilityCatalog::reviewed();
        let native_capabilities = catalogue.resolve(&self.native_profile);
        let numeric_release = numeric_engine_release(&self.native_profile);
        if target.implementation() != DOCKER_TARGET
            || numeric_release.is_none_or(|version| !target.versions().contains(version))
            || native_capabilities.is_err()
        {
            mapping.loss(
                "target.docker".to_owned(),
                ConversionKind::Invalid,
                "Select an exact DockerLens reviewed build, API, mode, and evidence key within the requested numeric release range.",
            )?;
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        }
        let Ok(capabilities) = native_capabilities else {
            unreachable!("reviewed profile was checked above")
        };
        mapping.map_application()?;
        if mapping.unsafe_service {
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        }
        if mapping.resources.is_empty() {
            mapping.loss(
                "application".to_owned(),
                ConversionKind::Invalid,
                "No safe standalone Docker resource remains; repair the reported subjects.",
            )?;
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        }
        // DockerLens owns cross-field intent validation. Retain only a unique
        // source field for attributing its value-free command-clear error.
        let clear_command_subject = mapping.unique_clear_command_subject();
        let intent = match TargetIntent::new(std::mem::take(&mut mapping.resources)) {
            Ok(intent) => intent,
            Err(error) => {
                mapping.native_intent_failure(
                    "target.docker.intent".to_owned(),
                    error,
                    "DockerLens rejected the typed standalone resource graph; inspect identities and duplicate settings.",
                    clear_command_subject.as_deref(),
                )?;
                return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
            }
        };
        let graph = match DockerPlanner.plan(&intent, &capabilities) {
            Ok(graph) => graph,
            Err(error) => {
                mapping.native_planning_failure(&error)?;
                return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
            }
        };
        let Ok(artifact) = DockerApiRenderer.render(&graph) else {
            mapping.loss(
                "target.docker.render".to_owned(),
                ConversionKind::Invalid,
                "DockerLens could not render the planned inert Engine requests.",
            )?;
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        };
        if artifact.complete_bytes().is_err() {
            mapping.loss(
                "target.docker.complete_artifact".to_owned(),
                ConversionKind::Invalid,
                "DockerLens could not serialize the complete native review artifact with its target context and prerequisites.",
            )?;
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        }
        let output = DockerOutput {
            profile: capabilities.profile().clone(),
            evidence: capabilities.evidence().clone(),
            artifact,
            dependency_decisions: std::mem::take(&mut mapping.dependency_decisions),
        };
        mapping.exact("target.docker.requests");
        ConversionPlan::new(Some(output), mapping.outcomes, mapping.diagnostics)
    }
}

fn numeric_engine_release(profile: &docker_lens::version::TargetProfile) -> Option<PlatformVersion> {
    let release = profile.release().as_str();
    let numeric = match profile.identity().build() {
        EngineBuild::Upstream => release,
        EngineBuild::DebianPackage(_) => release.split_once('+').map_or(release, |(value, _)| value),
    };
    numeric.parse().ok()
}

struct Mapping<'a> {
    exporter: &'a DockerExporter,
    application: &'a Application,
    resources: Vec<TargetResource>,
    networks: BTreeMap<String, Option<ResourceRef>>,
    volumes: BTreeMap<String, Option<ResourceRef>>,
    published_ports: BTreeMap<(u16, bool), Vec<Option<IpAddr>>>,
    service_refs: Vec<(ResourceRef, String)>,
    dependency_decisions: Vec<DockerDependencyDecision>,
    unsafe_service: bool,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl Mapping<'_> {
    fn exact(&mut self, subject: impl Into<String>) {
        self.outcomes.push(ConversionOutcome::exact(subject));
    }

    fn loss(&mut self, subject: String, kind: ConversionKind, reason: &'static str) -> Result<(), PlanError> {
        let code = if subject == "target.docker" {
            &self.exporter.invalid_target
        } else if kind == ConversionKind::Invalid {
            &self.exporter.planning
        } else {
            &self.exporter.unsupported
        };
        let severity = if kind == ConversionKind::Invalid {
            Severity::Error
        } else {
            Severity::Warning
        };
        let diagnostic = Diagnostic::new(code.clone(), severity, reason)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(&subject)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
            .with_field(DiagnosticField::new(
                "decision",
                DiagnosticValue::plain(match kind {
                    ConversionKind::Invalid => "rejected",
                    ConversionKind::Approximate => "adjusted",
                    _ => "omitted",
                }),
            ));
        self.outcomes
            .push(ConversionOutcome::loss(subject, kind, code.clone())?);
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn unsupported(&mut self, subject: String) -> Result<(), PlanError> {
        self.loss(
            subject,
            ConversionKind::Unsupported,
            "This field has no reviewed standalone Docker target mapping; author a supported shape or accept partial loss.",
        )
    }

    fn loss_with_origins(
        &mut self,
        subject: String,
        kind: ConversionKind,
        reason: &'static str,
        origins: &[Provenance],
    ) -> Result<(), PlanError> {
        self.loss(subject, kind, reason)?;
        if let Some(outcome) = self.outcomes.pop() {
            self.outcomes
                .push(origins.iter().cloned().fold(outcome, ConversionOutcome::with_origin));
        }
        Ok(())
    }

    fn native_intent_failure(
        &mut self,
        subject: String,
        error: IntentError,
        reason: &'static str,
        clear_command_subject: Option<&str>,
    ) -> Result<(), PlanError> {
        let (subject, reason) = if error == IntentError::CommandClearRequiresEntrypoint {
            if let Some(field) = clear_command_subject {
                self.outcomes
                    .retain(|outcome| outcome.subject() != field || outcome.kind() != ConversionKind::Exact);
                (
                    field.to_owned(),
                    "Clearing a Docker command requires an explicit nonempty exec entrypoint; set one or omit the command clear.",
                )
            } else {
                (
                    "target.docker.command_entrypoint".to_owned(),
                    "A Docker service clears its command without an explicit nonempty exec entrypoint; review command and entrypoint together for the selected services.",
                )
            }
        } else {
            (subject, reason)
        };
        let diagnostic = Diagnostic::new(self.exporter.planning.clone(), Severity::Error, reason)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(&subject)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
            .with_field(DiagnosticField::new(
                "native_category",
                DiagnosticValue::plain(format!("{error:?}")),
            ))
            .with_field(DiagnosticField::new("decision", DiagnosticValue::plain("rejected")));
        self.outcomes.push(ConversionOutcome::loss(
            subject,
            ConversionKind::Invalid,
            self.exporter.planning.clone(),
        )?);
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn unique_clear_command_subject(&self) -> Option<String> {
        let mut candidates = self.resources.iter().filter_map(|resource| match resource {
            TargetResource::Container(container) if matches!(&container.command, ImageCommand::Clear) => {
                Some(container.reference)
            }
            _ => None,
        });
        let reference = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        self.service_refs
            .iter()
            .find(|(candidate, _)| *candidate == reference)
            .map(|(_, subject)| format!("{subject}.command"))
    }

    fn native_planning_failure(&mut self, error: &PlanningError) -> Result<(), PlanError> {
        let (subject, category, reason) = match error {
            PlanningError::UnsupportedApi { .. } => (
                "target.docker.api".to_owned(),
                "unsupported_api",
                "Select a reviewed Docker Engine profile with a supported API version.",
            ),
            PlanningError::MissingCapability { resource, field, .. } => {
                self.missing_capability_context(*resource, *field)
            }
            PlanningError::RestrictedPort { resource, .. } => {
                let service_subject = self.resource_subject(*resource);
                if self.restricted_port_failures(&service_subject)? {
                    return Ok(());
                }
                (
                    service_subject,
                    "restricted_port",
                    "Rootless Docker rejected a privileged host port; choose a port at least 1024 or a reviewed rootful profile.",
                )
            }
            PlanningError::UnsupportedNetworkIpam { resource } => (
                self.resource_subject(*resource),
                "unsupported_network_ipam",
                "The network IPAM shape is unsupported by DockerLens; choose a reviewed network configuration.",
            ),
            PlanningError::InvalidDependency => (
                "target.docker.dependencies".to_owned(),
                "invalid_dependency",
                "DockerLens rejected the resource dependency graph; inspect volume and container relationships.",
            ),
            PlanningError::DependencyMismatch { resource, .. } => (
                self.resource_subject(*resource),
                "dependency_mismatch",
                "DockerLens found an incompatible resource dependency; inspect the referenced volume and container relationships.",
            ),
            PlanningError::Cycle => (
                "target.docker.dependencies".to_owned(),
                "cycle",
                "DockerLens found a dependency cycle; remove the cyclic resource relationship.",
            ),
        };
        let capability = match error {
            PlanningError::MissingCapability { capability, .. } => Some(*capability),
            _ => None,
        };
        self.native_planning_loss(subject, category, reason, capability)
    }

    fn missing_capability_context(
        &mut self,
        resource: ResourceRef,
        field: TargetField,
    ) -> (String, &'static str, &'static str) {
        let base = self.resource_subject(resource);
        let suffix = match field {
            TargetField::Port
            | TargetField::PortExposeOnly
            | TargetField::PortHostIpv4
            | TargetField::PortHostIpv6
            | TargetField::PortMultipleBindings
            | TargetField::PortEphemeral => ".ports",
            TargetField::NetworkInternal | TargetField::NetworkExternalInternalExpectation => ".internal",
            TargetField::NamedVolume | TargetField::BindMount | TargetField::TmpfsMount | TargetField::BindRelabel => {
                ".mounts"
            }
            TargetField::Environment => ".environment",
            TargetField::Command | TargetField::CommandClear => ".command",
            TargetField::Entrypoint | TargetField::EntrypointClear => ".entrypoint",
            TargetField::Healthcheck
            | TargetField::HealthShell
            | TargetField::HealthDisabled
            | TargetField::HealthStartPeriod
            | TargetField::HealthStartInterval => ".healthcheck",
            TargetField::Restart => ".restart_policy",
            _ => "",
        };
        let reason = if matches!(field, TargetField::NetworkInternal) {
            "The reviewed Docker Engine profile lacks NetworkInternal capability; select a profile with reviewed internal-network evidence before converting this network."
        } else if matches!(field, TargetField::NetworkExternalInternalExpectation) {
            "The reviewed Docker Engine profile lacks external-network isolation prerequisite evidence; wait for independently reviewed profile admission before exporting this network."
        } else if matches!(field, TargetField::BindRelabel) {
            "The reviewed Docker Engine profile lacks SELinux bind-mount relabel capability evidence; select an independently admitted profile before exporting this relabel intent."
        } else {
            "The reviewed Docker Engine profile lacks a capability needed by this resource; select a supported profile or simplify the field."
        };
        let subject = format!("{base}{suffix}");
        if matches!(
            field,
            TargetField::NetworkInternal
                | TargetField::NetworkExternalInternalExpectation
                | TargetField::NetworkExternalReference
        ) {
            let internal = format!("{base}.internal");
            let prerequisite = format!("{base}.prerequisite");
            self.outcomes.retain(|outcome| {
                outcome.kind() != ConversionKind::Exact
                    || (outcome.subject() != subject
                        && outcome.subject() != base
                        && outcome.subject() != internal
                        && outcome.subject() != prerequisite)
            });
        }
        (subject, "missing_capability", reason)
    }

    fn restricted_port_failures(&mut self, service_subject: &str) -> Result<bool, PlanError> {
        let restricted_fields: Vec<_> = self
            .application
            .services()
            .iter()
            .filter(|service| {
                let subject = format!("services.{}", service.value().name().as_str());
                self.service_refs.iter().any(|(_, mapped)| mapped == &subject)
            })
            .flat_map(|service| {
                let subject = format!("services.{}", service.value().name().as_str());
                service
                    .value()
                    .ports()
                    .iter()
                    .enumerate()
                    .filter(|(_, port)| port.value().published().is_some_and(|host| host != 0 && host < 1024))
                    .map(|(index, _)| format!("{subject}.ports.{index}"))
                    .collect::<Vec<_>>()
            })
            .collect();
        for field in &restricted_fields {
            let parent = field
                .rsplit_once(".ports.")
                .map_or(service_subject, |(parent, _)| parent);
            self.outcomes.retain(|outcome| {
                outcome.kind() != ConversionKind::Exact
                    || (outcome.subject() != field.as_str() && outcome.subject() != parent)
            });
            self.native_planning_loss(field.clone(), "restricted_port",
                "Rootless Docker cannot publish a fixed host port below 1024; choose a port at least 1024 or a reviewed rootful profile.", None)?;
        }
        Ok(!restricted_fields.is_empty())
    }

    fn resource_subject(&self, reference: ResourceRef) -> String {
        if let Some((_, subject)) = self.service_refs.iter().find(|(candidate, _)| *candidate == reference) {
            return subject.clone();
        }
        if let Some((name, _)) = self
            .networks
            .iter()
            .find(|(_, candidate)| **candidate == Some(reference))
        {
            return format!("networks.{name}");
        }
        if let Some((name, _)) = self
            .volumes
            .iter()
            .find(|(_, candidate)| **candidate == Some(reference))
        {
            return format!("volumes.{name}");
        }
        "target.docker.resources".to_owned()
    }

    fn native_planning_loss(
        &mut self,
        subject: String,
        category: &'static str,
        reason: &'static str,
        capability: Option<Capability>,
    ) -> Result<(), PlanError> {
        let mut diagnostic = Diagnostic::new(self.exporter.planning.clone(), Severity::Error, reason)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(&subject)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
            .with_field(DiagnosticField::new(
                "native_category",
                DiagnosticValue::plain(category),
            ))
            .with_field(DiagnosticField::new("decision", DiagnosticValue::plain("rejected")));
        if let Some(capability) = capability {
            // Capability is a fieldless native enum, not a runtime value. Keep
            // the exact missing feature actionable without exposing intent.
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "native_feature",
                DiagnosticValue::plain(format!("{capability:?}")),
            ));
        }
        let origins = self.network_internal_origins(&subject);
        self.outcomes.push(origins.into_iter().fold(
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.exporter.planning.clone())?,
            ConversionOutcome::with_origin,
        ));
        self.diagnostics.push(diagnostic);
        Ok(())
    }

    fn network_internal_origins(&self, subject: &str) -> Vec<Provenance> {
        let Some(name) = subject
            .strip_prefix("networks.")
            .and_then(|subject| subject.strip_suffix(".internal"))
        else {
            return Vec::new();
        };
        let Some(network) = self
            .application
            .networks()
            .iter()
            .find(|network| network.value().name().as_str() == name)
        else {
            return Vec::new();
        };
        let mut origins = network.origins().to_vec();
        if let Some(internal) = network.value().internal() {
            for origin in internal.origins() {
                if !origins.contains(origin) {
                    origins.push(origin.clone());
                }
            }
        }
        origins
    }

    fn next_reference(&self) -> ResourceRef {
        let next = u64::try_from(self.resources.len()).map_or(u64::MAX, |count| count.saturating_add(1));
        ResourceRef::new(next)
    }

    fn map_application(&mut self) -> Result<(), PlanError> {
        self.exact("application");
        let application = self.application;
        for acquisition in application.image_acquisitions() {
            self.unsupported(format!("image_acquisitions.{}", acquisition.value().name().as_str()))?;
        }
        for build in application.image_builds() {
            self.unsupported(format!("image_builds.{}", build.value().name().as_str()))?;
        }
        for group in application.service_groups() {
            self.unsupported(format!("service_groups.{}", group.value().name().as_str()))?;
        }
        for network in application.networks() {
            self.map_network(network)?;
        }
        for config in application.configs() {
            self.unsupported(format!("configs.{}", config.value().name().as_str()))?;
        }
        for secret in application.secrets() {
            self.unsupported(format!("secrets.{}", secret.value().name().as_str()))?;
        }
        for volume in application.volumes() {
            self.map_volume(volume.value())?;
        }
        for service in application.services() {
            self.map_service(service.value())?;
        }
        self.map_dependencies()?;
        Ok(())
    }

    fn map_network(&mut self, sourced: &Sourced<boxferry_model::Network>) -> Result<(), PlanError> {
        let network = sourced.value();
        let name = network.name().as_str();
        let subject = format!("networks.{name}");
        let identity = if let Some(runtime_name) = network.runtime_name().filter(|value| value.value().is_sensitive()) {
            self.loss_with_origins(
                format!("{subject}.runtime_name"),
                ConversionKind::Unsupported,
                "A protected Docker network identity cannot be emitted to a native review artifact.",
                runtime_name.origins(),
            )?;
            None
        } else {
            let identity_text = network.runtime_name().map_or(name, |value| value.value().expose());
            if let Ok(identity) = TargetIdentity::new(identity_text.as_bytes().to_vec()) {
                Some(identity)
            } else {
                self.loss(
                    format!("{subject}.runtime_name"),
                    ConversionKind::Invalid,
                    "The requested Docker network identity is invalid; choose a portable runtime name.",
                )?;
                None
            }
        };
        let mut safe = identity.is_some();
        let mut approximate = false;
        if let Some(driver) = network.driver().filter(|value| value.value().is_sensitive()) {
            self.loss_with_origins(
                format!("{subject}.driver"),
                ConversionKind::Unsupported,
                "A protected Docker network driver cannot be used to authorize a native review artifact.",
                driver.origins(),
            )?;
            safe = false;
        }
        let driver = network
            .driver()
            .filter(|value| !value.value().is_sensitive())
            .map(|value| value.value().expose());
        if driver.is_some_and(|value| value != "bridge") {
            self.loss(
                format!("{subject}.driver"),
                ConversionKind::Unsupported,
                "Only Docker bridge networks have a reviewed standalone target mapping; choose bridge or a different target.",
            )?;
            safe = false;
        }
        let source = self.map_network_source(sourced, &subject, driver.is_some(), &mut safe, &mut approximate)?;
        for (field, present) in [
            ("driver_options", network.driver_options().is_some()),
            ("ipam_driver", network.ipam_driver().is_some()),
            ("ipam_configs", network.ipam_configs().is_some()),
        ] {
            if present {
                self.unsupported(format!("{subject}.{field}"))?;
                safe = false;
            }
        }
        let reference = if let (true, Some(identity)) = (safe, identity) {
            let reference = self.next_reference();
            self.resources.push(TargetResource::Network(NetworkIntent {
                reference,
                identity,
                role: NetworkRole::Declared,
                source,
            }));
            self.exact(format!("{subject}.identity"));
            if network.ownership() == ResourceOwnership::External {
                self.exact(format!("{subject}.prerequisite"));
                if network.internal().is_some() {
                    let internal = format!("{subject}.internal");
                    self.outcomes.push(
                        self.network_internal_origins(&internal)
                            .into_iter()
                            .fold(ConversionOutcome::exact(internal), ConversionOutcome::with_origin),
                    );
                }
            }
            if !approximate {
                self.exact(subject);
            }
            Some(reference)
        } else {
            if network.ownership() == ResourceOwnership::External && network.internal().is_some() {
                let internal = format!("{subject}.internal");
                self.loss_with_origins(internal.clone(), ConversionKind::Unsupported,
                    "The external-network isolation prerequisite cannot be retained while other network settings are unsupported; repair the reported network fields before reviewing this prerequisite.",
                    &self.network_internal_origins(&internal))?;
            }
            None
        };
        self.networks.insert(name.to_owned(), reference);
        Ok(())
    }

    fn map_network_source(
        &mut self,
        sourced: &Sourced<boxferry_model::Network>,
        subject: &str,
        has_driver: bool,
        safe: &mut bool,
        approximate: &mut bool,
    ) -> Result<NetworkSource, PlanError> {
        let network = sourced.value();
        let source = match network.ownership() {
            ResourceOwnership::Application => {
                let mut create = NetworkCreate::bridge();
                if let Some(internal) = network.internal() {
                    create.internal = *internal.value();
                    self.exact(format!("{subject}.internal"));
                }
                if let Some(ipv6) = network.ipv6() {
                    create.enable_ipv6 = *ipv6.value();
                    self.exact(format!("{subject}.ipv6"));
                }
                self.map_network_labels(network, subject, &mut create, safe)?;
                NetworkSource::Create(create)
            }
            ResourceOwnership::External => {
                if !has_driver {
                    *approximate = true;
                    self.loss_with_origins(format!("{subject}.driver"), ConversionKind::Approximate,
                        "DockerLens requires a bridge-driver constraint for an external network; verify that the existing network is bridge before use.", sourced.origins())?;
                }
                for (field, present) in [
                    ("ipv6", network.ipv6().is_some()),
                    ("labels", network.labels().is_some()),
                ] {
                    if present {
                        self.unsupported(format!("{subject}.{field}"))?;
                        *safe = false;
                    }
                }
                NetworkSource::External {
                    expected_driver: NetworkDriver::Bridge,
                    expected_internal: network.internal().map(|internal| *internal.value()),
                }
            }
            _ => {
                self.loss(
                    format!("{subject}.ownership"),
                    ConversionKind::Unsupported,
                    "Implicit or uncertain network ownership cannot be converted into create or external intent.",
                )?;
                *safe = false;
                NetworkSource::Create(NetworkCreate::bridge())
            }
        };
        Ok(source)
    }

    fn map_network_labels(
        &mut self,
        network: &boxferry_model::Network,
        subject: &str,
        create: &mut NetworkCreate,
        safe: &mut bool,
    ) -> Result<(), PlanError> {
        if let Some(labels) = network.labels() {
            let mut names = BTreeSet::new();
            for (index, label) in labels.iter().enumerate() {
                let label = label.value();
                if !names.insert(label.name().as_str()) {
                    self.loss(
                        format!("{subject}.labels.{index}"),
                        ConversionKind::Invalid,
                        "The network repeats a metadata label name; remove the duplicate.",
                    )?;
                    *safe = false;
                    continue;
                }
                if label.value().is_sensitive() {
                    self.loss_with_origins(
                        format!("{subject}.labels.{index}"),
                        ConversionKind::Unsupported,
                        "A protected Docker network label cannot be emitted without a reviewed disclosure policy.",
                        labels[index].origins(),
                    )?;
                    *safe = false;
                    continue;
                }
                if let Ok(native) = docker_lens::target::NetworkLabel::new(
                    label.name().as_str().as_bytes().to_vec(),
                    label.value().expose().as_bytes().to_vec(),
                ) {
                    create.labels.push(native);
                } else {
                    self.loss(
                        format!("{subject}.labels.{index}"),
                        ConversionKind::Invalid,
                        "The network label cannot be represented by DockerLens.",
                    )?;
                    *safe = false;
                }
            }
            if *safe {
                self.exact(format!("{subject}.labels"));
            }
        }
        Ok(())
    }

    fn map_dependencies(&mut self) -> Result<(), PlanError> {
        let services = self
            .application
            .services()
            .iter()
            .map(Sourced::value)
            .collect::<Vec<_>>();
        let names = services
            .iter()
            .map(|service| service.name().as_str())
            .collect::<BTreeSet<_>>();
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for service in &services {
            let service_name = service.name().as_str();
            let mut seen = BTreeSet::new();
            for (index, sourced) in service.dependencies().iter().enumerate() {
                let dependency = sourced.value();
                let target_name = dependency.service().as_str();
                let subject = format!("services.{service_name}.dependencies.{index}");
                if !seen.insert(target_name) {
                    self.loss_with_origins(
                        subject,
                        ConversionKind::Invalid,
                        "The service repeats a dependency edge; keep one decision per target service.",
                        sourced.origins(),
                    )?;
                    self.unsafe_service = true;
                    continue;
                }
                let Some(target) = services
                    .iter()
                    .find(|candidate| candidate.name().as_str() == target_name)
                else {
                    self.loss_with_origins(
                        subject,
                        ConversionKind::Invalid,
                        "The dependency refers to a missing service; declare the referenced application service.",
                        sourced.origins(),
                    )?;
                    self.unsafe_service = true;
                    continue;
                };
                self.map_dependency(service, target, sourced, subject, &mut edges)?;
            }
        }
        let mut unresolved = names.iter().map(|name| (*name).to_owned()).collect::<BTreeSet<_>>();
        loop {
            let resolved = unresolved
                .iter()
                .filter(|name| {
                    edges
                        .get(*name)
                        .is_none_or(|dependencies| dependencies.iter().all(|target| !unresolved.contains(target)))
                })
                .cloned()
                .collect::<Vec<_>>();
            if resolved.is_empty() {
                break;
            }
            for name in resolved {
                unresolved.remove(&name);
            }
        }
        if !unresolved.is_empty() {
            let origins = self
                .application
                .services()
                .iter()
                .flat_map(|service| service.value().dependencies())
                .flat_map(Sourced::origins)
                .cloned()
                .collect::<Vec<_>>();
            self.loss_with_origins(
                "target.docker.dependencies".to_owned(),
                ConversionKind::Invalid,
                "The service dependency graph contains a cycle; remove cyclic start/readiness edges.",
                &origins,
            )?;
            self.unsafe_service = true;
        }
        Ok(())
    }

    fn map_dependency(
        &mut self,
        service: &Service,
        target: &Service,
        sourced: &Sourced<boxferry_model::ServiceDependency>,
        subject: String,
        edges: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> Result<(), PlanError> {
        let dependency = sourced.value();
        let service_name = service.name().as_str();
        let target_name = dependency.service().as_str();
        if matches!(
            dependency.condition().map(Sourced::value),
            Some(ServiceDependencyCondition::Other(_))
        ) {
            self.loss_with_origins(format!("{subject}.condition"), ConversionKind::Unsupported,
                "The dependency condition is not supported by the reviewed Docker target; choose started, healthy, or completed successfully.",
                dependency.condition().map_or(&[], Sourced::origins))?;
            self.unsafe_service = true;
            return Ok(());
        }
        if service.runtime_name().is_some_and(|value| value.value().is_sensitive())
            || target.runtime_name().is_some_and(|value| value.value().is_sensitive())
        {
            self.loss_with_origins(
                subject,
                ConversionKind::Unsupported,
                "A protected Docker dependency identity cannot be emitted to the native plan or decision sidecar.",
                sourced.origins(),
            )?;
            self.unsafe_service = true;
            return Ok(());
        }
        let identity_text = service
            .runtime_name()
            .map_or(service_name, |value| value.value().expose());
        let target_identity_text = target
            .runtime_name()
            .map_or(target_name, |value| value.value().expose());
        let (Ok(service_identity), Ok(dependency_identity)) = (
            TargetIdentity::new(identity_text.as_bytes().to_vec()),
            TargetIdentity::new(target_identity_text.as_bytes().to_vec()),
        ) else {
            self.loss_with_origins(
                subject,
                ConversionKind::Invalid,
                "A dependency target runtime identity is invalid for DockerLens.",
                sourced.origins(),
            )?;
            self.unsafe_service = true;
            return Ok(());
        };
        edges
            .entry(service_name.to_owned())
            .or_default()
            .insert(target_name.to_owned());
        let mut origins = sourced.origins().to_vec();
        for field_origins in [
            dependency.condition().map(Sourced::origins),
            dependency.required().map(Sourced::origins),
            dependency.restart().map(Sourced::origins),
        ]
        .into_iter()
        .flatten()
        {
            origins.extend_from_slice(field_origins);
        }
        self.loss_with_origins(subject, ConversionKind::Approximate,
            "Standalone Docker create has no dependency readiness or restart-propagation field; a separate reviewed start/wait choreography is required.", &origins)?;
        self.dependency_decisions.push(DockerDependencyDecision {
            service: service_name.to_owned(),
            dependency: target_name.to_owned(),
            service_identity,
            dependency_identity,
            condition: dependency.condition().map(|value| value.value().clone()),
            required: dependency.required().map(|value| *value.value()),
            restart: dependency.restart().map(|value| *value.value()),
            reference_origins: sourced.origins().to_vec(),
            condition_origins: dependency
                .condition()
                .map_or_else(Vec::new, |value| value.origins().to_vec()),
            required_origins: dependency
                .required()
                .map_or_else(Vec::new, |value| value.origins().to_vec()),
            restart_origins: dependency
                .restart()
                .map_or_else(Vec::new, |value| value.origins().to_vec()),
            sensitive_identity: service.runtime_name().is_some_and(|value| value.value().is_sensitive())
                || target.runtime_name().is_some_and(|value| value.value().is_sensitive()),
        });
        Ok(())
    }

    fn map_volume(&mut self, volume: &boxferry_model::Volume) -> Result<(), PlanError> {
        let name = volume.name().as_str();
        let subject = format!("volumes.{name}");
        let mut safe = matches!(
            volume.ownership(),
            ResourceOwnership::Application | ResourceOwnership::External
        );
        if !safe {
            self.loss(
                format!("{subject}.ownership"),
                ConversionKind::Unsupported,
                "Implicit or uncertain volume ownership cannot be converted into create or external intent.",
            )?;
        }
        for (field, present) in [
            ("service_name", volume.service_name().is_some()),
            ("driver", volume.driver().is_some()),
            ("device", volume.device().is_some()),
            ("type", volume.volume_type().is_some()),
            ("options", volume.options().is_some()),
            ("copy", volume.copy().is_some()),
            ("user", volume.user().is_some()),
            ("group", volume.group().is_some()),
            ("uid", volume.uid().is_some()),
            ("gid", volume.gid().is_some()),
            ("image_source", volume.image_source().is_some()),
        ] {
            if present {
                self.unsupported(format!("{subject}.{field}"))?;
                safe = false;
            }
        }
        let labels = self.map_volume_labels(volume, &subject, &mut safe)?;
        if let Some(runtime_name) = volume.runtime_name().filter(|value| value.value().is_sensitive()) {
            self.loss_with_origins(
                format!("{subject}.runtime_name"),
                ConversionKind::Unsupported,
                "A protected Docker volume identity cannot be emitted to a native review artifact.",
                runtime_name.origins(),
            )?;
            safe = false;
        }
        let reference = if safe {
            let identity_text = volume.runtime_name().map_or(name, |value| value.value().expose());
            if let Ok(identity) = TargetIdentity::new(identity_text.as_bytes().to_vec()) {
                let reference = self.next_reference();
                if volume.ownership() == ResourceOwnership::External {
                    self.resources
                        .push(TargetResource::ExternalVolume { reference, identity });
                } else {
                    self.resources.push(TargetResource::Volume {
                        reference,
                        identity,
                        labels,
                    });
                }
                self.exact(subject);
                Some(reference)
            } else {
                self.loss(
                    format!("{subject}.runtime_name"),
                    ConversionKind::Invalid,
                    "The Docker volume identity is invalid; choose a portable runtime name.",
                )?;
                None
            }
        } else {
            None
        };
        self.volumes.insert(name.to_owned(), reference);
        Ok(())
    }

    fn map_volume_labels(
        &mut self,
        volume: &boxferry_model::Volume,
        subject: &str,
        safe: &mut bool,
    ) -> Result<Vec<VolumeLabel>, PlanError> {
        let mut labels = Vec::new();
        if let Some(authored) = volume.labels() {
            if volume.ownership() == ResourceOwnership::External && !authored.is_empty() {
                self.loss_with_origins(format!("{subject}.labels"), ConversionKind::Unsupported,
                    "Labels on an external Docker volume cannot be applied or verified by a create request; inspect the prerequisite volume.", volume.labels_origins())?;
                *safe = false;
            } else if volume.ownership() == ResourceOwnership::Application {
                let mut names = BTreeSet::new();
                for (index, label) in authored.iter().enumerate() {
                    let value = label.value();
                    if !names.insert(value.name().as_str()) {
                        self.loss_with_origins(
                            format!("{subject}.labels.{index}"),
                            ConversionKind::Invalid,
                            "The Docker volume repeats a metadata label name; keep one value per key.",
                            label.origins(),
                        )?;
                        *safe = false;
                        continue;
                    }
                    if value.value().is_sensitive() {
                        self.loss_with_origins(
                            format!("{subject}.labels.{index}"),
                            ConversionKind::Unsupported,
                            "A protected Docker volume label cannot be emitted without a reviewed disclosure policy.",
                            label.origins(),
                        )?;
                        *safe = false;
                        continue;
                    }
                    if let Ok(mapped) = VolumeLabel::new(
                        value.name().as_str().as_bytes().to_vec(),
                        value.value().expose().as_bytes().to_vec(),
                    ) {
                        labels.push(mapped);
                    } else {
                        self.loss_with_origins(format!("{subject}.labels.{index}"), ConversionKind::Invalid,
                            "A Docker volume label is invalid for the reviewed native target; choose a valid UTF-8 key and value within native limits.", label.origins())?;
                        *safe = false;
                    }
                }
                if *safe {
                    self.exact(format!("{subject}.labels"));
                }
            }
        }
        Ok(labels)
    }

    fn map_service(&mut self, service: &Service) -> Result<(), PlanError> {
        let subject = format!("services.{}", service.name().as_str());
        let mut safe = true;
        for (field, present) in [
            ("rootfs", service.rootfs().is_some()),
            ("image_acquisition", service.image_acquisition().is_some()),
            ("image_build", service.image_build().is_some()),
        ] {
            if present {
                self.unsupported(format!("{subject}.{field}"))?;
                safe = false;
            }
        }
        if self.unsupported_service_fields(service, &subject)? {
            safe = false;
        }
        let image = self.map_service_image(service, &subject)?;
        let identity = self.map_service_identity(service, &subject)?;
        let command = self.map_command(service.command(), &subject)?;
        let entrypoint = self.map_entrypoint(service.entrypoint(), &subject)?;
        let (environment, environment_safe, environment_complete) = self.map_environment(service, &subject)?;
        let (ports, ports_safe) = self.map_ports(service, &subject)?;
        // An inert Engine create request has no artifact-visible marker for a
        // required assignment omitted under partial loss. Never publish one
        // that appears complete when its environment is known incomplete.
        safe &= environment_safe && environment_complete && ports_safe;
        let mounts = self.map_mounts(service, &subject, &mut safe)?;
        let (networks, networks_safe) = self.map_networks(service, &subject)?;
        safe &= networks_safe;
        let restart = self.map_restart(service, &subject)?;
        let healthcheck = self.map_healthcheck(service, &subject)?;
        let (settings, settings_safe) = self.map_settings(service, &subject)?;
        safe &= settings_safe;
        if service.command().is_some() && command.is_none()
            || service.entrypoint().is_some() && entrypoint.is_none()
            || service.healthcheck().is_some() && healthcheck.is_none()
            || service.restart_policy().is_some() && restart.is_none()
        {
            safe = false;
        }
        let (Some(identity), Some(image)) = (identity, image) else {
            self.unsafe_service = true;
            return Ok(());
        };
        if !safe {
            self.unsupported(format!("{subject}.container"))?;
            self.unsafe_service = true;
            return Ok(());
        }
        let reference = self.next_reference();
        self.resources.push(TargetResource::Container(Box::new(ContainerIntent {
            reference,
            identity,
            image,
            environment,
            ports,
            mounts,
            networks,
            entrypoint: entrypoint.unwrap_or(ImageCommand::Inherit),
            command: command.unwrap_or(ImageCommand::Inherit),
            healthcheck,
            restart,
            settings,
        })));
        self.service_refs.push((reference, subject.clone()));
        if service.dependencies().is_empty() {
            self.exact(subject);
        }
        Ok(())
    }

    fn map_service_image(&mut self, service: &Service, subject: &str) -> Result<Option<ImageReference>, PlanError> {
        let image = if let Some(value) = service.image() {
            let authored = value.value().as_str();
            // The neutral model preserves parameterized source text. Detect parameter
            // intent without resolving it or duplicating DockerLens's native image grammar.
            let parameterized = authored
                .as_bytes()
                .windows(2)
                .any(|pair| pair[0] == b'$' && (matches!(pair[1], b'{' | b'_') || pair[1].is_ascii_alphabetic()));
            if parameterized {
                self.loss_with_origins(
                    format!("{subject}.image"),
                    ConversionKind::Invalid,
                    "The image contains unresolved source parameters; resolve them to a literal image before converting.",
                    value.origins(),
                )?;
                None
            } else if let Ok(image) = ImageReference::new(authored.as_bytes().to_vec()) {
                self.exact(format!("{subject}.image"));
                Some(image)
            } else {
                self.loss(
                    format!("{subject}.image"),
                    ConversionKind::Invalid,
                    "The Docker image reference is invalid; author a portable named image.",
                )?;
                None
            }
        } else {
            self.loss(
                format!("{subject}.image"),
                ConversionKind::Invalid,
                "A standalone Docker container requires an explicit image reference.",
            )?;
            None
        };
        Ok(image)
    }

    fn map_service_identity(&mut self, service: &Service, subject: &str) -> Result<Option<TargetIdentity>, PlanError> {
        let identity = if let Some(runtime_name) = service.runtime_name().filter(|value| value.value().is_sensitive()) {
            self.loss_with_origins(
                format!("{subject}.runtime_name"),
                ConversionKind::Unsupported,
                "A protected Docker container identity cannot be emitted to a native review artifact.",
                runtime_name.origins(),
            )?;
            None
        } else {
            let identity_text = service
                .runtime_name()
                .map_or(service.name().as_str(), |value| value.value().expose());
            if let Ok(identity) = TargetIdentity::new(identity_text.as_bytes().to_vec()) {
                Some(identity)
            } else {
                self.loss(
                    format!("{subject}.runtime_name"),
                    ConversionKind::Invalid,
                    "The requested Docker container identity is invalid; choose a portable runtime name.",
                )?;
                None
            }
        };
        Ok(identity)
    }

    fn unsupported_service_fields(&mut self, service: &Service, subject: &str) -> Result<bool, PlanError> {
        let mut omitted = false;
        if let Some(environment) = service.execution_environment() {
            self.loss_with_origins(
                format!("{subject}.execution_environment"),
                ConversionKind::Unsupported,
                "Resolve the service execution environment outside this inert Docker artifact; provider plans are not Engine configuration.",
                environment.origins(),
            )?;
            omitted = true;
        }
        if let Some(policy) = service.pull_policy() {
            if matches!(policy.value(), PullPolicy::Never) {
                // The inert native artifact contains create requests only; it never acquires an image.
                self.exact(format!("{subject}.pull_policy"));
            } else {
                self.loss_with_origins(format!("{subject}.pull_policy"), ConversionKind::Unsupported,
                    "This Docker artifact performs no image acquisition; use pull_policy never with a separately supplied image, or review acquisition outside BoxFerry.",
                    policy.origins())?;
                omitted = true;
            }
        }
        for (field, present) in [
            ("startup_notification", service.startup_notification().is_some()),
            ("annotations", service.annotations().is_some()),
            ("logging", service.logging().is_some()),
            ("reload_action", service.reload_action().is_some()),
            ("user_namespace", service.user_namespace().is_some()),
            ("dns_options", service.dns_options().is_some()),
            ("dns_search_domains", service.dns_search_domains().is_some()),
            ("tmpfs", service.tmpfs().is_some()),
            ("sysctls", service.sysctls().is_some()),
            ("ulimits", service.ulimits().is_some()),
            ("devices", service.devices().is_some()),
            ("environment_files", !service.environment_files().is_empty()),
            ("host_mappings", !service.host_mappings().is_empty()),
            ("config_grants", !service.config_grants().is_empty()),
            ("secret_grants", !service.secret_grants().is_empty()),
        ] {
            if present {
                self.unsupported(format!("{subject}.{field}"))?;
                omitted = true;
            }
        }
        Ok(omitted)
    }

    fn map_settings(&mut self, service: &Service, subject: &str) -> Result<(ContainerSettings, bool), PlanError> {
        let mut settings = ContainerSettings::default();
        let mut safe = true;
        for (field, value) in [
            ("user", service.user()),
            ("group", service.group()),
            ("working_directory", service.working_directory()),
            ("hostname", service.hostname()),
            ("stop_signal", service.stop_signal()),
            ("memory_limit", service.memory_limit()),
            ("pids_limit", service.pids_limit()),
            ("shm_size", service.shm_size()),
        ] {
            if let Some(value) = value.filter(|value| value.value().is_sensitive()) {
                self.loss_with_origins(
                    format!("{subject}.{field}"),
                    ConversionKind::Unsupported,
                    "A protected Docker setting cannot be emitted to a native review artifact.",
                    value.origins(),
                )?;
                safe = false;
            }
        }
        for (field, values) in [
            ("supplementary_groups", service.supplementary_groups()),
            ("dns_servers", service.dns_servers().unwrap_or(&[])),
            ("cap_add", service.cap_add().unwrap_or(&[])),
            ("cap_drop", service.cap_drop().unwrap_or(&[])),
        ] {
            for (index, value) in values
                .iter()
                .enumerate()
                .filter(|(_, value)| value.value().is_sensitive())
            {
                self.loss_with_origins(
                    format!("{subject}.{field}.{index}"),
                    ConversionKind::Unsupported,
                    "A protected Docker setting cannot be emitted to a native review artifact.",
                    value.origins(),
                )?;
                safe = false;
            }
        }
        if !safe {
            return Ok((settings, false));
        }
        if let Some(value) = service.run_init() {
            settings.init = Some(*value.value());
            self.exact(format!("{subject}.run_init"));
        }
        if let Some(value) = service.read_only_root_filesystem() {
            settings.read_only_rootfs = Some(*value.value());
            self.exact(format!("{subject}.read_only_root_filesystem"));
        }
        safe = self.map_process_settings(service, subject, &mut settings, safe)?;
        safe = self.map_resource_settings(service, subject, &mut settings, safe)?;
        safe = self.map_label_settings(service, subject, &mut settings, safe)?;
        safe = self.map_network_security_settings(service, subject, &mut settings, safe)?;
        Ok((settings, safe))
    }

    fn map_process_settings(
        &mut self,
        service: &Service,
        subject: &str,
        settings: &mut ContainerSettings,
        mut safe: bool,
    ) -> Result<bool, PlanError> {
        if let Some(value) = service.user() {
            let user = value.value().expose();
            let combined = service
                .group()
                .map_or_else(|| user.to_owned(), |group| format!("{user}:{}", group.value().expose()));
            if let Ok(native) = ContainerUser::new(combined.into_bytes()) {
                settings.user = Some(native);
                self.exact(format!("{subject}.user"));
                if service.group().is_some() {
                    self.exact(format!("{subject}.group"));
                }
            } else {
                self.loss(
                    format!("{subject}.user"),
                    ConversionKind::Invalid,
                    "The Docker user/group spelling is invalid.",
                )?;
                safe = false;
            }
        } else if service.group().is_some() {
            self.unsupported(format!("{subject}.group"))?;
            safe = false;
        }
        for (index, value) in service.supplementary_groups().iter().enumerate() {
            if let Ok(native) = ContainerUser::new(value.value().expose().as_bytes().to_vec()) {
                settings.group_add.push(native);
                self.exact(format!("{subject}.supplementary_groups.{index}"));
            } else {
                self.loss(
                    format!("{subject}.supplementary_groups.{index}"),
                    ConversionKind::Invalid,
                    "The Docker supplementary group is invalid.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.working_directory() {
            if let Ok(native) = WorkingDirectory::new(value.value().expose().as_bytes().to_vec()) {
                settings.working_dir = Some(native);
                self.exact(format!("{subject}.working_directory"));
            } else {
                self.loss(
                    format!("{subject}.working_directory"),
                    ConversionKind::Invalid,
                    "The Docker working directory must be an absolute path.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.hostname() {
            if let Ok(native) = ContainerHostname::new(value.value().expose().as_bytes().to_vec()) {
                settings.hostname = Some(native);
                self.exact(format!("{subject}.hostname"));
            } else {
                self.loss(
                    format!("{subject}.hostname"),
                    ConversionKind::Invalid,
                    "The Docker hostname is invalid.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.stop_signal() {
            if let Ok(native) = Argument::new(value.value().expose().as_bytes().to_vec()) {
                settings.stop_signal = Some(native);
                self.exact(format!("{subject}.stop_signal"));
            } else {
                self.loss(
                    format!("{subject}.stop_signal"),
                    ConversionKind::Invalid,
                    "The Docker stop signal is invalid.",
                )?;
                safe = false;
            }
        }
        Ok(safe)
    }

    fn map_resource_settings(
        &mut self,
        service: &Service,
        subject: &str,
        settings: &mut ContainerSettings,
        mut safe: bool,
    ) -> Result<bool, PlanError> {
        if let Some(value) = service.stop_timeout() {
            let seconds = parse_duration_ns(value.value().as_str())
                .ok()
                .filter(|nanos| nanos % 1_000_000_000 == 0)
                .and_then(|nanos| u32::try_from(nanos / 1_000_000_000).ok());
            if let Some(seconds) = seconds {
                settings.stop_timeout_seconds = Some(seconds);
                self.exact(format!("{subject}.stop_timeout"));
            } else {
                self.loss(
                    format!("{subject}.stop_timeout"),
                    ConversionKind::Invalid,
                    "The Docker stop timeout must be a finite whole number of seconds.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.memory_limit() {
            let memory = parse_size_bytes(value.value().expose())
                .map(|bytes| NonZeroU64::new(bytes).map_or(MemoryLimit::Unlimited, MemoryLimit::Bytes));
            if let Some(memory) = memory {
                settings.memory_limit = Some(memory);
                self.exact(format!("{subject}.memory_limit"));
            } else {
                self.loss(
                    format!("{subject}.memory_limit"),
                    ConversionKind::Invalid,
                    "The Docker memory limit must be a finite byte size with an optional k, m, or g suffix.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.pids_limit() {
            let text = value.value().expose();
            let limit = if text == "-1" {
                Some(PidsLimit::Unlimited)
            } else {
                text.parse::<u64>().ok().and_then(NonZeroU64::new).map(PidsLimit::Count)
            };
            if let Some(limit) = limit {
                settings.pids_limit = Some(limit);
                self.exact(format!("{subject}.pids_limit"));
            } else {
                self.loss(
                    format!("{subject}.pids_limit"),
                    ConversionKind::Invalid,
                    "The Docker PID limit must be a positive count or -1 for unlimited.",
                )?;
                safe = false;
            }
        }
        if let Some(value) = service.shm_size() {
            let size = parse_size_bytes(value.value().expose()).and_then(NonZeroU64::new);
            if let Some(size) = size {
                settings.shm_size_bytes = Some(size);
                self.exact(format!("{subject}.shm_size"));
            } else {
                self.loss(
                    format!("{subject}.shm_size"),
                    ConversionKind::Invalid,
                    "The Docker shared-memory size must be a positive finite byte size.",
                )?;
                safe = false;
            }
        }
        Ok(safe)
    }

    fn map_label_settings(
        &mut self,
        service: &Service,
        subject: &str,
        settings: &mut ContainerSettings,
        mut safe: bool,
    ) -> Result<bool, PlanError> {
        let mut label_names = BTreeSet::new();
        for (index, value) in service.labels().iter().enumerate() {
            let label = value.value();
            if !label_names.insert(label.name().as_str()) {
                self.loss(
                    format!("{subject}.labels.{index}"),
                    ConversionKind::Invalid,
                    "The service repeats a container label name.",
                )?;
                safe = false;
                continue;
            }
            if label.value().is_sensitive() {
                self.loss_with_origins(
                    format!("{subject}.labels.{index}"),
                    ConversionKind::Unsupported,
                    "A protected Docker container label cannot be emitted without a reviewed disclosure policy.",
                    value.origins(),
                )?;
                safe = false;
                continue;
            }
            if let Ok(native) = ContainerLabel::new(
                label.name().as_str().as_bytes().to_vec(),
                label.value().expose().as_bytes().to_vec(),
            ) {
                settings.labels.push(native);
            } else {
                self.loss(
                    format!("{subject}.labels.{index}"),
                    ConversionKind::Invalid,
                    "The Docker container label is invalid.",
                )?;
                safe = false;
            }
        }
        if !service.labels().is_empty() && safe {
            self.exact(format!("{subject}.labels"));
        }
        Ok(safe)
    }

    fn map_network_security_settings(
        &mut self,
        service: &Service,
        subject: &str,
        settings: &mut ContainerSettings,
        mut safe: bool,
    ) -> Result<bool, PlanError> {
        if let Some(values) = service.dns_servers() {
            for (index, value) in values.iter().enumerate() {
                if let Ok(address) = value.value().expose().parse::<IpAddr>() {
                    settings.dns.push(address);
                } else {
                    self.loss(
                        format!("{subject}.dns_servers.{index}"),
                        ConversionKind::Invalid,
                        "A Docker DNS server must be a literal IP address.",
                    )?;
                    safe = false;
                }
            }
            if safe {
                self.exact(format!("{subject}.dns_servers"));
            }
        }
        for (name, values, target) in [
            ("cap_add", service.cap_add(), &mut settings.cap_add),
            ("cap_drop", service.cap_drop(), &mut settings.cap_drop),
        ] {
            if let Some(values) = values {
                for (index, value) in values.iter().enumerate() {
                    if let Ok(native) = ContainerToken::new(value.value().expose().as_bytes().to_vec()) {
                        target.push(native);
                    } else {
                        self.loss(
                            format!("{subject}.{name}.{index}"),
                            ConversionKind::Invalid,
                            "The Docker capability token is invalid.",
                        )?;
                        safe = false;
                    }
                }
                if safe {
                    self.exact(format!("{subject}.{name}"));
                }
            }
        }
        if let Some(values) = service.security_options() {
            for (index, value) in values.iter().enumerate() {
                if let boxferry_model::SecurityOption::NoNewPrivileges(enabled) = value.value() {
                    settings
                        .security_options
                        .push(NativeSecurityOption::NoNewPrivileges(*enabled));
                    self.exact(format!("{subject}.security_options.{index}"));
                } else {
                    self.unsupported(format!("{subject}.security_options.{index}"))?;
                    safe = false;
                }
            }
        }
        Ok(safe)
    }

    fn map_command(
        &mut self,
        command: Option<&Sourced<Command>>,
        subject: &str,
    ) -> Result<Option<ImageCommand>, PlanError> {
        let Some(sourced) = command else {
            return Ok(None);
        };
        let command = sourced.value();
        if match command {
            Command::Exec(values) => values.iter().any(boxferry_model::ProtectedString::is_sensitive),
            Command::Shell(value) => value.is_sensitive(),
            _ => false,
        } {
            self.loss_with_origins(
                format!("{subject}.command"),
                ConversionKind::Unsupported,
                "A protected Docker command cannot be emitted to a native review artifact.",
                sourced.origins(),
            )?;
            return Ok(None);
        }
        match command {
            Command::Exec(values) if values.first().is_some_and(|value| !value.expose().is_empty()) => {
                let mut arguments = Vec::with_capacity(values.len());
                for value in values {
                    let Ok(argument) = Argument::new(value.expose().as_bytes().to_vec()) else {
                        self.loss(
                            format!("{subject}.command"),
                            ConversionKind::Invalid,
                            "The exec command contains a value DockerLens cannot represent.",
                        )?;
                        return Ok(None);
                    };
                    arguments.push(argument);
                }
                self.exact(format!("{subject}.command"));
                Ok(Some(ImageCommand::Exec(arguments)))
            }
            Command::Exec(_) => {
                self.loss(
                    format!("{subject}.command"),
                    ConversionKind::Invalid,
                    "An exec command requires a nonempty first argument.",
                )?;
                Ok(None)
            }
            Command::Empty => {
                self.exact(format!("{subject}.command"));
                Ok(Some(ImageCommand::Clear))
            }
            Command::Shell(text) => {
                let arguments = ["/bin/sh", "-c", text.expose()]
                    .into_iter()
                    .map(|value| Argument::new(value.as_bytes().to_vec()))
                    .collect::<Result<Vec<_>, _>>();
                if let Ok(arguments) = arguments {
                    self.loss(
                        format!("{subject}.command"),
                        ConversionKind::Approximate,
                        "Shell command text is lowered to `/bin/sh -c`; verify the target image shell and quoting semantics.",
                    )?;
                    Ok(Some(ImageCommand::Exec(arguments)))
                } else {
                    self.loss(
                        format!("{subject}.command"),
                        ConversionKind::Invalid,
                        "The shell command cannot be represented by DockerLens.",
                    )?;
                    Ok(None)
                }
            }
            _ => {
                self.unsupported(format!("{subject}.command"))?;
                Ok(None)
            }
        }
    }

    fn map_entrypoint(
        &mut self,
        entrypoint: Option<&Sourced<Entrypoint>>,
        subject: &str,
    ) -> Result<Option<ImageCommand>, PlanError> {
        let Some(sourced) = entrypoint else {
            return Ok(None);
        };
        let entrypoint = sourced.value();
        if match entrypoint {
            Entrypoint::Exec(values) => values.iter().any(boxferry_model::ProtectedString::is_sensitive),
            Entrypoint::Shell(value) => value.is_sensitive(),
            _ => false,
        } {
            self.loss_with_origins(
                format!("{subject}.entrypoint"),
                ConversionKind::Unsupported,
                "A protected Docker entrypoint cannot be emitted to a native review artifact.",
                sourced.origins(),
            )?;
            return Ok(None);
        }
        match entrypoint {
            Entrypoint::Exec(values) if values.first().is_some_and(|value| !value.expose().is_empty()) => {
                let mut arguments = Vec::with_capacity(values.len());
                for value in values {
                    let Ok(argument) = Argument::new(value.expose().as_bytes().to_vec()) else {
                        self.loss(
                            format!("{subject}.entrypoint"),
                            ConversionKind::Invalid,
                            "The exec entrypoint contains a value DockerLens cannot represent.",
                        )?;
                        return Ok(None);
                    };
                    arguments.push(argument);
                }
                self.exact(format!("{subject}.entrypoint"));
                Ok(Some(ImageCommand::Exec(arguments)))
            }
            Entrypoint::Exec(_) => {
                self.loss(
                    format!("{subject}.entrypoint"),
                    ConversionKind::Invalid,
                    "An exec entrypoint requires a nonempty first argument.",
                )?;
                Ok(None)
            }
            Entrypoint::Empty => {
                self.exact(format!("{subject}.entrypoint"));
                Ok(Some(ImageCommand::Clear))
            }
            Entrypoint::Shell(text) => {
                let arguments = ["/bin/sh", "-c", text.expose()]
                    .into_iter()
                    .map(|value| Argument::new(value.as_bytes().to_vec()))
                    .collect::<Result<Vec<_>, _>>();
                if let Ok(arguments) = arguments {
                    self.loss(
                        format!("{subject}.entrypoint"),
                        ConversionKind::Approximate,
                        "Shell entrypoint text is lowered to `/bin/sh -c`; verify the target image shell and argument-combination semantics.",
                    )?;
                    Ok(Some(ImageCommand::Exec(arguments)))
                } else {
                    self.loss(
                        format!("{subject}.entrypoint"),
                        ConversionKind::Invalid,
                        "The shell entrypoint cannot be represented by DockerLens.",
                    )?;
                    Ok(None)
                }
            }
            _ => {
                self.unsupported(format!("{subject}.entrypoint"))?;
                Ok(None)
            }
        }
    }

    fn map_environment(
        &mut self,
        service: &Service,
        subject: &str,
    ) -> Result<(Vec<EnvironmentAssignment>, bool, bool), PlanError> {
        let mut assignments = Vec::new();
        let mut names = BTreeSet::new();
        let mut safe = true;
        let mut complete = true;
        for variable in service.environment() {
            let value = variable.value();
            let name = value.name().as_str();
            let field = format!("{subject}.environment.{name}");
            if !names.insert(name) {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "This service repeats an environment name; remove the duplicate assignment before Docker export.",
                )?;
                safe = false;
                complete = false;
                continue;
            }
            match value.value() {
                EnvironmentValue::Literal(literal)
                    if self.exporter.environment_policy == DockerEnvironmentPolicy::IncludeLiteral =>
                {
                    if let Ok(assignment) =
                        EnvironmentAssignment::new(name.as_bytes().to_vec(), literal.expose().as_bytes().to_vec())
                    {
                        assignments.push(assignment);
                        self.exact(field);
                    } else {
                        self.loss(
                            field,
                            ConversionKind::Invalid,
                            "The environment assignment is invalid for DockerLens; review its name and literal value.",
                        )?;
                        safe = false;
                        complete = false;
                    }
                }
                EnvironmentValue::Literal(_) => {
                    self.loss(
                        field,
                        ConversionKind::Unsupported,
                        "Literal environment output is withheld by default; opt in separately or supply the value outside this artifact.",
                    )?;
                    complete = false;
                }
                _ => {
                    self.loss(
                        field,
                        ConversionKind::Unsupported,
                        "This environment value is not a literal; resolve it explicitly before Docker target export.",
                    )?;
                    complete = false;
                }
            }
        }
        Ok((assignments, safe, complete))
    }

    fn map_ports(&mut self, service: &Service, subject: &str) -> Result<(Vec<PortPublication>, bool), PlanError> {
        let mut grouped: BTreeMap<(u16, bool), Vec<HostBinding>> = BTreeMap::new();
        let mut safe = true;
        for (index, sourced) in service.ports().iter().enumerate() {
            let field = format!("{subject}.ports.{index}");
            safe &= self.map_port(sourced.value(), field, &mut grouped)?;
        }
        if let Some(exposed) = service.exposed_ports() {
            for (index, sourced) in exposed.iter().enumerate() {
                let field = format!("{subject}.exposed_ports.{index}");
                let port = sourced.value();
                let protocol = match port.protocol() {
                    Protocol::Tcp => false,
                    Protocol::Udp => true,
                    _ => {
                        self.unsupported(field)?;
                        safe = false;
                        continue;
                    }
                };
                grouped.entry((port.container(), protocol)).or_default();
                self.exact(field);
            }
        }
        let mut ports = Vec::with_capacity(grouped.len());
        for ((number, udp), bindings) in grouped {
            let Some(container) = NonZeroU16::new(number) else {
                safe = false;
                continue;
            };
            let protocol = if udp {
                docker_lens::target::Protocol::Udp
            } else {
                docker_lens::target::Protocol::Tcp
            };
            if bindings.is_empty() {
                ports.push(PortPublication::exposed(container, protocol));
            } else if let Ok(publication) = PortPublication::published(container, protocol, bindings) {
                ports.push(publication);
            } else {
                safe = false;
            }
        }
        Ok((ports, safe))
    }

    fn map_port(
        &mut self,
        port: &boxferry_model::Port,
        field: String,
        grouped: &mut BTreeMap<(u16, bool), Vec<HostBinding>>,
    ) -> Result<bool, PlanError> {
        let protocol = match port.protocol() {
            Protocol::Tcp => docker_lens::target::Protocol::Tcp,
            Protocol::Udp => docker_lens::target::Protocol::Udp,
            _ => {
                self.unsupported(field)?;
                return Ok(false);
            }
        };
        let Some(container) = NonZeroU16::new(port.container()) else {
            self.loss(
                field,
                ConversionKind::Invalid,
                "The container port must be nonzero for DockerLens.",
            )?;
            return Ok(false);
        };
        let host_ip = if let Some(text) = port.host_address() {
            if let Ok(address) = text.parse::<IpAddr>() {
                PortHostIp::Address(address)
            } else {
                self.loss(
                    format!("{field}.host_address"),
                    ConversionKind::Invalid,
                    "The host publication address must be an IPv4 or IPv6 literal.",
                )?;
                return Ok(false);
            }
        } else {
            PortHostIp::Unspecified
        };
        let key = (container.get(), protocol == docker_lens::target::Protocol::Udp);
        let binding = match port.published().map(NonZeroU16::new) {
            Some(None) => Some(HostBinding {
                host_ip,
                host_port: PortHostPort::Ephemeral,
            }),
            Some(Some(host)) => {
                let address = match host_ip {
                    PortHostIp::Unspecified => None,
                    PortHostIp::Address(value) => Some(value),
                };
                let used = self.published_ports.entry((host.get(), key.1)).or_default();
                if used.iter().any(|existing| {
                    existing.is_none()
                        || address.is_none()
                        || existing.as_ref().is_some_and(IpAddr::is_unspecified)
                        || address.is_some_and(|value| value.is_unspecified())
                        || *existing == address
                }) {
                    self.loss(
                        field,
                        ConversionKind::Invalid,
                        "This host address, port, and protocol conflict with another application publication.",
                    )?;
                    return Ok(false);
                }
                used.push(address);
                Some(HostBinding {
                    host_ip,
                    host_port: PortHostPort::Fixed(host),
                })
            }
            None if port.host_address().is_some() => {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "An exposed-only port cannot carry a host publication address.",
                )?;
                return Ok(false);
            }
            None => None,
        };
        let bindings = grouped.entry(key).or_default();
        if let Some(binding) = binding {
            bindings.push(binding);
        }
        self.exact(field);
        Ok(true)
    }

    fn map_mounts(&mut self, service: &Service, subject: &str, safe: &mut bool) -> Result<Vec<Mount>, PlanError> {
        let mut mounts = Vec::new();
        let mut targets = BTreeSet::new();
        for (index, sourced) in service.mounts().iter().enumerate() {
            let mount = sourced.value();
            let field = format!("{subject}.mounts.{index}");
            if !targets.insert(mount.target()) {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "This service repeats a container mount destination; choose one attachment for each target path.",
                )?;
                *safe = false;
                continue;
            }
            if mount.selinux_relabel().is_some() && !matches!(mount.source(), MountSource::HostPath(_)) {
                self.loss_with_origins(
                    format!("{field}.selinux_relabel"),
                    ConversionKind::Invalid,
                    "SELinux relabelling requires a host-path bind; choose a bind mount or remove relabelling from this named-volume or tmpfs mount.",
                    sourced.origins(),
                )?;
                *safe = false;
                continue;
            }
            let native = match mount.source() {
                MountSource::Volume(name) => {
                    let Some(Some(reference)) = self.volumes.get(name.as_str()).copied() else {
                        self.loss(
                            field,
                            ConversionKind::Invalid,
                            "The mount references a missing or unrepresentable named volume; declare an application-owned or external volume.",
                        )?;
                        *safe = false;
                        continue;
                    };
                    Mount::volume(reference, mount.target().as_bytes().to_vec(), mount.read_only())
                }
                MountSource::HostPath(_) => native_bind_mount(mount),
                _ => {
                    self.unsupported(field)?;
                    *safe = false;
                    continue;
                }
            };
            if let Ok(native) = native {
                mounts.push(native);
                self.exact(field);
                if mount.selinux_relabel().is_some() {
                    let mut outcome = ConversionOutcome::exact(format!("{subject}.mounts.{index}.selinux_relabel"));
                    for origin in sourced.origins() {
                        outcome = outcome.with_origin(origin.clone());
                    }
                    self.outcomes.push(outcome);
                }
            } else {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "The Docker mount requires valid absolute paths; relabelled binds also require colon-free paths. Resolve the source and destination explicitly.",
                )?;
                *safe = false;
            }
        }
        Ok(mounts)
    }

    fn map_networks(
        &mut self,
        service: &Service,
        subject: &str,
    ) -> Result<(Vec<NetworkAttachmentIntent>, bool), PlanError> {
        let mut attachments = Vec::new();
        let mut safe = true;
        let mut seen = BTreeSet::new();
        for (index, sourced) in service.networks().iter().enumerate() {
            let attachment = sourced.value();
            let field = format!("{subject}.networks.{index}");
            let name = attachment.network().as_str();
            if !seen.insert(name) {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "The service repeats a network attachment; retain only one attachment for each network.",
                )?;
                safe = false;
                continue;
            }
            let Some(Some(reference)) = self.networks.get(name).copied() else {
                self.loss(
                    field,
                    ConversionKind::Invalid,
                    "The service references a missing or unrepresentable application network; declare a reviewed target network.",
                )?;
                safe = false;
                continue;
            };
            let native_aliases = self.map_network_aliases(attachment, &field, &mut safe)?;
            let ipv4_address = match attachment.ipv4_address() {
                Some(value) if value.value().is_sensitive() => {
                    self.loss_with_origins(
                        format!("{field}.ipv4_address"),
                        ConversionKind::Unsupported,
                        "A protected Docker network address cannot be emitted to a native review artifact.",
                        value.origins(),
                    )?;
                    safe = false;
                    None
                }
                Some(value) => match NetworkAddress::new(value.value().expose()) {
                    Ok(address) if !address.is_ipv6() => Some(address),
                    _ => {
                        self.loss(
                            format!("{field}.ipv4_address"),
                            ConversionKind::Invalid,
                            "The requested IPv4 address is invalid for DockerLens.",
                        )?;
                        safe = false;
                        None
                    }
                },
                None => None,
            };
            let ipv6_address = match attachment.ipv6_address() {
                Some(value) if value.value().is_sensitive() => {
                    self.loss_with_origins(
                        format!("{field}.ipv6_address"),
                        ConversionKind::Unsupported,
                        "A protected Docker network address cannot be emitted to a native review artifact.",
                        value.origins(),
                    )?;
                    safe = false;
                    None
                }
                Some(value) => match NetworkAddress::new(value.value().expose()) {
                    Ok(address) if address.is_ipv6() => Some(address),
                    _ => {
                        self.loss(
                            format!("{field}.ipv6_address"),
                            ConversionKind::Invalid,
                            "The requested IPv6 address is invalid for DockerLens.",
                        )?;
                        safe = false;
                        None
                    }
                },
                None => None,
            };
            attachments.push(NetworkAttachmentIntent {
                network: reference,
                aliases: native_aliases,
                ipv4_address,
                ipv6_address,
            });
            self.exact(field);
        }
        Ok((attachments, safe))
    }

    fn map_network_aliases(
        &mut self,
        attachment: &boxferry_model::NetworkAttachment,
        field: &str,
        safe: &mut bool,
    ) -> Result<Vec<NetworkAlias>, PlanError> {
        let mut native_aliases = Vec::new();
        for (alias_index, alias) in attachment.aliases().iter().enumerate() {
            if attachment
                .alias_sensitivities()
                .get(alias_index)
                .copied()
                .unwrap_or(true)
            {
                self.loss(
                    format!("{field}.aliases.{alias_index}"),
                    ConversionKind::Unsupported,
                    "A protected network alias needs independent target-artifact value authorization.",
                )?;
                *safe = false;
                continue;
            }
            if let Ok(native) = NetworkAlias::new(alias.as_bytes().to_vec()) {
                native_aliases.push(native);
            } else {
                self.loss(
                    format!("{field}.aliases.{alias_index}"),
                    ConversionKind::Invalid,
                    "The network alias is invalid for DockerLens; choose a portable alias.",
                )?;
                *safe = false;
            }
        }
        Ok(native_aliases)
    }

    fn map_restart(&mut self, service: &Service, subject: &str) -> Result<Option<RestartPolicy>, PlanError> {
        let Some(value) = service.restart_policy() else {
            return Ok(None);
        };
        let native = match value.value() {
            NeutralRestartPolicy::Never => Some(RestartPolicy::No),
            NeutralRestartPolicy::Always => Some(RestartPolicy::Always),
            NeutralRestartPolicy::UnlessStopped => Some(RestartPolicy::UnlessStopped),
            NeutralRestartPolicy::OnFailure { maximum_retries } => {
                let retries = maximum_retries.map_or(Some(0), |count| u32::try_from(count.get()).ok());
                retries.map(|maximum_retries| RestartPolicy::OnFailure { maximum_retries })
            }
            _ => None,
        };
        if native.is_some() {
            self.exact(format!("{subject}.restart_policy"));
        } else {
            self.unsupported(format!("{subject}.restart_policy"))?;
        }
        Ok(native)
    }

    fn map_healthcheck(&mut self, service: &Service, subject: &str) -> Result<Option<Healthcheck>, PlanError> {
        let Some(value) = service.healthcheck() else {
            return Ok(None);
        };
        let source = value.value();
        let field = format!("{subject}.healthcheck");
        if let Some(command) = source.command() {
            let protected = match command.value() {
                HealthcheckCommand::Exec(arguments) => {
                    arguments.iter().any(boxferry_model::ProtectedString::is_sensitive)
                }
                HealthcheckCommand::Shell(text) => text.is_sensitive(),
                _ => false,
            };
            if protected {
                self.loss_with_origins(
                    format!("{field}.command"),
                    ConversionKind::Unsupported,
                    "A protected Docker healthcheck cannot be emitted to a native review artifact.",
                    command.origins(),
                )?;
                return Ok(None);
            }
        }
        if source.disabled().is_some_and(|disabled| *disabled.value()) {
            return self.map_disabled_healthcheck(source, field);
        }
        self.map_configured_healthcheck(source, field)
    }

    fn map_disabled_healthcheck(
        &mut self,
        source: &boxferry_model::Healthcheck,
        field: String,
    ) -> Result<Option<Healthcheck>, PlanError> {
        if source.command().is_some()
            || source.interval().is_some()
            || source.timeout().is_some()
            || source.retries().is_some()
            || source.start_period().is_some()
            || source.start_interval().is_some()
        {
            self.loss(
                field,
                ConversionKind::Invalid,
                "A disabled Docker healthcheck cannot also carry a command or timing fields.",
            )?;
            return Ok(None);
        }
        if let Ok(native) = Healthcheck::configured(HealthTest::Disabled, None, None, None) {
            self.exact(field);
            return Ok(Some(native));
        }
        self.loss(
            field,
            ConversionKind::Invalid,
            "DockerLens rejected the disabled healthcheck under this target contract.",
        )?;
        Ok(None)
    }

    fn map_configured_healthcheck(
        &mut self,
        source: &boxferry_model::Healthcheck,
        field: String,
    ) -> Result<Option<Healthcheck>, PlanError> {
        let mut valid = true;
        let test = match source.command().map(Sourced::value) {
            Some(HealthcheckCommand::Exec(arguments)) if !arguments.is_empty() => {
                let parsed = arguments
                    .iter()
                    .map(|argument| Argument::new(argument.expose().as_bytes().to_vec()))
                    .collect::<Result<Vec<_>, _>>();
                parsed.ok().map(HealthTest::Exec)
            }
            Some(HealthcheckCommand::Shell(command)) => Argument::new(command.expose().as_bytes().to_vec())
                .ok()
                .map(HealthTest::Shell),
            _ => None,
        };
        let Some(test) = test else {
            self.unsupported(format!("{field}.command"))?;
            return Ok(None);
        };
        let interval = source
            .interval()
            .map(|value| parse_nonzero_duration(value.value().as_str()));
        let timeout = source
            .timeout()
            .map(|value| parse_nonzero_duration(value.value().as_str()));
        let retries = source
            .retries()
            .map(|value| value.value().as_str().parse::<u32>().ok().and_then(NonZeroU32::new));
        if interval.as_ref().is_some_and(Result::is_err) {
            self.loss(
                format!("{field}.interval"),
                ConversionKind::Invalid,
                "The health interval must be a positive finite Docker duration.",
            )?;
            valid = false;
        }
        if timeout.as_ref().is_some_and(Result::is_err) {
            self.loss(
                format!("{field}.timeout"),
                ConversionKind::Invalid,
                "The health timeout must be a positive finite Docker duration.",
            )?;
            valid = false;
        }
        if retries.is_some_and(|value| value.is_none()) {
            self.loss(
                format!("{field}.retries"),
                ConversionKind::Invalid,
                "The health retry count must be a positive finite integer.",
            )?;
            valid = false;
        }
        if !valid {
            return Ok(None);
        }
        let native = Healthcheck::configured(
            test,
            interval.and_then(Result::ok),
            timeout.and_then(Result::ok),
            retries.flatten(),
        );
        let native = native.and_then(|native| {
            let native = if let Some(period) = source.start_period() {
                native.with_start_period(
                    parse_duration_ns(period.value().as_str()).map_err(|()| IntentError::InvalidHealthcheck)?,
                )?
            } else {
                native
            };
            if let Some(interval) = source.start_interval() {
                native.with_start_interval(
                    parse_duration_ns(interval.value().as_str()).map_err(|()| IntentError::InvalidHealthcheck)?,
                )
            } else {
                Ok(native)
            }
        });
        if let Ok(native) = native {
            self.exact(field);
            Ok(Some(native))
        } else {
            self.loss(
                field,
                ConversionKind::Invalid,
                "DockerLens rejected the configured healthcheck or its timing under this target contract.",
            )?;
            Ok(None)
        }
    }
}

fn parse_nonzero_duration(value: &str) -> Result<NonZeroU64, ()> {
    let nanos = parse_duration_ns(value)?;
    if nanos < 1_000_000 {
        return Err(());
    }
    NonZeroU64::new(nanos).ok_or(())
}

fn parse_duration_ns(value: &str) -> Result<u64, ()> {
    let (digits, factor): (&str, u64) = if let Some(digits) = value.strip_suffix("ns") {
        (digits, 1)
    } else if let Some(digits) = value.strip_suffix("us") {
        (digits, 1_000)
    } else if let Some(digits) = value.strip_suffix("ms") {
        (digits, 1_000_000)
    } else if let Some(digits) = value.strip_suffix('s') {
        (digits, 1_000_000_000)
    } else if let Some(digits) = value.strip_suffix('m') {
        (digits, 60_000_000_000)
    } else if let Some(digits) = value.strip_suffix('h') {
        (digits, 3_600_000_000_000)
    } else {
        return Err(());
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    let nanos = digits
        .parse::<u64>()
        .ok()
        .and_then(|count| count.checked_mul(factor))
        .ok_or(())?;
    if nanos != 0 && !(1_000_000..=i64::MAX as u64).contains(&nanos) {
        return Err(());
    }
    Ok(nanos)
}

fn parse_size_bytes(value: &str) -> Option<u64> {
    let spelling = value.to_ascii_lowercase();
    let (digits, multiplier) = if let Some(digits) = spelling.strip_suffix('k') {
        (digits, 1_024_u64)
    } else if let Some(digits) = spelling.strip_suffix('m') {
        (digits, 1_048_576)
    } else if let Some(digits) = spelling.strip_suffix('g') {
        (digits, 1_073_741_824)
    } else if let Some(digits) = spelling.strip_suffix('b') {
        (digits, 1)
    } else {
        (spelling.as_str(), 1)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u64>()
        .ok()?
        .checked_mul(multiplier)
        .filter(|bytes| i64::try_from(*bytes).is_ok())
}

/// Typed intent only; `DockerLens` independently checks native capability and host prerequisites.
fn native_bind_mount(mount: &boxferry_model::Mount) -> Result<Mount, IntentError> {
    let MountSource::HostPath(source) = mount.source() else {
        return Err(IntentError::InvalidMount);
    };
    let native = Mount::bind(
        source.as_bytes().to_vec(),
        mount.target().as_bytes().to_vec(),
        mount.read_only(),
    )?;
    match mount.selinux_relabel() {
        Some(SelinuxRelabel::Shared) => native.with_bind_relabel(BindRelabel::Shared),
        Some(SelinuxRelabel::Private) => native.with_bind_relabel(BindRelabel::Private),
        Some(_) => Err(IntentError::InvalidMount),
        None => Ok(native),
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;

#[cfg(test)]
mod destination_pids_consumer_tests {
    use super::*;
    use boxferry_engine::LossPolicy;
    use boxferry_model::{Identifier, ImageReference as NeutralImageReference, ProtectedString};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn application(pids_limit: Option<&str>) -> Result<Application, Box<dyn std::error::Error>> {
        let mut service = Service::new(Identifier::new("web")?);
        service.set_image(Sourced::generated(NeutralImageReference::parse("busybox:1")?));
        if let Some(limit) = pids_limit {
            service.set_pids_limit(Sourced::generated(ProtectedString::plain(limit)));
        }
        let mut application = Application::new(Identifier::new("example")?);
        application.add_service(Sourced::generated(service))?;
        Ok(application)
    }

    #[test]
    fn reviewed_no_pids_output_keeps_empty_destination_obligations_and_legacy_schema() -> TestResult {
        let catalogue = TargetCapabilityCatalog::reviewed();
        let mut checked = 0;
        for profile in catalogue.profiles() {
            let version = numeric_engine_release(profile).ok_or("reviewed release is not numeric")?;
            let target = TargetProfile::new(DOCKER_TARGET, version, Some(version))?;
            let planned = DockerExporter::new(profile.clone())?.plan(&application(None)?, &target)?;
            let output = planned.candidate().ok_or("reviewed no-PIDs output missing")?;
            assert!(output.destination_pids_prerequisites().is_empty());
            assert!(output.network_prerequisites().is_empty());
            assert!(output.volume_prerequisites().is_empty());
            let complete = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
            let complete = std::str::from_utf8(&complete)?;
            assert!(complete.starts_with("{\"schema_version\":1,"));
            assert!(complete.ends_with("\"prerequisites\":[]}\n"));
            assert!(planned.authorize(LossPolicy::ExactOnly).output().is_some());
            checked += 1;
        }
        assert!(checked > 0, "no reviewed native profile exercised");
        Ok(())
    }

    #[test]
    fn unadmitted_finite_and_unlimited_pids_remain_structured_rejections() -> TestResult {
        let catalogue = TargetCapabilityCatalog::reviewed();
        let mut checked = 0;
        for profile in catalogue.profiles() {
            let version = numeric_engine_release(profile).ok_or("reviewed release is not numeric")?;
            let target = TargetProfile::new(DOCKER_TARGET, version, Some(version))?;
            for limit in ["16", "-1"] {
                let planned = DockerExporter::new(profile.clone())?.plan(&application(Some(limit))?, &target)?;
                assert!(planned.candidate().is_none());
                assert!(planned.outcomes().iter().any(|outcome| {
                    outcome.subject() == "services.web" && outcome.kind() == ConversionKind::Invalid
                }));
                assert!(planned.diagnostics().iter().any(|diagnostic| {
                    diagnostic.fields().iter().any(|field| {
                        field.name() == "native_category" && field.value().expose() == "missing_capability"
                    }) && diagnostic
                        .fields()
                        .iter()
                        .any(|field| field.name() == "native_feature" && field.value().expose() == "PidsLimit")
                }));
                assert!(planned.authorize(LossPolicy::AllowPartial).output().is_none());
                checked += 1;
            }
        }
        assert!(checked > 0, "no reviewed native profile exercised");
        Ok(())
    }
}
