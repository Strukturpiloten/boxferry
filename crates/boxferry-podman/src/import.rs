//! `PodmanLens` observations into the neutral application model.

use std::collections::{BTreeMap, BTreeSet};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, ImportAdapter, ImportResult, InvalidDiagnosticCode,
    NativeFinding, Severity,
};
use boxferry_model::{
    Application, Command, Entrypoint, Identifier, ImageReference, MetadataLabel, ModelError, Mount, MountSource,
    Network, NetworkAttachment as NeutralNetworkAttachment, ProtectedString, Provenance, ResourceOwnership, Secret,
    Service, ServiceDependency, ServiceGroup, SourceId, Sourced, Volume,
};
use podman_lens::{
    ContainerMountKind, ContainerMountObservation, ContainerMountSource, ContainerObservation,
    ContainerSecretGrantObservation, NativeNetworkingObservation, ObservationField, ObservationOrigin, ResourceDetails,
    ResourceIdentity, ResourceKind, ResourceObservation, ResourceObservationState,
};

use crate::PodmanSource;

/// PodmanLens-to-neutral-model semantic importer.
#[derive(Clone, Debug)]
pub struct PodmanImporter {
    unsupported: DiagnosticCode,
    invalid: DiagnosticCode,
    policy: DiagnosticCode,
    identity: DiagnosticCode,
    secret: DiagnosticCode,
}

impl PodmanImporter {
    /// Creates the importer and validates its stable Podman adapter diagnostic codes.
    ///
    /// # Errors
    ///
    /// Returns an error only if this crate's static codes are malformed.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            unsupported: DiagnosticCode::new("BFP0002")?,
            invalid: DiagnosticCode::new("BFP0001")?,
            policy: DiagnosticCode::new("BFP0003")?,
            identity: DiagnosticCode::new("BFP0004")?,
            secret: DiagnosticCode::new("BFP0005")?,
        })
    }
}

impl ImportAdapter for PodmanImporter {
    type Source = PodmanSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let Ok(source_id) = SourceId::new("podman") else {
            return ImportResult::new(
                None,
                Vec::new(),
                vec![Diagnostic::new(
                    self.invalid.clone(),
                    Severity::Error,
                    "Podman provenance identity is invalid",
                )],
            );
        };
        Mapping::new(self, source, source_id).map()
    }
}

struct Mapping<'a> {
    importer: &'a PodmanImporter,
    source: &'a PodmanSource,
    origin: Provenance,
    selected: BTreeSet<ResourceIdentity>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
    service_names: BTreeMap<String, Identifier>,
    pod_memberships: BTreeMap<String, Identifier>,
}

impl<'a> Mapping<'a> {
    fn new(importer: &'a PodmanImporter, source: &'a PodmanSource, source_id: SourceId) -> Self {
        Self {
            importer,
            source,
            origin: Provenance::runtime_observation(source_id),
            selected: selected_identities(source),
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
            service_names: BTreeMap::new(),
            pod_memberships: BTreeMap::new(),
        }
    }

    fn map(mut self) -> ImportResult {
        let mut application = Application::new(self.source.application_name().clone());
        self.report_missing_selected_observations();
        self.exact("application");

        for observation in observations(self.source, &self.selected, ResourceKind::Network) {
            self.map_network(&mut application, observation);
        }
        for observation in observations(self.source, &self.selected, ResourceKind::Volume) {
            self.map_volume(&mut application, observation);
        }
        for observation in observations(self.source, &self.selected, ResourceKind::Secret) {
            self.map_secret(&mut application, observation);
        }
        for observation in observations(self.source, &self.selected, ResourceKind::Container) {
            self.map_container(&mut application, observation);
        }
        for observation in observations(self.source, &self.selected, ResourceKind::Pod) {
            self.map_pod(&mut application, observation);
        }
        for observation in observations(self.source, &self.selected, ResourceKind::Image) {
            self.map_image(observation);
        }

        self.report_native_findings();
        ImportResult::new(Some(application), self.outcomes, self.diagnostics)
    }

    fn map_network(&mut self, application: &mut Application, observation: &ResourceObservation) {
        let subject = format!("networks.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Network(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        let Some(name) = self.identifier(&subject, observation.header().identity()) else {
            return;
        };
        let mut network = Network::new(name, ResourceOwnership::Uncertain);
        self.map_network_labels(&subject, details.labels(), &mut network);
        self.observation_only(
            &format!("{subject}.internal"),
            details.internal(),
            "effective network-internal state needs explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.ipam"),
            details.subnets(),
            "effective network IPAM needs explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.routes"),
            details.routes(),
            "native network routes have no neutral desired-state field",
        );
        self.observation_only(
            &format!("{subject}.options"),
            details.options(),
            "network option values are intentionally not exposed by PodmanLens",
        );
        self.exact(&subject);
        if let Err(error) = application.add_network(self.sourced(network)) {
            self.model_error(subject, &error);
        }
    }

    fn map_volume(&mut self, application: &mut Application, observation: &ResourceObservation) {
        let subject = format!("volumes.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Volume(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        let Some(name) = self.identifier(&subject, observation.header().identity()) else {
            return;
        };
        let mut volume = Volume::new(name, ResourceOwnership::Uncertain);
        self.map_volume_labels(&subject, details.labels(), &mut volume);
        self.observation_only(
            &format!("{subject}.driver"),
            details.driver(),
            "effective volume driver is observation-only",
        );
        self.observation_only(
            &format!("{subject}.uid"),
            details.uid(),
            "effective volume ownership is observation-only",
        );
        self.observation_only(
            &format!("{subject}.gid"),
            details.gid(),
            "effective volume ownership is observation-only",
        );
        self.observation_only(
            &format!("{subject}.created_at"),
            details.created_at(),
            "volume creation time is runtime evidence only",
        );
        self.observation_only(
            &format!("{subject}.anonymous"),
            details.anonymous(),
            "anonymous-volume classification is runtime evidence only",
        );
        self.exact(&subject);
        if let Err(error) = application.add_volume(self.sourced(volume)) {
            self.model_error(subject, &error);
        }
    }

    fn map_secret(&mut self, application: &mut Application, observation: &ResourceObservation) {
        let subject = format!("secrets.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Secret(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        let Some(name) = self.identifier(&subject, observation.header().identity()) else {
            return;
        };
        let secret = Secret::new(name, ResourceOwnership::Uncertain);
        self.observation_only(
            &format!("{subject}.driver"),
            details.driver(),
            "secret metadata does not contain deployable secret material",
        );
        self.observation_only(
            &format!("{subject}.labels"),
            details.labels(),
            "neutral secret metadata has no label field",
        );
        self.observation_only(
            &format!("{subject}.created_at"),
            details.created_at(),
            "secret creation time is runtime evidence only",
        );
        self.observation_only(
            &format!("{subject}.updated_at"),
            details.updated_at(),
            "secret update time is runtime evidence only",
        );
        self.exact(&subject);
        if let Err(error) = application.add_secret(self.sourced(secret)) {
            self.model_error(subject, &error);
        }
    }

    fn map_container(&mut self, application: &mut Application, observation: &ResourceObservation) {
        let subject = format!("services.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Container(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        let is_infra = details.infra().observed().is_some_and(|value| *value.value());
        self.observation_only(
            &format!("{subject}.infra"),
            details.infra(),
            "infra-container classification is runtime topology evidence",
        );
        if is_infra {
            self.exact(format!("{subject}.pod_infra"));
            self.report_infra_container_fields(&subject, details);
            return;
        }
        let Some(name) = self.identifier(&subject, observation.header().identity()) else {
            return;
        };
        let mut service = Service::new(name.clone());
        self.service_names
            .insert(observation.header().identity().id().to_owned(), name.clone());
        if let Some(native_name) = observation.header().identity().name() {
            self.service_names.insert(native_name.to_owned(), name);
        }
        self.map_pod_membership(&subject, observation.header().identity(), details);

        self.map_configured_image(&subject, details, &mut service);
        self.map_configured_settings(&subject, details, &mut service);
        self.map_container_labels(&subject, details.labels(), &mut service);
        self.map_mounts(&subject, details, &mut service);
        self.map_container_networks(&subject, details, &mut service);
        self.map_dependencies(&subject, details, &mut service);
        self.observation_only(
            &format!("{subject}.environment"),
            details.environment(),
            "protected environment values require caller authorization",
        );
        self.report_secret_grants(&format!("{subject}.secret_grants"), details.secret_grants());
        self.observation_only(
            &format!("{subject}.memory_swappiness"),
            details.memory_swappiness(),
            "configured memory swappiness has no neutral field",
        );
        self.observation_only(
            &format!("{subject}.restart_policy"),
            details.restart_policy(),
            "effective restart policy needs explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.healthcheck"),
            details.health_check(),
            "effective health configuration needs explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.health_failure_action"),
            details.health_failure_action(),
            "effective health failure action has no complete neutral equivalent",
        );
        self.observation_only(
            &format!("{subject}.startup_healthcheck"),
            details.startup_health_check(),
            "startup health has no complete neutral equivalent",
        );
        self.observation_only(
            &format!("{subject}.logging"),
            details.logging(),
            "effective logging needs explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.security"),
            details.security(),
            "effective security settings need explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.namespaces"),
            details.namespaces(),
            "effective namespace settings need explicit promotion policy",
        );
        self.observation_only(
            &format!("{subject}.resource_controls"),
            details.resource_controls(),
            "effective resource controls need explicit promotion policy",
        );

        self.exact(&subject);
        if let Err(error) = application.add_service(self.sourced(service)) {
            self.model_error(subject, &error);
        }
    }

    fn map_pod(&mut self, application: &mut Application, observation: &ResourceObservation) {
        let subject = format!("service_groups.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Pod(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        let Some(name) = self.identifier(&subject, observation.header().identity()) else {
            return;
        };
        let mut group = ServiceGroup::new(name.clone(), ResourceOwnership::Uncertain);
        for container in observations(self.source, &self.selected, ResourceKind::Container) {
            if self.pod_memberships.get(container.header().identity().id()) != Some(&name) {
                continue;
            }
            if let Some(member) = self.service_names.get(container.header().identity().id()).cloned() {
                if let Err(error) = group.add_member(self.sourced(member)) {
                    self.model_error(format!("{subject}.members"), &error);
                } else {
                    self.exact(format!("{subject}.members"));
                }
            }
        }
        self.report_networking_only(&format!("{subject}.runtime"), details.networking());
        self.observation_only(
            &format!("{subject}.create_infra"),
            details.create_infra(),
            "Podman infra-container creation is native runtime evidence",
        );
        self.observation_only(
            &format!("{subject}.labels"),
            details.labels(),
            "neutral service groups have no metadata-label field",
        );
        self.exact(&subject);
        if let Err(error) = application.add_service_group(self.sourced(group)) {
            self.model_error(subject, &error);
        }
    }

    fn map_image(&mut self, observation: &ResourceObservation) {
        let subject = format!("images.{}", identity_name(observation.header().identity()));
        if !self.require_complete(observation, &subject) {
            return;
        }
        let ResourceDetails::Image(details) = observation.details() else {
            self.invalid(subject, "native resource kind and details disagree");
            return;
        };
        self.observation_only(
            &format!("{subject}.labels"),
            details.labels(),
            "runtime image labels are evidence, not authored acquisition",
        );
        self.observation_only(
            &format!("{subject}.repo_tags"),
            details.repo_tags(),
            "local repository tags must not replace configured image intent",
        );
        self.observation_only(
            &format!("{subject}.repo_digests"),
            details.repo_digests(),
            "local repository digests must not replace configured image intent",
        );
        self.observation_only(
            &format!("{subject}.environment"),
            details.environment(),
            "protected image defaults are observation-only",
        );
        self.observation_only(
            &format!("{subject}.digest"),
            details.digest(),
            "local image digest is resolution evidence only",
        );
        self.observation_only(
            &format!("{subject}.created"),
            details.created(),
            "image creation time is runtime evidence only",
        );
        self.observation_only(
            &format!("{subject}.author"),
            details.author(),
            "image author is metadata evidence only",
        );
        self.observation_only(
            &format!("{subject}.architecture"),
            details.architecture(),
            "image architecture is metadata evidence only",
        );
        self.observation_only(
            &format!("{subject}.operating_system"),
            details.operating_system(),
            "image operating system is metadata evidence only",
        );
        self.observation_only(
            &format!("{subject}.manifest_type"),
            details.manifest_type(),
            "image manifest type is metadata evidence only",
        );
    }

    fn map_configured_image(&mut self, subject: &str, details: &ContainerObservation, service: &mut Service) {
        let field_subject = format!("{subject}.image");
        if let Some(image) = self.configured(details.configured_image(), &field_subject) {
            match ImageReference::parse(image) {
                Ok(image) => service.set_image(self.sourced(image)),
                Err(error) => self.model_error(field_subject, &error),
            }
        }
        self.observation_only(
            &format!("{subject}.local_image_id"),
            details.local_image_id(),
            "local image resolution must not replace configured image intent",
        );
    }

    fn map_configured_settings(&mut self, subject: &str, details: &ContainerObservation, service: &mut Service) {
        let command_subject = format!("{subject}.command");
        if let Some(command) = self.configured(details.command(), &command_subject) {
            let value = if command.arguments().is_empty() {
                Command::Empty
            } else {
                Command::Exec(command.arguments().iter().map(ProtectedString::plain).collect())
            };
            service.set_command(self.sourced(value));
        }
        let entrypoint_subject = format!("{subject}.entrypoint");
        if let Some(entrypoint) = self.configured(details.entrypoint(), &entrypoint_subject) {
            let value = if entrypoint.arguments().is_empty() {
                Entrypoint::Empty
            } else {
                Entrypoint::Exec(entrypoint.arguments().iter().map(ProtectedString::plain).collect())
            };
            service.set_entrypoint(self.sourced(value));
        }
        let user_subject = format!("{subject}.user");
        if let Some(user) = self.configured(details.user(), &user_subject) {
            let spelling = user.value();
            if let Some((user, group)) = spelling.split_once(':') {
                if !user.is_empty() && !group.is_empty() && !group.contains(':') {
                    service.set_user(self.sourced(ProtectedString::plain(user)));
                    service.set_group(self.sourced(ProtectedString::plain(group)));
                } else {
                    service.set_user(self.sourced(ProtectedString::plain(spelling)));
                }
            } else {
                service.set_user(self.sourced(ProtectedString::plain(spelling)));
            }
        }
        let workdir_subject = format!("{subject}.working_directory");
        if let Some(workdir) = self.configured(details.working_directory(), &workdir_subject) {
            service.set_working_directory(self.sourced(ProtectedString::plain(workdir.value())));
        }
        let hostname_subject = format!("{subject}.hostname");
        if let Some(hostname) = self.configured(details.hostname(), &hostname_subject) {
            service.set_hostname(self.sourced(ProtectedString::plain(hostname.value())));
        }
    }

    fn map_container_labels(
        &mut self,
        subject: &str,
        field: &ObservationField<podman_lens::observation::Labels>,
        service: &mut Service,
    ) {
        let labels_subject = format!("{subject}.labels");
        let Some(labels) = self.configured(field, &labels_subject) else {
            return;
        };
        for (name, value) in labels {
            match Identifier::new(name) {
                Ok(name) => service.add_label(self.sourced(MetadataLabel::new(name, ProtectedString::plain(value)))),
                Err(error) => self.model_error(&labels_subject, &error),
            }
        }
    }

    fn map_network_labels(
        &mut self,
        subject: &str,
        field: &ObservationField<podman_lens::observation::Labels>,
        network: &mut Network,
    ) {
        let labels_subject = format!("{subject}.labels");
        let Some(labels) = self.configured(field, &labels_subject) else {
            return;
        };
        for (name, value) in labels {
            match Identifier::new(name) {
                Ok(name) => network.add_label(self.sourced(MetadataLabel::new(name, ProtectedString::plain(value)))),
                Err(error) => self.model_error(&labels_subject, &error),
            }
        }
    }

    fn map_volume_labels(
        &mut self,
        subject: &str,
        field: &ObservationField<podman_lens::observation::Labels>,
        volume: &mut Volume,
    ) {
        let labels_subject = format!("{subject}.labels");
        let Some(labels) = self.configured(field, &labels_subject) else {
            return;
        };
        for (name, value) in labels {
            match Identifier::new(name) {
                Ok(name) => volume.add_label(self.sourced(MetadataLabel::new(name, ProtectedString::plain(value)))),
                Err(error) => self.model_error(&labels_subject, &error),
            }
        }
    }

    fn report_infra_container_fields(&mut self, subject: &str, details: &ContainerObservation) {
        macro_rules! report {
            ($name:literal, $field:expr) => {
                self.observation_only(
                    &format!("{subject}.{}", $name),
                    $field,
                    "infra-container field is runtime topology evidence only",
                );
            };
        }
        report!("configured_image", details.configured_image());
        report!("labels", details.labels());
        report!("local_image_id", details.local_image_id());
        report!("environment", details.environment());
        report!("command", details.command());
        report!("entrypoint", details.entrypoint());
        report!("user", details.user());
        report!("working_directory", details.working_directory());
        report!("hostname", details.hostname());
        report!("pod_membership", details.pod_membership());
        report!("native_dependencies", details.native_dependencies());
        report!("mounts", details.mounts());
        if let Some(mounts) = details.mounts().observed() {
            for (index, mount) in mounts.value().iter().enumerate() {
                self.report_unpromoted_mount(&format!("{subject}.mounts[{index}]"), mount);
            }
        }
        self.report_secret_grants(&format!("{subject}.secret_grants"), details.secret_grants());
        report!("memory_swappiness", details.memory_swappiness());
        report!("restart_policy", details.restart_policy());
        report!("health_check", details.health_check());
        report!("health_failure_action", details.health_failure_action());
        report!("startup_health_check", details.startup_health_check());
        report!("logging", details.logging());
        report!("security", details.security());
        report!("namespaces", details.namespaces());
        report!("resource_controls", details.resource_controls());
        self.report_networking_only(&format!("{subject}.networking"), details.networking());
    }

    fn map_pod_membership(&mut self, subject: &str, identity: &ResourceIdentity, details: &ContainerObservation) {
        let field_subject = format!("{subject}.pod_membership");
        let Some(reference) = self.configured(details.pod_membership(), &field_subject) else {
            return;
        };
        let Some(group) = self.resolve_name(ResourceKind::Pod, reference.reference()) else {
            self.identity_conflict(
                field_subject,
                "pod reference does not resolve to exactly one selected pod",
            );
            return;
        };
        self.pod_memberships.insert(identity.id().to_owned(), group);
    }

    fn report_unpromoted_mount(&mut self, subject: &str, mount: &ContainerMountObservation) {
        self.report_mount_core(subject, mount);
        self.report_mount_remainders(subject, mount);
    }

    fn report_mount_core(&mut self, subject: &str, mount: &ContainerMountObservation) {
        self.observation_only(
            &format!("{subject}.source"),
            mount.source(),
            "mount source is not promoted without an exact portable mapping",
        );
        self.observation_only(
            &format!("{subject}.destination"),
            mount.destination(),
            "mount destination is not promoted without an exact portable mapping",
        );
        self.observation_only(
            &format!("{subject}.writable"),
            mount.writable(),
            "mount access is not promoted without an exact portable mapping",
        );
    }

    fn report_mount_remainders(&mut self, subject: &str, mount: &ContainerMountObservation) {
        self.observation_only(
            &format!("{subject}.local_backing_path"),
            mount.local_backing_path(),
            "target-local mount backing paths are observation-only",
        );
        self.observation_only(
            &format!("{subject}.options"),
            mount.options(),
            "native mount options have no complete neutral mapping",
        );
        self.observation_only(
            &format!("{subject}.propagation"),
            mount.propagation(),
            "native mount propagation has no reviewed neutral mapping",
        );
        self.observation_only(
            &format!("{subject}.subpath"),
            mount.subpath(),
            "native volume subpath has no reviewed neutral mapping",
        );
    }

    fn map_mounts(&mut self, subject: &str, details: &ContainerObservation, service: &mut Service) {
        let field_subject = format!("{subject}.mounts");
        let Some(observed) = details.mounts().observed() else {
            self.report_state(details.mounts(), &field_subject);
            return;
        };
        if observed.origin() != ObservationOrigin::Effective {
            for (index, mount) in observed.value().iter().enumerate() {
                self.report_unpromoted_mount(&format!("{field_subject}[{index}]"), mount);
            }
            self.unsupported(
                &field_subject,
                "runtime-assigned or local mount evidence is not desired intent",
            );
            return;
        }
        if !self.source.promotion_policy().promotes_effective_named_volume_mounts() {
            for (index, mount) in observed.value().iter().enumerate() {
                self.report_unpromoted_mount(&format!("{field_subject}[{index}]"), mount);
            }
            self.promotion_required(
                &field_subject,
                "effective named-volume mounts require explicit promotion authorization",
            );
            return;
        }
        for (index, mount) in observed.value().iter().enumerate() {
            let mount_subject = format!("{field_subject}[{index}]");
            self.report_mount_remainders(&mount_subject, mount);
            if mount.kind() != ContainerMountKind::NamedVolume {
                self.report_mount_core(&mount_subject, mount);
                self.unsupported(
                    mount_subject,
                    "local bind paths never become portable intent automatically",
                );
                continue;
            }
            let (Some(source), Some(destination), Some(writable)) = (
                mount.source().observed(),
                mount.destination().observed(),
                mount.writable().observed(),
            ) else {
                self.report_state(mount.source(), &format!("{mount_subject}.source"));
                self.report_state(mount.destination(), &format!("{mount_subject}.destination"));
                self.report_state(mount.writable(), &format!("{mount_subject}.writable"));
                self.invalid(mount_subject, "mount source, destination, or access is incomplete");
                continue;
            };
            if source.origin() != ObservationOrigin::Effective
                || destination.origin() != ObservationOrigin::Effective
                || writable.origin() != ObservationOrigin::Effective
            {
                self.unsupported(
                    mount_subject,
                    "runtime-assigned or local mount fields are never promoted",
                );
                continue;
            }
            let ContainerMountSource::NamedVolume(source_name) = source.value() else {
                self.unsupported(mount_subject, "mount source is target-local");
                continue;
            };
            let Some(volume_name) = self.resolve_name(ResourceKind::Volume, source_name) else {
                self.identity_conflict(mount_subject, "named-volume reference does not resolve uniquely");
                continue;
            };
            match Mount::new(MountSource::Volume(volume_name), destination.value(), !writable.value()) {
                Ok(mount) => {
                    service.add_mount(self.decision_sourced(mount));
                    self.approximate(
                        mount_subject,
                        "portable named-volume mount promoted by explicit BoxFerry policy",
                    );
                }
                Err(error) => self.model_error(mount_subject, &error),
            }
        }
    }

    fn report_networking_only(&mut self, subject: &str, field: &ObservationField<NativeNetworkingObservation>) {
        self.observation_only(
            subject,
            field,
            "native networking evidence is not promoted at this topology boundary",
        );
        if let Some(networking) = field.observed() {
            self.report_networking_remainders(subject, networking.value());
            self.observation_only(
                &format!("{subject}.references"),
                networking.value().networks(),
                "native network references are not promoted at this topology boundary",
            );
        }
    }

    fn report_networking_remainders(&mut self, subject: &str, networking: &NativeNetworkingObservation) {
        macro_rules! report {
            ($name:literal, $field:expr, $reason:literal) => {
                self.observation_only(&format!("{subject}.{}", $name), $field, $reason);
            };
        }
        report!(
            "port_bindings",
            networking.port_bindings(),
            "runtime port bindings require explicit authored publication intent"
        );
        if let Some(bindings) = networking.port_bindings().observed() {
            for (index, binding) in bindings.value().iter().enumerate() {
                self.observation_only(
                    &format!("{subject}.port_bindings[{index}].host_ip"),
                    binding.host_ip(),
                    "runtime-assigned host IP is observation-only",
                );
                self.observation_only(
                    &format!("{subject}.port_bindings[{index}].host_port"),
                    binding.host_port(),
                    "runtime-assigned host port is observation-only",
                );
            }
        }
        report!(
            "create_net_ns",
            networking.create_net_ns(),
            "effective network namespace creation requires explicit promotion"
        );
        report!(
            "host_network",
            networking.host_network(),
            "effective host networking requires explicit promotion"
        );
        report!(
            "dns_servers",
            networking.dns_servers(),
            "effective DNS servers require explicit promotion"
        );
        report!(
            "dns_search",
            networking.dns_search(),
            "effective DNS search domains require explicit promotion"
        );
        report!(
            "dns_options",
            networking.dns_options(),
            "effective DNS options require explicit promotion"
        );
        report!(
            "host_entries",
            networking.host_entries(),
            "opaque host-entry evidence cannot reconstruct authored mappings"
        );
        report!(
            "network_options",
            networking.network_options(),
            "opaque network options cannot reconstruct authored mappings"
        );
        report!(
            "no_manage_resolv_conf",
            networking.no_manage_resolv_conf(),
            "effective resolver-management policy requires explicit promotion"
        );
        report!(
            "no_manage_hosts",
            networking.no_manage_hosts(),
            "effective hosts-management policy requires explicit promotion"
        );
        report!(
            "static_ip",
            networking.static_ip(),
            "runtime-assigned static IP evidence is not promoted automatically"
        );
        report!(
            "static_mac",
            networking.static_mac(),
            "runtime-assigned static MAC evidence is not promoted automatically"
        );
    }

    fn map_container_networks(&mut self, subject: &str, details: &ContainerObservation, service: &mut Service) {
        if details.pod_membership().is_observed() {
            self.report_networking_only(&format!("{subject}.pod_member_networking"), details.networking());
            return;
        }
        let field_subject = format!("{subject}.networks");
        let Some(networking) = details.networking().observed() else {
            self.report_state(details.networking(), &field_subject);
            return;
        };
        self.report_networking_remainders(&field_subject, networking.value());
        if networking.origin() != ObservationOrigin::Effective {
            self.observation_only(
                &format!("{field_subject}.references"),
                networking.value().networks(),
                "pod or container network references are not portable from this observation origin",
            );
            self.unsupported(
                &field_subject,
                "runtime-assigned or local networking evidence is not desired intent",
            );
            return;
        }
        let Some(networks) = networking.value().networks().observed() else {
            self.report_state(networking.value().networks(), &field_subject);
            return;
        };
        if networks.origin() != ObservationOrigin::Effective {
            self.unsupported(
                &field_subject,
                "runtime-assigned or local network references are never promoted",
            );
            return;
        }
        if !self.source.promotion_policy().promotes_effective_named_networks() {
            self.promotion_required(
                &field_subject,
                "effective named networks require explicit promotion authorization",
            );
            return;
        }
        for reference in networks.value() {
            let Some(name) = self.resolve_name(ResourceKind::Network, reference.reference()) else {
                self.identity_conflict(&field_subject, "network reference does not resolve uniquely");
                continue;
            };
            service.add_network(self.decision_sourced(NeutralNetworkAttachment::new(name, Vec::new())));
            self.approximate(
                &field_subject,
                "effective named network promoted without runtime-assigned addresses",
            );
        }
    }

    fn map_dependencies(&mut self, subject: &str, details: &ContainerObservation, service: &mut Service) {
        let field_subject = format!("{subject}.dependencies");
        let Some(dependencies) = self.configured(details.native_dependencies(), &field_subject) else {
            return;
        };
        for dependency in dependencies {
            let Some(name) = self.resolve_name(ResourceKind::Container, dependency.reference()) else {
                self.identity_conflict(&field_subject, "container dependency does not resolve uniquely");
                continue;
            };
            service.add_dependency(self.sourced(ServiceDependency::new(name)));
        }
    }

    fn report_missing_selected_observations(&mut self) {
        let missing = self
            .selected
            .iter()
            .filter(|identity| self.source.inventory().observation(identity).is_none())
            .cloned()
            .collect::<Vec<_>>();
        for identity in missing {
            self.identity_conflict(
                format!(
                    "podman.selected[{}].{}",
                    identity.kind().canonical_rank(),
                    identity_name(&identity)
                ),
                "selected discovery identity is absent from the supplied Podman inventory",
            );
        }
    }

    fn report_secret_grants(&mut self, subject: &str, field: &ObservationField<Vec<ContainerSecretGrantObservation>>) {
        let Some(grants) = field.observed() else {
            self.report_state(field, subject);
            return;
        };
        self.secret_incomplete(subject, "inspect does not preserve secret delivery form and target");
        for (index, grant) in grants.value().iter().enumerate() {
            let grant_subject = format!("{subject}[{index}]");
            self.report_secret_grant_field(&format!("{grant_subject}.reference"), grant.reference());
            self.report_secret_grant_field(&format!("{grant_subject}.uid"), grant.uid());
            self.report_secret_grant_field(&format!("{grant_subject}.gid"), grant.gid());
            self.report_secret_grant_field(&format!("{grant_subject}.mode"), grant.mode());
        }
    }

    fn report_secret_grant_field<T>(&mut self, subject: &str, field: &ObservationField<T>) {
        if field.is_observed() {
            self.secret_incomplete(
                subject,
                "secret grant metadata cannot reconstruct authored delivery form or target",
            );
        } else {
            self.report_state(field, subject);
        }
    }

    fn configured<'b, T>(&mut self, field: &'b ObservationField<T>, subject: &str) -> Option<&'b T> {
        let Some(observed) = field.observed() else {
            self.report_state(field, subject);
            return None;
        };
        match observed.origin() {
            ObservationOrigin::Configured => {
                self.exact(subject);
                Some(observed.value())
            }
            ObservationOrigin::Effective => {
                self.promotion_required(
                    subject,
                    "effective observation requires explicit field-specific promotion",
                );
                None
            }
            ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => {
                self.unsupported(
                    subject,
                    "runtime-assigned and local-resolution observations are never authored intent",
                );
                None
            }
            _ => {
                self.unsupported(subject, "future observation origin is not reviewed for promotion");
                None
            }
        }
    }

    fn observation_only<T>(&mut self, subject: &str, field: &ObservationField<T>, reason: &'static str) {
        match field {
            ObservationField::Observed(observed) => match observed.origin() {
                ObservationOrigin::Configured => self.unsupported(subject, reason),
                ObservationOrigin::Effective => self.promotion_required(subject, reason),
                ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => self.unsupported(
                    subject,
                    "runtime-assigned and local-resolution observations are never authored intent",
                ),
                _ => self.unsupported(subject, "future observation origin is not reviewed"),
            },
            _ => self.report_state(field, subject),
        }
    }

    fn report_state<T>(&mut self, field: &ObservationField<T>, subject: &str) {
        match field {
            ObservationField::Absent | ObservationField::NotApplicable => {
                self.exact(subject);
            }
            ObservationField::VersionInapplicable => self.unsupported(
                subject,
                "native field is version-inapplicable for the inspected Podman version",
            ),
            ObservationField::Observed(observed) => match observed.origin() {
                ObservationOrigin::Configured => self.exact(subject),
                ObservationOrigin::Effective => self.promotion_required(
                    subject,
                    "effective observation requires explicit field-specific promotion",
                ),
                ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => self.unsupported(
                    subject,
                    "runtime-assigned and local-resolution observations are never authored intent",
                ),
                _ => self.unsupported(subject, "future observation origin is not reviewed for promotion"),
            },
            ObservationField::Unavailable => {
                self.invalid(subject, "native observation is unavailable");
            }
            ObservationField::Malformed => {
                self.invalid(subject, "native observation is malformed");
            }
            ObservationField::Unmodelled(_) => {
                self.unsupported(subject, "native field is retained only as bounded metadata");
            }
            _ => self.unsupported(subject, "future native field state is not reviewed"),
        }
    }

    fn require_complete(&mut self, observation: &ResourceObservation, subject: &str) -> bool {
        if observation.header().state() == ResourceObservationState::Complete {
            true
        } else {
            self.invalid(subject, "selected native resource is unavailable or malformed");
            false
        }
    }

    fn identifier(&mut self, subject: &str, identity: &ResourceIdentity) -> Option<Identifier> {
        match Identifier::new(identity_name(identity)) {
            Ok(name) => Some(name),
            Err(error) => {
                self.model_error(subject, &error);
                None
            }
        }
    }

    fn resolve_name(&self, kind: ResourceKind, reference: &str) -> Option<Identifier> {
        let mut matches = self
            .selected
            .iter()
            .filter(|identity| {
                identity.kind() == kind && (identity.id() == reference || identity.name() == Some(reference))
            })
            .map(identity_name);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Identifier::new(first).ok()
    }

    fn report_native_findings(&mut self) {
        let inventory_findings = self
            .selected
            .iter()
            .filter_map(|identity| self.source.inventory().observation(identity))
            .flat_map(|observation| observation.header().findings())
            .map(podman_lens::InventoryFinding::code)
            .collect::<Vec<_>>();
        for code in inventory_findings {
            self.native_finding(code, "acquisition");
        }
        let graph_findings = self
            .source
            .graph()
            .findings()
            .iter()
            .map(podman_lens::DiscoveryFinding::code)
            .collect::<Vec<_>>();
        for code in graph_findings {
            self.native_finding(code, "discovery");
        }
    }

    fn native_finding(&mut self, code: podman_lens::DiagnosticCode, stage: &'static str) {
        let native = NativeFinding::new(
            "podman",
            "podman-lens",
            code.as_str(),
            stage,
            Severity::Warning,
            podman_lens::Diagnostic::new(code).message(),
        );
        self.diagnostics.push(
            Diagnostic::new(
                self.importer.unsupported.clone(),
                Severity::Warning,
                "PodmanLens retained a native acquisition or discovery finding",
            )
            .with_native_finding(native),
        );
        self.push_loss(
            format!("podman.{stage}.{}", code.as_str()),
            ConversionKind::Unsupported,
            self.importer.unsupported.clone(),
        );
    }

    fn exact(&mut self, subject: impl Into<String>) {
        self.outcomes
            .push(ConversionOutcome::exact(subject).with_origin(self.origin.clone()));
    }

    fn approximate(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics.push(Diagnostic::new(
            self.importer.policy.clone(),
            Severity::Warning,
            summary,
        ));
        self.push_loss(subject, ConversionKind::Approximate, self.importer.policy.clone());
    }

    fn promotion_required(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics.push(Diagnostic::new(
            self.importer.policy.clone(),
            Severity::Warning,
            summary,
        ));
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.policy.clone());
    }

    fn unsupported(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics.push(Diagnostic::new(
            self.importer.unsupported.clone(),
            Severity::Warning,
            summary,
        ));
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.unsupported.clone());
    }

    fn identity_conflict(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics.push(Diagnostic::new(
            self.importer.identity.clone(),
            Severity::Error,
            summary,
        ));
        self.push_loss(subject, ConversionKind::Invalid, self.importer.identity.clone());
    }

    fn secret_incomplete(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics.push(Diagnostic::new(
            self.importer.secret.clone(),
            Severity::Warning,
            summary,
        ));
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.secret.clone());
    }

    fn invalid(&mut self, subject: impl Into<String>, summary: &'static str) {
        self.diagnostics
            .push(Diagnostic::new(self.importer.invalid.clone(), Severity::Error, summary));
        self.push_loss(subject, ConversionKind::Invalid, self.importer.invalid.clone());
    }

    fn model_error(&mut self, subject: impl Into<String>, error: &ModelError) {
        match error {
            ModelError::DuplicateResource { .. }
            | ModelError::DuplicateServiceGroupMember { .. }
            | ModelError::UnknownServiceGroupMember { .. }
            | ModelError::ServiceInMultipleGroups { .. } => self.identity_conflict(
                subject,
                "native identities or references conflict in the neutral namespace",
            ),
            _ => {
                self.invalid(subject, "native value cannot be represented safely in neutral model");
            }
        }
    }

    fn push_loss(&mut self, subject: impl Into<String>, kind: ConversionKind, code: DiagnosticCode) {
        if let Ok(outcome) = ConversionOutcome::loss(subject, kind, code) {
            self.outcomes.push(outcome.with_origin(self.origin.clone()));
        }
    }

    fn sourced<T>(&self, value: T) -> Sourced<T> {
        Sourced::from_source(value, self.origin.clone())
    }

    fn decision_sourced<T>(&self, value: T) -> Sourced<T> {
        Sourced::from_source(value, Provenance::conversion_decision(self.origin.source_id().clone()))
    }
}

fn observations<'a>(
    source: &'a PodmanSource,
    selected: &BTreeSet<ResourceIdentity>,
    kind: ResourceKind,
) -> Vec<&'a ResourceObservation> {
    selected
        .iter()
        .filter(|identity| identity.kind() == kind)
        .filter_map(|identity| source.inventory().observation(identity))
        .collect()
}

fn selected_identities(source: &PodmanSource) -> BTreeSet<ResourceIdentity> {
    let mut selected = BTreeSet::new();
    selected.extend(source.graph().resolved_roots().iter().cloned());
    for group in source.graph().groups() {
        selected.insert(group.id().clone());
        selected.extend(group.members().iter().cloned());
        selected.extend(group.prerequisites().iter().cloned());
    }
    selected.extend(source.graph().shared_prerequisites().iter().cloned());
    for dependency in source.graph().dependencies() {
        selected.insert(dependency.dependent().clone());
        selected.insert(dependency.prerequisite().clone());
    }
    selected
}

fn identity_name(identity: &ResourceIdentity) -> &str {
    identity.name().unwrap_or(identity.id())
}
