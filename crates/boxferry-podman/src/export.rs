//! Neutral application model to reviewable Podman deployment artifacts.

use std::{error::Error, fmt};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, NativeFinding, PlanError, PlatformVersion, RuleId, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, Entrypoint, EnvironmentValue, MountSource, ProtectedString, PullPolicy, ResourceOwnership,
    RestartPolicy, Service,
};
use podman_lens::artifact::deployment_v1;
use podman_lens::{
    AbsoluteContainerPath, ArgumentArray, ContainerHostname, ContainerIntent, ContainerUser, ContainerWorkdir,
    DeploymentEnvironmentValue, DeploymentIntent, DeploymentResource, DeploymentResourceId, EnvironmentAssignment,
    EnvironmentName, ExternalPrecondition, ImageIntent, ImagePullPolicy, ImageSource, Label, LabelKey, MountAccess,
    NamedVolumeCopyMode, NamedVolumeMount, NetworkAttachment, NetworkIntent, ObservedApiVersion, ObservedPodmanVersion,
    PlanningFinding, PodIntent, PublicEnvironmentValue, PublicLabelValue, RenderingFinding, ResourceKind, SecretIntent,
    SensitiveInputReference, StartupDependency, TargetExecutionContext, TmpfsMount, VolumeIntent, plan_deployment,
    render_deployment,
};

use crate::PodmanOutput;

/// Target implementation name accepted by the Podman exporter.
pub const PODMAN_TARGET: &str = "podman";

const REVIEWED_VERSIONS: [PlatformVersion; 7] = [
    PlatformVersion::new(5, 4, 0),
    PlatformVersion::new(5, 5, 0),
    PlatformVersion::new(5, 6, 0),
    PlatformVersion::new(5, 7, 0),
    PlatformVersion::new(5, 8, 6),
    PlatformVersion::new(6, 0, 0),
    PlatformVersion::new(6, 1, 0),
];

/// Returns exact Podman engine versions backed by `PodmanLens` rendering evidence.
#[must_use]
pub const fn reviewed_podman_versions() -> &'static [PlatformVersion] {
    &REVIEWED_VERSIONS
}

/// A `BoxFerry` target resolved to one exact reviewed Podman engine and API pair.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPodmanTarget {
    version: PlatformVersion,
    profile: podman_lens::TargetProfile,
}

impl ResolvedPodmanTarget {
    /// Returns the exact reviewed engine version selected from the requested range.
    #[must_use]
    pub const fn version(&self) -> PlatformVersion {
        self.version
    }

    /// Returns the validated `PodmanLens` engine/API profile.
    #[must_use]
    pub const fn profile(&self) -> &podman_lens::TargetProfile {
        &self.profile
    }
}

/// Failure to resolve a `BoxFerry` target to reviewed Podman rendering evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PodmanTargetError {
    /// The requested implementation was not Podman.
    ImplementationMismatch,
    /// The requested version range contains no exact reviewed renderer target.
    NoReviewedVersion,
    /// `PodmanLens` rejected the embedded reviewed engine/API pair.
    InvalidReviewedProfile(podman_lens::Diagnostic),
}

impl fmt::Display for PodmanTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImplementationMismatch => formatter.write_str("target implementation must be podman"),
            Self::NoReviewedVersion => formatter.write_str("target range contains no reviewed Podman renderer version"),
            Self::InvalidReviewedProfile(error) => {
                write!(formatter, "reviewed Podman target profile is invalid: {error}")
            }
        }
    }
}

impl Error for PodmanTargetError {}

/// Selects the newest exact reviewed Podman version in the requested inclusive range.
///
/// # Errors
///
/// Returns [`PodmanTargetError`] for another implementation, an unsupported range, or an
/// internally inconsistent embedded `PodmanLens` target pair.
pub fn resolve_podman_target(target: &TargetProfile) -> Result<ResolvedPodmanTarget, PodmanTargetError> {
    if target.implementation() != PODMAN_TARGET {
        return Err(PodmanTargetError::ImplementationMismatch);
    }
    let version = REVIEWED_VERSIONS
        .iter()
        .rev()
        .copied()
        .find(|version| target.versions().contains(*version))
        .ok_or(PodmanTargetError::NoReviewedVersion)?;
    let spelling = version.to_string();
    let podman_version = ObservedPodmanVersion::parse(&spelling).map_err(PodmanTargetError::InvalidReviewedProfile)?;
    let api_version = ObservedApiVersion::parse(&spelling).map_err(PodmanTargetError::InvalidReviewedProfile)?;
    let profile = podman_lens::TargetProfile::new(podman_version, api_version)
        .map_err(PodmanTargetError::InvalidReviewedProfile)?;
    Ok(ResolvedPodmanTarget { version, profile })
}

/// Loss-aware Podman exporter producing only review artifacts.
#[derive(Clone, Debug)]
pub struct PodmanExporter {
    codes: Codes,
    execution_context: TargetExecutionContext,
}

impl PodmanExporter {
    /// Creates an exporter without assuming rootful or rootless execution context.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only if a repository-owned diagnostic code is malformed.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            codes: Codes {
                invalid_target: RuleId::PodmanTargetInvalid.definition().diagnostic_code()?,
                unsupported: RuleId::PodmanOutputUnsupported.definition().diagnostic_code()?,
                planning: RuleId::PodmanPlanningFailed.definition().diagnostic_code()?,
            },
            execution_context: TargetExecutionContext::Unknown,
        })
    }

    /// Attaches caller-proven target privilege context used by `PodmanLens` capability checks.
    #[must_use]
    pub const fn with_execution_context(mut self, context: TargetExecutionContext) -> Self {
        self.execution_context = context;
        self
    }

    /// Returns the caller-selected target privilege context.
    #[must_use]
    pub const fn execution_context(&self) -> TargetExecutionContext {
        self.execution_context
    }
}

impl ExportAdapter for PodmanExporter {
    type Output = PodmanOutput;

    fn plan(
        &self,
        application: &Application,
        target: &TargetProfile,
    ) -> Result<ConversionPlan<Self::Output>, PlanError> {
        let resolved = match resolve_podman_target(target) {
            Ok(resolved) => resolved,
            Err(error) => {
                let diagnostic = Diagnostic::new(self.codes.invalid_target.clone(), Severity::Error, error.to_string());
                let outcome = ConversionOutcome::loss(
                    "target.podman",
                    ConversionKind::Invalid,
                    self.codes.invalid_target.clone(),
                )?;
                return ConversionPlan::new(None, vec![outcome], vec![diagnostic]);
            }
        };

        let mut profile = resolved.profile().clone();
        profile.set_execution_context(self.execution_context);
        let mut mapping = Mapping::new(self, application, DeploymentIntent::new(profile));
        mapping.map_application();
        let Some(intent) = mapping.intent.take() else {
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        };

        let planning = plan_deployment(&intent);
        if !planning.findings().is_empty() {
            for finding in planning.findings() {
                mapping.native_planning_error(finding);
            }
        }
        let Some(plan) = planning.plan() else {
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        };

        let rendering = render_deployment(plan);
        if !rendering.findings().is_empty() {
            for finding in rendering.findings() {
                mapping.native_rendering_error(finding);
            }
        }
        let Some(rendering) = rendering.rendering() else {
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        };

        let Ok(deployment_json) = serde_json::to_string_pretty(&deployment_v1::deployment(rendering)) else {
            mapping.error(
                "deployment.serialize",
                "Podman deployment artifact serialization failed",
            );
            return ConversionPlan::new(None, mapping.outcomes, mapping.diagnostics);
        };
        let output = PodmanOutput::new(resolved.version(), deployment_json, rendering.shell_script());
        mapping.exact("deployment.artifacts");
        ConversionPlan::new(Some(output), mapping.outcomes, mapping.diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_target: DiagnosticCode,
    unsupported: DiagnosticCode,
    planning: DiagnosticCode,
}

struct Mapping<'a> {
    exporter: &'a PodmanExporter,
    application: &'a Application,
    intent: Option<DeploymentIntent>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Mapping<'a> {
    const fn new(exporter: &'a PodmanExporter, application: &'a Application, intent: DeploymentIntent) -> Self {
        Self {
            exporter,
            application,
            intent: Some(intent),
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn map_application(&mut self) {
        self.exact("application");
        self.map_networks();
        self.map_volumes();
        self.map_secrets();
        self.map_groups();
        self.map_services();
        self.map_dependencies();
    }

    fn map_networks(&mut self) {
        for network in self.application.networks() {
            let network = network.value();
            let subject = format!("networks.{}", network.name().as_str());
            let result = self
                .resource_id(ResourceKind::Network, network.name().as_str())
                .and_then(|id| match network.ownership() {
                    ResourceOwnership::Application => NetworkIntent::new(id).map(DeploymentResource::Network),
                    ResourceOwnership::External | ResourceOwnership::Implicit | ResourceOwnership::Uncertain => {
                        ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition)
                    }
                    _ => ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition),
                });
            self.add_resource_result(&subject, result);
            if network.runtime_name().is_some()
                || network.driver().is_some()
                || network.driver_options().is_some()
                || network.labels().is_some()
                || network.internal().is_some()
                || network.ipv6().is_some()
                || network.ipam_driver().is_some()
                || network.ipam_configs().is_some()
            {
                self.unsupported(
                    format!("{subject}.settings"),
                    "PodmanLens network intent does not yet expose all neutral network settings",
                );
            }
        }
    }

    fn map_volumes(&mut self) {
        for volume in self.application.volumes() {
            let volume = volume.value();
            let subject = format!("volumes.{}", volume.name().as_str());
            let result = self
                .resource_id(ResourceKind::Volume, volume.name().as_str())
                .and_then(|id| match volume.ownership() {
                    ResourceOwnership::Application => VolumeIntent::new(id).map(DeploymentResource::Volume),
                    ResourceOwnership::External | ResourceOwnership::Implicit | ResourceOwnership::Uncertain => {
                        ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition)
                    }
                    _ => ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition),
                });
            self.add_resource_result(&subject, result);
            if volume.runtime_name().is_some()
                || volume.driver().is_some()
                || volume.device().is_some()
                || volume.volume_type().is_some()
                || volume.options().is_some()
                || volume.labels().is_some()
                || volume.user().is_some()
                || volume.group().is_some()
                || volume.uid().is_some()
                || volume.gid().is_some()
                || volume.image_source().is_some()
            {
                self.unsupported(
                    format!("{subject}.settings"),
                    "PodmanLens volume intent does not yet expose all neutral volume settings",
                );
            }
        }
    }

    fn map_secrets(&mut self) {
        for secret in self.application.secrets() {
            let secret = secret.value();
            let subject = format!("secrets.{}", secret.name().as_str());
            let result = self
                .resource_id(ResourceKind::Secret, secret.name().as_str())
                .and_then(|id| match secret.ownership() {
                    ResourceOwnership::Application => {
                        let reference =
                            SensitiveInputReference::new(format!("boxferry-secret-{}", secret.name().as_str()))?;
                        SecretIntent::new(id, reference).map(DeploymentResource::Secret)
                    }
                    ResourceOwnership::External | ResourceOwnership::Implicit | ResourceOwnership::Uncertain => {
                        ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition)
                    }
                    _ => ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition),
                });
            self.add_resource_result(&subject, result);
            if secret.runtime_name().is_some() {
                self.unsupported(
                    format!("{subject}.runtime_name"),
                    "runtime secret names are not yet represented by PodmanLens deployment identities",
                );
            }
            if secret.ownership() == ResourceOwnership::Application && secret.material().is_none() {
                self.unsupported(
                    format!("{subject}.material"),
                    "application-managed secret requires caller-supplied deployment material",
                );
            }
        }
    }

    fn map_groups(&mut self) {
        for group in self.application.service_groups() {
            let group = group.value();
            let subject = format!("groups.{}", group.name().as_str());
            if group.ownership() != ResourceOwnership::Application {
                self.unsupported(
                    format!("{subject}.ownership"),
                    "runtime-imported or externally owned pod topology is not a Podman external precondition",
                );
                self.unsupported(
                    format!("{subject}.members"),
                    "runtime-imported pod membership is omitted unless pod lifecycle ownership is authored",
                );
                continue;
            }
            let result = self
                .resource_id(ResourceKind::Pod, group.name().as_str())
                .and_then(|id| match group.ownership() {
                    ResourceOwnership::Application => {
                        let mut pod = PodIntent::new(id)?;
                        for member in group.members() {
                            pod.add_member(self.resource_id(ResourceKind::Container, member.value().as_str())?)?;
                        }
                        Ok(DeploymentResource::Pod(pod))
                    }
                    _ => ExternalPrecondition::new(id).map(DeploymentResource::ExternalPrecondition),
                });
            self.add_resource_result(&subject, result);
            if group.runtime().is_some() {
                self.unsupported(
                    format!("{subject}.runtime"),
                    "PodmanLens pod intent does not yet expose neutral group runtime settings",
                );
            }
        }
    }

    fn map_services(&mut self) {
        for service in self.application.services() {
            self.map_service(service.value());
        }
    }

    fn map_service(&mut self, service: &Service) {
        let subject = format!("services.{}", service.name().as_str());
        let Some(image) = service.image() else {
            self.error(
                format!("{subject}.image"),
                "Podman container export requires an image reference",
            );
            return;
        };
        let image_name = format!("{}-image", service.name().as_str());
        let image_result = (|| {
            let id = self.resource_id(ResourceKind::Image, &image_name)?;
            let source = ImageSource::new(image.value().as_str())?;
            let policy = match service.pull_policy().map(boxferry_model::Sourced::value) {
                Some(PullPolicy::Always) => ImagePullPolicy::Always,
                Some(PullPolicy::Never) => ImagePullPolicy::Never,
                Some(PullPolicy::Missing) | None => ImagePullPolicy::Missing,
                Some(_) => {
                    self.unsupported(
                        format!("{subject}.pull_policy"),
                        "future neutral pull policy is not reviewed for Podman",
                    );
                    ImagePullPolicy::Missing
                }
            };
            ImageIntent::new(id, source, policy).map(DeploymentResource::Image)
        })();
        self.add_resource_result(&format!("{subject}.image"), image_result);

        let container_result = (|| {
            let container_id = self.resource_id(ResourceKind::Container, service.name().as_str())?;
            let image_id = self.resource_id(ResourceKind::Image, &image_name)?;
            let mut container = ContainerIntent::new(container_id, image_id)?;
            if let Some(group) = self.group_for(service.name().as_str()) {
                container.set_pod(self.resource_id(ResourceKind::Pod, group)?)?;
            }
            self.map_service_settings(service, &mut container)?;
            self.map_service_mounts(service, &mut container)?;
            self.map_service_networks(service, &mut container);
            Ok(DeploymentResource::Container(container))
        })();
        self.add_resource_result(&subject, container_result);
        self.report_unmapped_service_fields(service, &subject);
    }

    #[allow(clippy::too_many_lines, reason = "keeps one container settings transaction atomic")]
    fn map_service_settings(
        &mut self,
        service: &Service,
        container: &mut ContainerIntent,
    ) -> podman_lens::PodmanLensResult<()> {
        if let Some(command) = service.command() {
            match command.value() {
                Command::Exec(arguments) => container
                    .settings_mut()
                    .set_command(ArgumentArray::new(expose(arguments))?)?,
                Command::Shell(command) => {
                    container
                        .settings_mut()
                        .set_command(ArgumentArray::new(["/bin/sh", "-c", command.expose()])?)?;
                }
                Command::Empty => self.unsupported(
                    format!("services.{}.command", service.name().as_str()),
                    "PodmanLens requires nonempty command arrays",
                ),
                _ => self.unsupported(
                    format!("services.{}.command", service.name().as_str()),
                    "future neutral command variant is not reviewed for Podman",
                ),
            }
        }
        if let Some(entrypoint) = service.entrypoint() {
            match entrypoint.value() {
                Entrypoint::Exec(arguments) => container
                    .settings_mut()
                    .set_entrypoint(ArgumentArray::new(expose(arguments))?)?,
                Entrypoint::Shell(command) => {
                    container.settings_mut().set_entrypoint(ArgumentArray::new([
                        "/bin/sh",
                        "-c",
                        command.expose(),
                    ])?)?;
                }
                Entrypoint::Empty => self.unsupported(
                    format!("services.{}.entrypoint", service.name().as_str()),
                    "PodmanLens requires nonempty entrypoint arrays",
                ),
                _ => self.unsupported(
                    format!("services.{}.entrypoint", service.name().as_str()),
                    "future neutral entrypoint variant is not reviewed for Podman",
                ),
            }
        }
        if let Some(user) = service.user() {
            let spelling = match service.group() {
                Some(group) => format!("{}:{}", user.value().expose(), group.value().expose()),
                None => user.value().expose().to_owned(),
            };
            container.settings_mut().set_user(ContainerUser::new(spelling)?)?;
        } else if service.group().is_some() {
            self.unsupported(
                format!("services.{}.group", service.name().as_str()),
                "Podman user syntax cannot express a group without a user",
            );
        }
        if let Some(workdir) = service.working_directory() {
            let path = AbsoluteContainerPath::new(workdir.value().expose())?;
            container.settings_mut().set_workdir(ContainerWorkdir::new(path))?;
        }
        if let Some(hostname) = service.hostname() {
            container
                .settings_mut()
                .set_hostname(ContainerHostname::new(hostname.value().expose())?)?;
        }
        for label in service.labels() {
            container.settings_mut().add_label(Label::new(
                LabelKey::new(label.value().name().as_str())?,
                PublicLabelValue::new(label.value().value().expose())?,
            ))?;
        }
        for environment in service.environment() {
            let value = match environment.value().value() {
                EnvironmentValue::Literal(value) if !value.is_sensitive() => {
                    DeploymentEnvironmentValue::Public(PublicEnvironmentValue::new(value.expose())?)
                }
                EnvironmentValue::Literal(_) | EnvironmentValue::Host | EnvironmentValue::Unset => {
                    self.unsupported(format!("services.{}.environment.{}", service.name().as_str(), environment.value().name().as_str()), "environment value requires external or sensitive-input planning not available from the neutral field");
                    continue;
                }
                _ => {
                    self.unsupported(
                        format!("services.{}.environment", service.name().as_str()),
                        "future neutral environment value is not reviewed for Podman",
                    );
                    continue;
                }
            };
            container.settings_mut().add_environment(EnvironmentAssignment::new(
                EnvironmentName::new(environment.value().name().as_str())?,
                value,
            ))?;
        }
        if let Some(restart) = service.restart_policy() {
            let policy = match restart.value() {
                RestartPolicy::Never => podman_lens::RestartPolicy::No,
                RestartPolicy::Always => podman_lens::RestartPolicy::Always,
                RestartPolicy::OnFailure { maximum_retries: None } => podman_lens::RestartPolicy::OnFailure,
                RestartPolicy::UnlessStopped => podman_lens::RestartPolicy::UnlessStopped,
                RestartPolicy::OnFailure {
                    maximum_retries: Some(_),
                } => {
                    self.unsupported(
                        format!("services.{}.restart_policy", service.name().as_str()),
                        "PodmanLens restart intent does not retain finite retry count",
                    );
                    podman_lens::RestartPolicy::OnFailure
                }
                _ => {
                    self.unsupported(
                        format!("services.{}.restart_policy", service.name().as_str()),
                        "future neutral restart policy is not reviewed for Podman",
                    );
                    podman_lens::RestartPolicy::No
                }
            };
            container.settings_mut().set_restart_policy(policy)?;
        }
        Ok(())
    }

    fn map_service_mounts(
        &mut self,
        service: &Service,
        container: &mut ContainerIntent,
    ) -> podman_lens::PodmanLensResult<()> {
        for (index, mount) in service.mounts().iter().enumerate() {
            let access = if mount.value().read_only() {
                MountAccess::ReadOnly
            } else {
                MountAccess::ReadWrite
            };
            let destination = AbsoluteContainerPath::new(mount.value().target())?;
            match mount.value().source() {
                MountSource::Volume(name) => {
                    let source = self.resource_id(ResourceKind::Volume, name.as_str())?;
                    container.add_mount(NamedVolumeMount::new(
                        source,
                        destination,
                        access,
                        NamedVolumeCopyMode::Copy,
                    )?);
                }
                MountSource::HostPath(_) | MountSource::Anonymous => self.unsupported(
                    format!("services.{}.mounts[{index}]", service.name().as_str()),
                    "host and anonymous mount resolution requires explicit target-side policy",
                ),
                _ => self.unsupported(
                    format!("services.{}.mounts[{index}]", service.name().as_str()),
                    "future neutral mount source is not reviewed for Podman",
                ),
            }
            if mount.value().selinux_relabel().is_some() {
                self.unsupported(
                    format!("services.{}.mounts[{index}].selinux", service.name().as_str()),
                    "PodmanLens named-volume intent does not expose SELinux relabel mode",
                );
            }
        }
        if let Some(tmpfs_mounts) = service.tmpfs() {
            if tmpfs_mounts.is_empty() {
                self.exact(format!("services.{}.tmpfs", service.name().as_str()));
            }
            for (index, tmpfs) in tmpfs_mounts.iter().enumerate() {
                let subject = format!("services.{}.tmpfs[{index}]", service.name().as_str());
                if tmpfs.value().is_sensitive() {
                    self.error(
                        subject,
                        "sensitive tmpfs destination cannot be rendered without exposing protected input",
                    );
                    continue;
                }
                let spelling = tmpfs.value().expose();
                let (destination, options) = spelling
                    .split_once(':')
                    .map_or((spelling, None), |(destination, options)| (destination, Some(options)));
                let destination = AbsoluteContainerPath::new(destination)?;
                container.add_mount(TmpfsMount::new(destination, MountAccess::ReadWrite));
                if options.is_some() {
                    self.unsupported(
                        format!("{subject}.options"),
                        "tmpfs options are not represented by the current PodmanLens tmpfs intent",
                    );
                } else {
                    self.exact(subject);
                }
            }
        }

        Ok(())
    }

    fn map_service_networks(&mut self, service: &Service, container: &mut ContainerIntent) {
        for (index, network) in service.networks().iter().enumerate() {
            let subject = format!("services.{}.networks[{index}]", service.name().as_str());
            let attachment = self
                .resource_id(ResourceKind::Network, network.value().network().as_str())
                .and_then(NetworkAttachment::new);
            let mut attachment = match attachment {
                Ok(attachment) => attachment,
                Err(error) => {
                    self.native_error(&subject, error.code().as_str(), error.message(), "mapping");
                    continue;
                }
            };
            let mut aliases_valid = true;
            for (alias_index, alias) in network.value().aliases().iter().enumerate() {
                if let Err(error) = attachment.add_alias(alias) {
                    self.native_error(
                        &format!("{subject}.aliases[{alias_index}]"),
                        error.code().as_str(),
                        error.message(),
                        "mapping",
                    );
                    aliases_valid = false;
                    break;
                }
            }
            if !aliases_valid {
                continue;
            }
            if network.value().ipv4_address().is_some() || network.value().ipv6_address().is_some() {
                self.unsupported(
                    format!("services.{}.networks[{index}].addresses", service.name().as_str()),
                    "static neutral network addresses are not yet mapped to PodmanLens attachment intent",
                );
            }
            if let Err(error) = container.add_network(attachment) {
                self.native_error(&subject, error.code().as_str(), error.message(), "mapping");
            }
        }
    }

    fn map_dependencies(&mut self) {
        for service in self.application.services() {
            for (index, dependency) in service.value().dependencies().iter().enumerate() {
                let result = (|| {
                    let predecessor =
                        self.resource_id(ResourceKind::Container, dependency.value().service().as_str())?;
                    let dependent = self.resource_id(ResourceKind::Container, service.value().name().as_str())?;
                    StartupDependency::new(predecessor, dependent)
                })();
                match result {
                    Ok(edge) => {
                        if let Some(intent) = &mut self.intent {
                            intent.add_startup_dependency(edge);
                        }
                        self.exact(format!(
                            "services.{}.dependencies[{index}]",
                            service.value().name().as_str()
                        ));
                    }
                    Err(error) => self.native_error(
                        &format!("services.{}.dependencies[{index}]", service.value().name().as_str()),
                        error.code().as_str(),
                        error.message(),
                        "mapping",
                    ),
                }
                if dependency.value().condition().is_some()
                    || dependency.value().restart().is_some()
                    || dependency.value().required().is_some()
                {
                    self.unsupported(
                        format!(
                            "services.{}.dependencies[{index}].options",
                            service.value().name().as_str()
                        ),
                        "Podman startup edges do not represent Compose dependency options",
                    );
                }
            }
        }
    }

    fn report_unmapped_service_fields(&mut self, service: &Service, subject: &str) {
        for (field, present) in [
            ("runtime_name", service.runtime_name().is_some()),
            ("rootfs", service.rootfs().is_some()),
            ("image_acquisition", service.image_acquisition().is_some()),
            ("image_build", service.image_build().is_some()),
            ("startup_notification", service.startup_notification().is_some()),
            ("run_init", service.run_init().is_some()),
            ("stop_timeout", service.stop_timeout().is_some()),
            ("memory_limit", service.memory_limit().is_some()),
            ("exposed_ports", service.exposed_ports().is_some()),
            ("healthcheck", service.healthcheck().is_some()),
            ("annotations", service.annotations().is_some()),
            ("logging", service.logging().is_some()),
            ("reload_action", service.reload_action().is_some()),
            ("user_namespace", service.user_namespace().is_some()),
            ("supplementary_groups", !service.supplementary_groups().is_empty()),
            (
                "read_only_root_filesystem",
                service.read_only_root_filesystem().is_some(),
            ),
            ("dns_servers", service.dns_servers().is_some()),
            ("dns_options", service.dns_options().is_some()),
            ("dns_search_domains", service.dns_search_domains().is_some()),
            ("security_options", service.security_options().is_some()),
            ("pids_limit", service.pids_limit().is_some()),
            ("shm_size", service.shm_size().is_some()),
            ("cap_add", service.cap_add().is_some()),
            ("cap_drop", service.cap_drop().is_some()),
            ("sysctls", service.sysctls().is_some()),
            ("ulimits", service.ulimits().is_some()),
            ("devices", service.devices().is_some()),
            ("stop_signal", service.stop_signal().is_some()),
            ("podman_args", service.podman_args().is_some()),
            ("environment_files", !service.environment_files().is_empty()),
            ("host_mappings", !service.host_mappings().is_empty()),
            ("ports", !service.ports().is_empty()),
            ("config_grants", !service.config_grants().is_empty()),
            ("secret_grants", !service.secret_grants().is_empty()),
        ] {
            if present {
                self.unsupported(
                    format!("{subject}.{field}"),
                    "neutral service field has no reviewed PodmanLens mapping yet",
                );
            }
        }
    }

    fn group_for(&self, service: &str) -> Option<&str> {
        self.application.service_groups().iter().find_map(|group| {
            (group.value().ownership() == ResourceOwnership::Application)
                .then_some(group.value())
                .and_then(|group| {
                    group
                        .members()
                        .iter()
                        .any(|member| member.value().as_str() == service)
                        .then(|| group.name().as_str())
                })
        })
    }

    #[allow(
        clippy::unused_self,
        reason = "keeps resource validation local to one mapping instance"
    )]
    fn resource_id(&self, kind: ResourceKind, name: &str) -> podman_lens::PodmanLensResult<DeploymentResourceId> {
        DeploymentResourceId::new(kind, name)
    }

    fn add_resource_result(&mut self, subject: &str, result: podman_lens::PodmanLensResult<DeploymentResource>) {
        match result {
            Ok(resource) => {
                if let Some(intent) = &mut self.intent {
                    intent.add_resource(resource);
                }
                self.exact(subject);
            }
            Err(error) => self.native_error(subject, error.code().as_str(), error.message(), "mapping"),
        }
    }

    fn exact(&mut self, subject: impl Into<String>) {
        self.outcomes.push(ConversionOutcome::exact(subject));
    }

    fn unsupported(&mut self, subject: impl Into<String>, summary: impl Into<String>) {
        let subject = subject.into();
        self.diagnostics.push(Diagnostic::new(
            self.exporter.codes.unsupported.clone(),
            Severity::Warning,
            summary,
        ));
        if let Ok(outcome) = ConversionOutcome::loss(
            subject,
            ConversionKind::Unsupported,
            self.exporter.codes.unsupported.clone(),
        ) {
            self.outcomes.push(outcome);
        } else {
            self.intent = None;
        }
    }

    fn error(&mut self, subject: impl Into<String>, summary: impl Into<String>) {
        let subject = subject.into();
        self.diagnostics.push(Diagnostic::new(
            self.exporter.codes.planning.clone(),
            Severity::Error,
            summary,
        ));
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.exporter.codes.planning.clone())
        {
            self.outcomes.push(outcome);
        }
        self.intent = None;
    }

    fn native_error(&mut self, subject: &str, native_code: &str, summary: &str, stage: &'static str) {
        let subject_field = DiagnosticField::new("subject", DiagnosticValue::plain(subject));
        let native = NativeFinding::new("podman", "podman-lens", native_code, stage, Severity::Error, summary)
            .with_field(subject_field.clone());
        self.diagnostics.push(
            Diagnostic::new(
                self.exporter.codes.planning.clone(),
                Severity::Error,
                "PodmanLens could not map, plan, or render deployment intent",
            )
            .with_field(subject_field)
            .with_native_finding(native),
        );
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.exporter.codes.planning.clone())
        {
            self.outcomes.push(outcome);
        }
        self.intent = None;
    }

    fn native_planning_error(&mut self, finding: &PlanningFinding) {
        let subject = finding.subject().map_or_else(
            || "deployment.plan".to_owned(),
            |resource| format!("{}.{}", resource_kind_name(resource.kind()), resource.name()),
        );
        let mut fields = vec![DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone()))];
        if let Some(resource) = finding.subject() {
            fields.push(DiagnosticField::new(
                "resource_kind",
                DiagnosticValue::plain(resource_kind_name(resource.kind())),
            ));
            fields.push(DiagnosticField::new(
                "resource_name",
                DiagnosticValue::plain(resource.name()),
            ));
        }
        if let Some(field) = finding.field() {
            fields.push(DiagnosticField::new("intent_field", DiagnosticValue::plain(field)));
        }
        if !finding.related().is_empty() {
            fields.push(DiagnosticField::new(
                "related_resources",
                DiagnosticValue::plain(
                    finding
                        .related()
                        .iter()
                        .map(|resource| format!("{}:{}", resource_kind_name(resource.kind()), resource.name()))
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            ));
        }
        if let Some(occurrence) = finding.occurrence() {
            fields.push(DiagnosticField::new(
                "occurrence",
                DiagnosticValue::plain(occurrence.to_string()),
            ));
        }
        if let Some(count) = finding.count() {
            fields.push(DiagnosticField::new("count", DiagnosticValue::plain(count.to_string())));
        }

        let mut native = NativeFinding::new(
            "podman",
            "podman-lens",
            finding.code().as_str(),
            "planning",
            Severity::Error,
            finding.message(),
        );
        let mut diagnostic = Diagnostic::new(
            self.exporter.codes.planning.clone(),
            Severity::Error,
            "PodmanLens could not map, plan, or render deployment intent",
        );
        for field in fields {
            native = native.with_field(field.clone());
            diagnostic = diagnostic.with_field(field);
        }
        self.diagnostics.push(diagnostic.with_native_finding(native));
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.exporter.codes.planning.clone())
        {
            self.outcomes.push(outcome);
        }
        self.intent = None;
    }

    fn native_rendering_error(&mut self, finding: &RenderingFinding) {
        let subject = finding.subject().map_or_else(
            || "deployment.render".to_owned(),
            |resource| format!("{}.{}", resource_kind_name(resource.kind()), resource.name()),
        );
        let mut fields = vec![DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone()))];
        if let Some(resource) = finding.subject() {
            fields.push(DiagnosticField::new(
                "resource_kind",
                DiagnosticValue::plain(resource_kind_name(resource.kind())),
            ));
            fields.push(DiagnosticField::new(
                "resource_name",
                DiagnosticValue::plain(resource.name()),
            ));
        }
        if let Some(field) = finding.field() {
            fields.push(DiagnosticField::new("intent_field", DiagnosticValue::plain(field)));
        }

        let mut native = NativeFinding::new(
            "podman",
            "podman-lens",
            finding.code().as_str(),
            "rendering",
            Severity::Error,
            finding.message(),
        );
        let mut diagnostic = Diagnostic::new(
            self.exporter.codes.planning.clone(),
            Severity::Error,
            "PodmanLens could not map, plan, or render deployment intent",
        );
        for field in fields {
            native = native.with_field(field.clone());
            diagnostic = diagnostic.with_field(field);
        }
        self.diagnostics.push(diagnostic.with_native_finding(native));
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.exporter.codes.planning.clone())
        {
            self.outcomes.push(outcome);
        }
        self.intent = None;
    }
}

const fn resource_kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Container => "container",
        ResourceKind::Pod => "pod",
        ResourceKind::Network => "network",
        ResourceKind::Volume => "volume",
        ResourceKind::Image => "image",
        ResourceKind::Secret => "secret",
        _ => "resource",
    }
}

fn expose(values: &[ProtectedString]) -> Vec<&str> {
    values.iter().map(ProtectedString::expose).collect()
}
