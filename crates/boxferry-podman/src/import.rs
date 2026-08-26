//! `PodmanLens` observations into the neutral application model.

use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, NativeFinding, Severity,
};
use boxferry_model::{
    Application, Command, Entrypoint, EnvironmentValue, EnvironmentVariable, Healthcheck, HealthcheckCommand,
    HealthcheckDuration, HealthcheckRetries, Identifier, ImageReference, MetadataLabel, ModelError, Mount, MountSource,
    Network, NetworkAttachment as NeutralNetworkAttachment, NetworkIpamConfig, Port, ProtectedString, Protocol,
    Provenance, ResourceOwnership, RestartPolicy, Secret, SelinuxRelabel, Service, ServiceDependency, ServiceGroup,
    SourceId, Sourced, Volume,
};
use podman_lens::{
    ContainerMountKind, ContainerMountObservation, ContainerMountSource, ContainerObservation,
    ContainerSecretGrantObservation, DiscoveryExplanationKind, MAX_UNKNOWN_FIELDS_PER_INVENTORY,
    MAX_UNKNOWN_FIELDS_PER_RECORD, NativeHealthCheckObservation, NativeHealthCommand, NativeNamespaceMode,
    NativeNetworkingObservation, NativePortBindingObservation, NativePortProtocol, NativeRestartPolicyName,
    NativeRestartPolicyObservation, ObservationField, ObservationOrigin, ProtectedEnvironment,
    ProtectedEnvironmentValue, ResourceDetails, ResourceIdentity, ResourceKind, ResourceObservation,
    ResourceObservationState,
};

use crate::PodmanSource;

const PORTABLE_EFFECTIVE_SETTINGS_FLAG: &str = "--promote-podman-portable-effective-settings";
const EFFECTIVE_BIND_MOUNTS_FLAG: &str = "--promote-podman-effective-bind-mounts";
const COMPOSE_LIFECYCLE_LABEL_PREFIX: &str = "com.docker.compose.";

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
        // Acquisition and discovery failures explain why a later field or resource is malformed.
        // Keep those causal diagnostics ahead of the derived mapping diagnostics so callers see
        // the actionable Podman evidence first.
        self.report_native_findings();
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
        let ownership = self.promoted_resource_ownership(
            observation.header().identity(),
            self.source.promotion_policy().promotes_effective_named_networks(),
        );
        let mut network = Network::new(name, ownership);
        self.map_network_labels(&subject, details.labels(), &mut network);
        self.map_network_internal(&subject, details.internal(), &mut network);
        self.map_network_ipam(&subject, details.subnets(), &mut network);
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

    fn map_network_internal(&mut self, subject: &str, field: &ObservationField<bool>, network: &mut Network) {
        let field_subject = format!("{subject}.internal");
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        let Some((internal, promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "effective network-internal state needs explicit promotion policy",
        ) else {
            return;
        };

        network.set_internal(self.portable_sourced(*internal, promoted));
        if promoted {
            self.approximate_with_flag(
                field_subject,
                "effective network-internal state explicitly promoted as portable intent",
                PORTABLE_EFFECTIVE_SETTINGS_FLAG,
            );
        }
    }

    fn map_network_ipam(
        &mut self,
        subject: &str,
        field: &ObservationField<Vec<podman_lens::NativeNetworkSubnetObservation>>,
        network: &mut Network,
    ) {
        let field_subject = format!("{subject}.ipam");
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        let Some((subnets, promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "effective network subnet and address-allocation settings need explicit promotion policy",
        ) else {
            return;
        };

        let mut retained = 0_usize;
        let mut has_ipv6_subnet = false;
        for (index, subnet) in subnets.iter().enumerate() {
            let row_subject = format!("{field_subject}[{index}]");
            let Some((cidr, _)) = self.portable_setting(
                subnet.cidr(),
                &format!("{row_subject}.subnet"),
                authorized,
                "effective network subnet needs explicit promotion policy",
            ) else {
                continue;
            };
            let mut config =
                match NetworkIpamConfig::new(self.portable_sourced(ProtectedString::plain(cidr.as_str()), promoted)) {
                    Ok(config) => config,
                    Err(error) => {
                        self.model_error(format!("{row_subject}.subnet"), &error);
                        continue;
                    }
                };
            has_ipv6_subnet |= cidr
                .as_str()
                .split_once('/')
                .and_then(|(address, _)| address.parse::<IpAddr>().ok())
                .is_some_and(|address| address.is_ipv6());

            if let Some((gateway, _)) = self.portable_setting(
                subnet.gateway(),
                &format!("{row_subject}.gateway"),
                authorized,
                "effective network gateway needs explicit promotion policy",
            ) {
                if let Err(error) =
                    config.set_gateway(self.portable_sourced(ProtectedString::plain(gateway.to_string()), promoted))
                {
                    self.model_error(format!("{row_subject}.gateway"), &error);
                }
            }

            self.map_network_lease_range(&row_subject, subnet.lease_range(), authorized, promoted, &mut config);
            network.add_ipam_config(self.portable_sourced(config, promoted));
            retained += 1;
        }

        if has_ipv6_subnet {
            network.set_ipv6(self.portable_sourced(true, promoted));
        }

        if promoted && retained > 0 {
            self.approximate_with_flag(
                field_subject,
                "effective network subnet and gateway settings explicitly promoted as portable IPAM intent",
                PORTABLE_EFFECTIVE_SETTINGS_FLAG,
            );
        }
    }

    fn map_network_lease_range(
        &mut self,
        row_subject: &str,
        field: &ObservationField<podman_lens::NativeNetworkLeaseRange>,
        authorized: bool,
        promoted: bool,
        config: &mut NetworkIpamConfig,
    ) {
        let range_subject = format!("{row_subject}.lease_range");
        let Some((range, _)) = self.portable_setting(
            field,
            &range_subject,
            authorized,
            "effective network lease range needs explicit promotion policy",
        ) else {
            return;
        };
        let Some((start, _)) = self.portable_setting(
            range.start_ip(),
            &format!("{range_subject}.start_ip"),
            authorized,
            "effective network lease-range start needs explicit promotion policy",
        ) else {
            return;
        };
        let Some((end, _)) = self.portable_setting(
            range.end_ip(),
            &format!("{range_subject}.end_ip"),
            authorized,
            "effective network lease-range end needs explicit promotion policy",
        ) else {
            return;
        };
        let Some(spelling) = lease_range_spelling(*start, *end) else {
            self.observation_only(
                &range_subject,
                field,
                "effective network lease-range endpoints use different address families",
            );
            return;
        };
        if let Err(error) = config.set_ip_range(self.portable_sourced(ProtectedString::plain(spelling), promoted)) {
            self.model_error(range_subject, &error);
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
        let ownership = self.promoted_resource_ownership(
            observation.header().identity(),
            self.source.promotion_policy().promotes_effective_named_volume_mounts(),
        );
        let mut volume = Volume::new(name, ownership);
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
        self.map_configured_settings(&subject, observation.header().identity().id(), details, &mut service);
        self.map_container_labels(&subject, details.labels(), &mut service);
        self.map_mounts(&subject, details, &mut service);
        self.map_container_networks(&subject, details, &mut service);
        self.map_dependencies(&subject, details, &mut service);
        self.map_portable_environment(&subject, details.environment(), &mut service);
        self.report_secret_grants(&format!("{subject}.secret_grants"), details.secret_grants());
        self.observation_only(
            &format!("{subject}.memory_swappiness"),
            details.memory_swappiness(),
            "configured memory swappiness has no neutral field",
        );
        self.map_portable_restart_policy(&subject, details.restart_policy(), &mut service);
        self.map_portable_healthcheck(&subject, details.health_check(), &mut service);
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

    fn map_portable_environment(
        &mut self,
        subject: &str,
        field: &ObservationField<ProtectedEnvironment>,
        service: &mut Service,
    ) {
        let field_subject = format!("{subject}.environment");
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        let Some((environment, promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "protected environment values require caller authorization",
        ) else {
            return;
        };

        let mut retained = 0_usize;
        for entry in environment.entries() {
            let entry_subject = format!("{field_subject}.{}", entry.name());
            let Ok(name) = Identifier::new(entry.name()) else {
                self.unsupported(
                    &entry_subject,
                    "environment name cannot be represented in the neutral model",
                );
                continue;
            };
            let ProtectedEnvironmentValue::AuthorizedOpaque(value) = entry.value() else {
                self.unsupported(
                    &entry_subject,
                    "environment value remained redacted during acquisition and cannot be promoted",
                );
                continue;
            };
            let variable = value.expose(|value| {
                EnvironmentVariable::new(name, EnvironmentValue::Literal(ProtectedString::sensitive(value)))
            });
            service.add_environment(if promoted {
                self.decision_sourced(variable)
            } else {
                self.sourced(variable)
            });
            retained += 1;
        }

        if promoted && retained > 0 {
            self.approximate_with_flag(
                field_subject,
                "effective environment values were explicitly promoted as protected portable intent",
                PORTABLE_EFFECTIVE_SETTINGS_FLAG,
            );
        }
    }

    fn map_portable_restart_policy(
        &mut self,
        subject: &str,
        field: &ObservationField<NativeRestartPolicyObservation>,
        service: &mut Service,
    ) {
        let field_subject = format!("{subject}.restart_policy");
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        let Some((restart, promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "effective restart policy needs explicit promotion policy",
        ) else {
            return;
        };
        let Some((name, _)) = self.portable_setting(
            restart.name(),
            &format!("{field_subject}.name"),
            authorized,
            "effective restart policy name needs explicit promotion policy",
        ) else {
            return;
        };
        let retry_count = self
            .portable_setting(
                restart.maximum_retry_count(),
                &format!("{field_subject}.maximum_retry_count"),
                authorized,
                "effective restart retry count needs explicit promotion policy",
            )
            .map(|(value, _)| *value);
        let policy = match name {
            NativeRestartPolicyName::No => RestartPolicy::Never,
            NativeRestartPolicyName::Always => RestartPolicy::Always,
            NativeRestartPolicyName::OnFailure => {
                RestartPolicy::on_failure(retry_count.and_then(std::num::NonZeroU64::new))
            }
            NativeRestartPolicyName::UnlessStopped => RestartPolicy::UnlessStopped,
            _ => {
                self.unsupported(
                    &field_subject,
                    "native restart policy is not reviewed for neutral promotion",
                );
                return;
            }
        };
        service.set_restart_policy(if promoted {
            self.decision_sourced(policy)
        } else {
            self.sourced(policy)
        });
        if promoted {
            self.approximate_with_flag(
                field_subject,
                "effective restart policy was explicitly promoted as portable intent",
                PORTABLE_EFFECTIVE_SETTINGS_FLAG,
            );
        }
    }

    fn map_portable_healthcheck(
        &mut self,
        subject: &str,
        field: &ObservationField<NativeHealthCheckObservation>,
        service: &mut Service,
    ) {
        let field_subject = format!("{subject}.healthcheck");
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        let Some((native, promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "effective health configuration needs explicit promotion policy",
        ) else {
            return;
        };
        let mut healthcheck = Healthcheck::new();
        let mut retained = false;

        if let Some((command, _)) = self.portable_setting(
            native.command(),
            &format!("{field_subject}.command"),
            authorized,
            "effective health command needs explicit promotion policy",
        ) {
            match command {
                NativeHealthCommand::Disabled => {
                    healthcheck.set_disabled(self.portable_sourced(true, promoted));
                    retained = true;
                }
                NativeHealthCommand::Shell(command) => {
                    let command = command.expose(|arguments| arguments.join(" "));
                    healthcheck.set_command(
                        self.portable_sourced(HealthcheckCommand::Shell(ProtectedString::sensitive(command)), promoted),
                    );
                    retained = true;
                }
                NativeHealthCommand::Exec(command) => {
                    let command =
                        command.expose(|arguments| arguments.iter().map(ProtectedString::sensitive).collect());
                    healthcheck.set_command(self.portable_sourced(HealthcheckCommand::Exec(command), promoted));
                    retained = true;
                }
                _ => {
                    self.unsupported(
                        format!("{field_subject}.command"),
                        "native health command is not reviewed for neutral promotion",
                    );
                }
            }
        }

        retained |= self.map_health_duration(
            native.interval(),
            &format!("{field_subject}.interval"),
            authorized,
            promoted,
            Healthcheck::set_interval,
            &mut healthcheck,
        );
        retained |= self.map_health_duration(
            native.timeout(),
            &format!("{field_subject}.timeout"),
            authorized,
            promoted,
            Healthcheck::set_timeout,
            &mut healthcheck,
        );
        retained |= self.map_health_duration(
            native.start_period(),
            &format!("{field_subject}.start_period"),
            authorized,
            promoted,
            Healthcheck::set_start_period,
            &mut healthcheck,
        );
        if let Some((retries, _)) = self.portable_setting(
            native.retries(),
            &format!("{field_subject}.retries"),
            authorized,
            "effective health retries need explicit promotion policy",
        ) {
            match HealthcheckRetries::new(retries.to_string()) {
                Ok(value) => {
                    healthcheck.set_retries(self.portable_sourced(value, promoted));
                    retained = true;
                }
                Err(error) => self.model_error(format!("{field_subject}.retries"), &error),
            }
        }

        if retained {
            service.set_healthcheck(if promoted {
                self.decision_sourced(healthcheck)
            } else {
                self.sourced(healthcheck)
            });
            if promoted {
                self.approximate_with_flag(
                    field_subject,
                    "effective normal healthcheck was explicitly promoted as portable intent",
                    PORTABLE_EFFECTIVE_SETTINGS_FLAG,
                );
            }
        }
    }

    fn map_health_duration(
        &mut self,
        field: &ObservationField<i64>,
        subject: &str,
        authorized: bool,
        promoted: bool,
        set: impl FnOnce(&mut Healthcheck, Sourced<HealthcheckDuration>),
        healthcheck: &mut Healthcheck,
    ) -> bool {
        let Some((value, _)) = self.portable_setting(
            field,
            subject,
            authorized,
            "effective health duration needs explicit promotion policy",
        ) else {
            return false;
        };
        if *value < 0 {
            self.unsupported(subject, "negative native health duration cannot become portable intent");
            return false;
        }
        match HealthcheckDuration::new(format!("{value}ns")) {
            Ok(value) => {
                set(healthcheck, self.portable_sourced(value, promoted));
                true
            }
            Err(error) => {
                self.model_error(subject, &error);
                false
            }
        }
    }

    fn portable_sourced<T>(&self, value: T, promoted: bool) -> Sourced<T> {
        if promoted {
            self.decision_sourced(value)
        } else {
            self.sourced(value)
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
        let configured_image_retained = if let Some(image) = self.configured(details.configured_image(), &field_subject)
        {
            match ImageReference::parse(image) {
                Ok(image) => {
                    service.set_image(self.sourced(image));
                    true
                }
                Err(error) => {
                    self.model_error(field_subject, &error);
                    false
                }
            }
        } else {
            false
        };
        self.observation_only(
            &format!("{subject}.local_image_id"),
            details.local_image_id(),
            if configured_image_retained {
                "Podman local image ID is host-local resolution evidence; Image= was copied unchanged from Podman inspect $.ImageName"
            } else {
                "Podman local image ID is host-local resolution evidence; no portable configured image reference was available"
            },
        );
    }

    fn map_configured_settings(
        &mut self,
        subject: &str,
        native_id: &str,
        details: &ContainerObservation,
        service: &mut Service,
    ) {
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
            if !spelling.is_empty() {
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
        }
        let workdir_subject = format!("{subject}.working_directory");
        if let Some(workdir) = self.configured(details.working_directory(), &workdir_subject) {
            service.set_working_directory(self.sourced(ProtectedString::plain(workdir.value())));
        }
        let hostname_subject = format!("{subject}.hostname");
        let runtime_hostname = native_id.get(..12).unwrap_or(native_id);
        let host_uts_namespace = details
            .namespaces()
            .observed()
            .and_then(|namespaces| namespaces.value().uts().observed())
            .is_some_and(|uts| matches!(uts.value(), NativeNamespaceMode::Host));
        let hostname_reason = if host_uts_namespace {
            "a hostname observed with the host UTS namespace is not portable authored intent"
        } else {
            "Podman's ID-derived hostname is runtime-assigned evidence"
        };
        if let Some(hostname) = self.configured_unless(
            details.hostname(),
            &hostname_subject,
            |hostname| hostname.value() == runtime_hostname || host_uts_namespace,
            hostname_reason,
        ) {
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
            if is_compose_lifecycle_label(name) {
                continue;
            }
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
            if is_compose_lifecycle_label(name) {
                continue;
            }
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
            if is_compose_lifecycle_label(name) {
                continue;
            }
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
        self.report_mount_options(subject, mount);
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

    #[expect(
        clippy::too_many_lines,
        reason = "one ordered mount pipeline keeps promotion and residual-loss decisions adjacent"
    )]
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
        if !self.source.promotion_policy().promotes_effective_named_volume_mounts()
            && !self.source.promotion_policy().promotes_effective_bind_mounts()
        {
            for (index, mount) in observed.value().iter().enumerate() {
                self.report_unpromoted_mount(&format!("{field_subject}[{index}]"), mount);
            }
            self.promotion_required_with_flag(
                &field_subject,
                "effective mounts require explicit promotion authorization",
                "--promote-podman-effective-named-volumes or --promote-podman-effective-bind-mounts",
            );
            return;
        }
        for (index, mount) in observed.value().iter().enumerate() {
            let mount_subject = format!("{field_subject}[{index}]");
            if mount.kind() == ContainerMountKind::Bind {
                self.map_effective_bind_mount(&mount_subject, mount, service);
                continue;
            }
            if !self.source.promotion_policy().promotes_effective_named_volume_mounts() {
                self.report_unpromoted_mount(&mount_subject, mount);
                self.promotion_required_with_flag(
                    mount_subject,
                    "effective named-volume mount requires explicit promotion authorization",
                    "--promote-podman-effective-named-volumes",
                );
                continue;
            }
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
            if !matches!(
                source.origin(),
                ObservationOrigin::Configured | ObservationOrigin::Effective
            ) || !matches!(
                destination.origin(),
                ObservationOrigin::Configured | ObservationOrigin::Effective
            ) || !matches!(
                writable.origin(),
                ObservationOrigin::Configured | ObservationOrigin::Effective
            ) {
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
                Ok(mut mapped) => {
                    if !Self::apply_mount_selinux_relabel(mount, &mut mapped) {
                        continue;
                    }
                    service.add_mount(self.decision_sourced(mapped));
                    self.approximate_with_flag(
                        mount_subject,
                        "portable named-volume mount promoted by explicit BoxFerry policy",
                        "--promote-podman-effective-named-volumes",
                    );
                }
                Err(error) => self.model_error(mount_subject, &error),
            }
        }
    }

    fn map_effective_bind_mount(&mut self, subject: &str, mount: &ContainerMountObservation, service: &mut Service) {
        if !self.source.promotion_policy().promotes_effective_bind_mounts() {
            self.report_unpromoted_mount(subject, mount);
            self.promotion_required_with_flag(
                subject,
                "host-local bind mount requires explicit same-path promotion authorization",
                EFFECTIVE_BIND_MOUNTS_FLAG,
            );
            return;
        }
        let (Some(source), Some(destination), Some(writable)) = (
            mount.source().observed(),
            mount.destination().observed(),
            mount.writable().observed(),
        ) else {
            self.report_state(mount.source(), &format!("{subject}.source"));
            self.report_state(mount.destination(), &format!("{subject}.destination"));
            self.report_state(mount.writable(), &format!("{subject}.writable"));
            self.invalid(subject, "bind-mount source, destination, or access is incomplete");
            return;
        };
        let ContainerMountSource::LocalBindPath(source_path) = source.value() else {
            self.invalid(subject, "bind-mount kind and source disagree");
            return;
        };
        if !source_path.starts_with('/') {
            self.unsupported(subject, "host-local bind source is not an absolute path");
            return;
        }
        if !matches!(
            destination.origin(),
            ObservationOrigin::Configured | ObservationOrigin::Effective
        ) || !matches!(
            writable.origin(),
            ObservationOrigin::Configured | ObservationOrigin::Effective
        ) {
            self.unsupported(subject, "runtime-assigned bind destination or access is never promoted");
            return;
        }

        self.report_bind_mount_remainders(subject, mount);
        match Mount::new(
            MountSource::HostPath(source_path.clone()),
            destination.value(),
            !writable.value(),
        ) {
            Ok(mut mapped) => {
                if !Self::apply_mount_selinux_relabel(mount, &mut mapped) {
                    return;
                }
                service.add_mount(self.decision_sourced(mapped));
                self.approximate_with_flag(
                    subject,
                    "host-local bind mount promoted for a reviewed same-path target",
                    EFFECTIVE_BIND_MOUNTS_FLAG,
                );
            }
            Err(error) => self.model_error(subject, &error),
        }
    }

    fn report_bind_mount_remainders(&mut self, subject: &str, mount: &ContainerMountObservation) {
        self.exact(format!("{subject}.local_backing_path"));
        self.report_mount_options(subject, mount);
        match mount.propagation().observed() {
            Some(propagation) if matches!(propagation.value().as_str(), "" | "private" | "rprivate") => {
                self.exact(format!("{subject}.propagation"));
            }
            _ => self.observation_only(
                &format!("{subject}.propagation"),
                mount.propagation(),
                "non-default bind-mount propagation has no reviewed neutral mapping",
            ),
        }
        match mount.subpath().observed() {
            Some(subpath) if subpath.value().is_empty() => self.exact(format!("{subject}.subpath")),
            _ => self.observation_only(
                &format!("{subject}.subpath"),
                mount.subpath(),
                "native bind-mount subpath has no reviewed neutral mapping",
            ),
        }
    }

    fn report_mount_options(&mut self, subject: &str, mount: &ContainerMountObservation) {
        let option_subject = format!("{subject}.options");
        let Some(options) = mount.options().observed() else {
            self.report_state(mount.options(), &option_subject);
            return;
        };
        match decoded_mount_options(options.value()) {
            Ok((_, true)) => self.exact(option_subject),
            Ok((_, false)) => self.observation_only(
                &option_subject,
                mount.options(),
                "native mount options beyond access, bind recursion, and SELinux relabeling have no neutral mapping",
            ),
            Err(()) => self.invalid(
                option_subject,
                "native mount options contain conflicting SELinux relabel modes",
            ),
        }
    }

    fn apply_mount_selinux_relabel(native: &ContainerMountObservation, mapped: &mut Mount) -> bool {
        let Some(options) = native.options().observed() else {
            return true;
        };
        let Ok((relabel, _)) = decoded_mount_options(options.value()) else {
            return false;
        };
        if let Some(relabel) = relabel {
            mapped.set_selinux_relabel(relabel);
        }
        true
    }

    fn report_networking_only(&mut self, subject: &str, field: &ObservationField<NativeNetworkingObservation>) {
        self.observation_only(
            subject,
            field,
            "native networking evidence is not promoted at this topology boundary",
        );
        if let Some(networking) = field.observed() {
            self.report_networking_remainders(subject, networking.value(), false);
            self.observation_only(
                &format!("{subject}.references"),
                networking.value().networks(),
                "native network references are not promoted at this topology boundary",
            );
        }
    }

    fn report_networking_remainders(
        &mut self,
        subject: &str,
        networking: &NativeNetworkingObservation,
        portable_fields_handled: bool,
    ) {
        macro_rules! report {
            ($name:literal, $field:expr, $reason:literal) => {
                self.observation_only(&format!("{subject}.{}", $name), $field, $reason);
            };
        }
        if !portable_fields_handled {
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
        if !portable_fields_handled {
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
        }
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

    fn map_portable_networking(
        &mut self,
        subject: &str,
        networking: &NativeNetworkingObservation,
        service: &mut Service,
    ) {
        let authorized = self.source.promotion_policy().promotes_portable_effective_settings();
        self.map_portable_ports(subject, networking.port_bindings(), authorized, service);

        let dns_servers_subject = format!("{subject}.dns_servers");
        if let Some((servers, promoted)) = self.portable_setting(
            networking.dns_servers(),
            &dns_servers_subject,
            authorized,
            "effective DNS servers require explicit promotion",
        ) {
            if servers.is_empty() {
                self.exact(dns_servers_subject);
            } else {
                let values = servers
                    .iter()
                    .map(|value| self.portable_sourced(ProtectedString::plain(value.to_string()), promoted))
                    .collect();
                service.set_dns_servers(values);
                if promoted {
                    self.approximate_with_flag(
                        dns_servers_subject,
                        "effective DNS servers were explicitly promoted as portable intent",
                        PORTABLE_EFFECTIVE_SETTINGS_FLAG,
                    );
                }
            }
        }

        let dns_search_subject = format!("{subject}.dns_search");
        if let Some((domains, promoted)) = self.portable_setting(
            networking.dns_search(),
            &dns_search_subject,
            authorized,
            "effective DNS search domains require explicit promotion",
        ) {
            if domains.is_empty() {
                self.exact(dns_search_subject);
            } else {
                let values = domains
                    .iter()
                    .map(|value| self.portable_sourced(ProtectedString::plain(value), promoted))
                    .collect();
                service.set_dns_search_domains(values);
                if promoted {
                    self.approximate_with_flag(
                        dns_search_subject,
                        "effective DNS search domains were explicitly promoted as portable intent",
                        PORTABLE_EFFECTIVE_SETTINGS_FLAG,
                    );
                }
            }
        }

        let dns_options_subject = format!("{subject}.dns_options");
        if let Some((options, promoted)) = self.portable_setting(
            networking.dns_options(),
            &dns_options_subject,
            authorized,
            "effective DNS options require explicit promotion",
        ) {
            if options.is_empty() {
                self.exact(dns_options_subject);
            } else {
                let values = options
                    .iter()
                    .map(|value| self.portable_sourced(ProtectedString::plain(value), promoted))
                    .collect();
                service.set_dns_options(values);
                if promoted {
                    self.approximate_with_flag(
                        dns_options_subject,
                        "effective DNS options were explicitly promoted as portable intent",
                        PORTABLE_EFFECTIVE_SETTINGS_FLAG,
                    );
                }
            }
        }
    }

    fn map_portable_ports(
        &mut self,
        subject: &str,
        field: &ObservationField<Vec<NativePortBindingObservation>>,
        authorized: bool,
        service: &mut Service,
    ) {
        let field_subject = format!("{subject}.port_bindings");
        let Some((bindings, collection_promoted)) = self.portable_setting(
            field,
            &field_subject,
            authorized,
            "effective published ports require explicit promotion",
        ) else {
            return;
        };
        let mut promoted_any = collection_promoted;
        for (index, binding) in bindings.iter().enumerate() {
            let binding_subject = format!("{field_subject}[{index}]");
            let host_port = self
                .portable_setting(
                    binding.host_port(),
                    &format!("{binding_subject}.host_port"),
                    authorized,
                    "effective published host port requires explicit promotion",
                )
                .map(|(value, promoted)| (*value, promoted));
            if binding.host_port().is_observed() && host_port.is_none() {
                continue;
            }
            let host_ip = self
                .portable_setting(
                    binding.host_ip(),
                    &format!("{binding_subject}.host_ip"),
                    authorized,
                    "effective published host address requires explicit promotion",
                )
                .map(|(value, promoted)| (value.to_string(), promoted));
            if binding.host_ip().is_observed() && host_ip.is_none() {
                continue;
            }
            let promoted = collection_promoted
                || host_port.is_some_and(|(_, promoted)| promoted)
                || host_ip.as_ref().is_some_and(|(_, promoted)| *promoted);
            promoted_any |= promoted;
            let protocol = match binding.protocol() {
                NativePortProtocol::Tcp => Protocol::Tcp,
                NativePortProtocol::Udp => Protocol::Udp,
                NativePortProtocol::Sctp => Protocol::Sctp,
                _ => {
                    self.unsupported(
                        &binding_subject,
                        "native port protocol is not reviewed for neutral promotion",
                    );
                    continue;
                }
            };
            match Port::new(
                binding.container_port(),
                host_port.map(|(value, _)| value),
                host_ip.map(|(value, _)| value),
                protocol,
            ) {
                Ok(port) => service.add_port(self.portable_sourced(port, promoted)),
                Err(error) => self.model_error(binding_subject, &error),
            }
        }
        if promoted_any {
            self.approximate_with_flag(
                field_subject,
                "effective published ports were explicitly promoted as portable intent",
                PORTABLE_EFFECTIVE_SETTINGS_FLAG,
            );
        }
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
        self.map_portable_networking(&field_subject, networking.value(), service);
        self.report_networking_remainders(&field_subject, networking.value(), true);
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
            self.promotion_required_with_flag(
                &field_subject,
                "effective named networks require explicit promotion authorization",
                "--promote-podman-effective-named-networks",
            );
            return;
        }
        for reference in networks.value() {
            let Some(name) = self.resolve_name(ResourceKind::Network, reference.reference()) else {
                self.identity_conflict(&field_subject, "network reference does not resolve uniquely");
                continue;
            };
            service.add_network(self.decision_sourced(NeutralNetworkAttachment::new(name, Vec::new())));
            self.approximate_with_flag(
                &field_subject,
                "effective named network promoted without runtime-assigned addresses",
                "--promote-podman-effective-named-networks",
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
                self.unsupported_with_origin(
                    subject,
                    "runtime-assigned and local-resolution observations are never authored intent",
                    observed.origin(),
                );
                None
            }
            _ => {
                self.unsupported(subject, "future observation origin is not reviewed for promotion");
                None
            }
        }
    }

    fn portable_setting<'b, T>(
        &mut self,
        field: &'b ObservationField<T>,
        subject: &str,
        effective_authorized: bool,
        effective_reason: &'static str,
    ) -> Option<(&'b T, bool)> {
        let Some(observed) = field.observed() else {
            self.report_state(field, subject);
            return None;
        };
        match observed.origin() {
            ObservationOrigin::Configured => {
                self.exact(subject);
                Some((observed.value(), false))
            }
            ObservationOrigin::Effective if effective_authorized => Some((observed.value(), true)),
            ObservationOrigin::Effective => {
                self.promotion_required_with_flag(subject, effective_reason, PORTABLE_EFFECTIVE_SETTINGS_FLAG);
                None
            }
            ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => {
                self.unsupported_with_origin(
                    subject,
                    "runtime-assigned and local-resolution observations are never promoted as portable settings",
                    observed.origin(),
                );
                None
            }
            _ => {
                self.unsupported(
                    subject,
                    "future observation origin is not reviewed for portable promotion",
                );
                None
            }
        }
    }

    fn configured_unless<'b, T>(
        &mut self,
        field: &'b ObservationField<T>,
        subject: &str,
        reject: impl FnOnce(&T) -> bool,
        reason: &'static str,
    ) -> Option<&'b T> {
        let Some(observed) = field.observed() else {
            self.report_state(field, subject);
            return None;
        };
        match observed.origin() {
            ObservationOrigin::Configured if reject(observed.value()) => {
                self.unsupported_with_origin(subject, reason, observed.origin());
                None
            }
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
                self.unsupported_with_origin(
                    subject,
                    "runtime-assigned and local-resolution observations are never authored intent",
                    observed.origin(),
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
                ObservationOrigin::Configured => self.unsupported_with_origin(subject, reason, observed.origin()),
                ObservationOrigin::Effective => self.promotion_required(subject, reason),
                ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => {
                    self.unsupported_with_origin(subject, reason, observed.origin());
                }
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
                ObservationOrigin::RuntimeAssigned | ObservationOrigin::LocalResolution => self
                    .unsupported_with_origin(
                        subject,
                        "runtime-assigned and local-resolution observations are never authored intent",
                        observed.origin(),
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
            let has_causal_finding = observation
                .header()
                .findings()
                .iter()
                .any(|finding| invalid_inventory_finding(finding.code()));
            if !has_causal_finding {
                self.invalid_observation(
                    subject,
                    "selected native resource is unavailable or malformed",
                    observation.header().identity(),
                );
            }
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
        let mut inventory_findings = self
            .source
            .inventory()
            .sections()
            .iter()
            .filter(|section| self.inventory_section_is_relevant(section.kind()))
            .flat_map(|section| {
                section
                    .findings()
                    .iter()
                    .filter(|finding| finding.resource().is_none())
                    .cloned()
                    .map(move |finding| (section.kind(), None, finding))
            })
            .collect::<Vec<_>>();
        inventory_findings.extend(
            self.selected
                .iter()
                .filter_map(|identity| self.source.inventory().observation(identity))
                .flat_map(|observation| {
                    observation.header().findings().iter().cloned().map(move |finding| {
                        (
                            observation.header().identity().kind(),
                            Some(observation_state_name(observation.header().state())),
                            finding,
                        )
                    })
                })
                .collect::<Vec<_>>(),
        );

        let mut ordinary = BTreeMap::<
            (String, &'static str, Option<String>, Option<&'static str>),
            Vec<(ResourceKind, Option<&'static str>, podman_lens::InventoryFinding)>,
        >::new();
        for (kind, state, finding) in inventory_findings {
            if invalid_inventory_finding(finding.code()) {
                self.inventory_finding(&finding, kind, state);
                continue;
            }
            let key = (
                finding.code().as_str().to_owned(),
                resource_kind_name(kind),
                finding.resource().map(resource_locator),
                state,
            );
            ordinary.entry(key).or_default().push((kind, state, finding));
        }
        for findings in ordinary.values() {
            self.native_inventory_findings(findings);
        }

        let graph_findings = self.source.graph().findings().to_vec();
        for finding in graph_findings {
            if self.discovery_finding_is_relevant(&finding) {
                self.discovery_finding(&finding);
            }
        }
    }

    fn discovery_finding_is_relevant(&self, finding: &podman_lens::DiscoveryFinding) -> bool {
        if matches!(
            finding.code(),
            podman_lens::DiagnosticCode::AdvisoryLabelIncomplete | podman_lens::DiagnosticCode::AdvisoryLabelConflict
        ) {
            return false;
        }
        finding
            .resource_identity()
            .is_none_or(|identity| self.selected.contains(identity))
    }

    fn promoted_resource_ownership(&self, identity: &ResourceIdentity, promoted: bool) -> ResourceOwnership {
        if !promoted {
            return ResourceOwnership::Uncertain;
        }
        if self.source.graph().explanations().iter().any(|explanation| {
            explanation.resource() == identity
                && matches!(explanation.kind(), DiscoveryExplanationKind::StoppedSharedBoundary)
        }) {
            ResourceOwnership::External
        } else {
            ResourceOwnership::Application
        }
    }

    fn inventory_section_is_relevant(&self, kind: ResourceKind) -> bool {
        let graph = self.source.graph();
        graph.all_requested()
            || !graph.requested_label_roots().is_empty()
            || graph.requested_roots().iter().any(|selector| selector.kind() == kind)
            || self.selected.iter().any(|identity| identity.kind() == kind)
    }

    fn inventory_finding(
        &mut self,
        finding: &podman_lens::InventoryFinding,
        section_kind: ResourceKind,
        observation_state: Option<&'static str>,
    ) {
        let code = finding.code();
        let resource = finding.resource().map(resource_locator);
        let subject = resource
            .clone()
            .unwrap_or_else(|| format!("podman.inventory.{}", resource_kind_name(section_kind)));
        let reason = podman_native_summary(code);
        let context = self.inventory_finding_context(finding, section_kind, observation_state);
        let mut native = NativeFinding::new(
            "podman",
            "podman-lens",
            code.as_str(),
            "acquisition",
            Severity::Warning,
            reason,
        )
        .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
        .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)));

        let adapter_code = self.importer.invalid.clone();
        let mut diagnostic = diagnostic_with_context(
            adapter_code.clone(),
            Severity::Error,
            "Podman source evidence is incomplete or malformed",
            &subject,
            reason,
            "rejected",
            None,
        );
        for field in context {
            native = native.with_field(field.clone());
            diagnostic = diagnostic.with_field(field);
        }
        self.diagnostics.push(diagnostic.with_native_finding(native));
        self.push_loss(
            format!("podman.acquisition.{}", code.as_str()),
            ConversionKind::Invalid,
            adapter_code,
        );
    }

    fn inventory_finding_context(
        &self,
        finding: &podman_lens::InventoryFinding,
        section_kind: ResourceKind,
        observation_state: Option<&'static str>,
    ) -> Vec<DiagnosticField> {
        let mut fields = vec![
            DiagnosticField::new("observation_origin", DiagnosticValue::plain("acquisition")),
            DiagnosticField::new(
                "source_engine",
                DiagnosticValue::plain(self.source.observed_engine_version()),
            ),
            DiagnosticField::new("source_api", DiagnosticValue::plain(self.source.observed_api_version())),
            DiagnosticField::new(
                "resource_kind",
                DiagnosticValue::plain(resource_kind_name(section_kind)),
            ),
        ];
        if let Some(resource) = finding.resource() {
            fields.push(DiagnosticField::new(
                "resource",
                DiagnosticValue::plain(resource_locator(resource)),
            ));
        }
        if let Some(state) = observation_state {
            fields.push(DiagnosticField::new("observation_state", DiagnosticValue::plain(state)));
        }
        if let Some(path) = finding.field_path() {
            fields.push(DiagnosticField::new("native_path", DiagnosticValue::plain(path)));
        }
        if let Some(occurrence) = finding.occurrence() {
            fields.push(DiagnosticField::new(
                "occurrence",
                DiagnosticValue::plain(occurrence.to_string()),
            ));
        }
        fields
    }

    fn discovery_finding(&mut self, finding: &podman_lens::DiscoveryFinding) {
        let code = finding.code();
        if !invalid_discovery_finding(code) {
            self.native_discovery_finding(finding);
            return;
        }

        let source_version = self.source.observed_engine_version();
        let source_api = self.source.observed_api_version();
        let resource = discovery_resource_locator(finding);
        let subject = resource.clone().unwrap_or_else(|| "podman.discovery".to_owned());
        let reason = podman_native_summary(code);
        let native = NativeFinding::new(
            "podman",
            "podman-lens",
            code.as_str(),
            "discovery",
            Severity::Warning,
            reason,
        )
        .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
        .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
        .with_field(DiagnosticField::new(
            "observation_origin",
            DiagnosticValue::plain("discovery"),
        ))
        .with_field(DiagnosticField::new(
            "source_engine",
            DiagnosticValue::plain(source_version),
        ))
        .with_field(DiagnosticField::new("source_api", DiagnosticValue::plain(source_api)));
        let native = resource.as_deref().map_or(native.clone(), |resource| {
            native.with_field(DiagnosticField::new("resource", DiagnosticValue::plain(resource)))
        });
        let native = finding.field_path().map_or(native.clone(), |field_path| {
            native.with_field(DiagnosticField::new("native_path", DiagnosticValue::plain(field_path)))
        });

        let diagnostic = Diagnostic::new(
            self.importer.invalid.clone(),
            Severity::Error,
            "Podman resource discovery is incomplete or ambiguous",
        )
        .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
        .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
        .with_field(DiagnosticField::new("decision", DiagnosticValue::plain("rejected")))
        .with_field(DiagnosticField::new(
            "observation_origin",
            DiagnosticValue::plain("discovery"),
        ))
        .with_field(DiagnosticField::new(
            "source_engine",
            DiagnosticValue::plain(source_version),
        ))
        .with_field(DiagnosticField::new("source_api", DiagnosticValue::plain(source_api)));
        let diagnostic = resource.as_deref().map_or(diagnostic.clone(), |resource| {
            diagnostic.with_field(DiagnosticField::new("resource", DiagnosticValue::plain(resource)))
        });
        let diagnostic = finding.field_path().map_or(diagnostic.clone(), |field_path| {
            diagnostic.with_field(DiagnosticField::new("native_path", DiagnosticValue::plain(field_path)))
        });
        self.diagnostics.push(diagnostic.with_native_finding(native));
        self.push_loss(
            format!("podman.discovery.{}", code.as_str()),
            ConversionKind::Invalid,
            self.importer.invalid.clone(),
        );
    }

    fn native_discovery_finding(&mut self, finding: &podman_lens::DiscoveryFinding) {
        let code = finding.code();
        let stage = "discovery";
        let reason = podman_native_summary(code);
        let resource = discovery_resource_locator(finding);
        let subject = resource.clone().unwrap_or_else(|| "podman.discovery".to_owned());
        let native = NativeFinding::new("podman", "podman-lens", code.as_str(), stage, Severity::Warning, reason)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
            .with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain(stage),
            ))
            .with_field(DiagnosticField::new(
                "source_engine",
                DiagnosticValue::plain(self.source.observed_engine_version()),
            ))
            .with_field(DiagnosticField::new(
                "source_api",
                DiagnosticValue::plain(self.source.observed_api_version()),
            ));
        let native = resource.as_deref().map_or(native.clone(), |resource| {
            native.with_field(DiagnosticField::new("resource", DiagnosticValue::plain(resource)))
        });
        let native = finding.field_path().map_or(native.clone(), |path| {
            native.with_field(DiagnosticField::new("native_path", DiagnosticValue::plain(path)))
        });
        let mut diagnostic = diagnostic_with_context(
            self.importer.unsupported.clone(),
            Severity::Warning,
            "PodmanLens retained a native discovery finding",
            &subject,
            reason,
            "omitted",
            Some("partial"),
        )
        .with_field(DiagnosticField::new(
            "available_promotion",
            DiagnosticValue::plain("none"),
        ))
        .with_field(DiagnosticField::new(
            "observation_origin",
            DiagnosticValue::plain(stage),
        ))
        .with_field(DiagnosticField::new(
            "source_engine",
            DiagnosticValue::plain(self.source.observed_engine_version()),
        ))
        .with_field(DiagnosticField::new(
            "source_api",
            DiagnosticValue::plain(self.source.observed_api_version()),
        ));
        if let Some(resource) = resource {
            diagnostic = diagnostic.with_field(DiagnosticField::new("resource", DiagnosticValue::plain(resource)));
        }
        if let Some(path) = finding.field_path() {
            diagnostic = diagnostic.with_field(DiagnosticField::new("native_path", DiagnosticValue::plain(path)));
        }
        self.diagnostics.push(diagnostic.with_native_finding(native));
        self.push_loss(
            format!("podman.{stage}.{}", code.as_str()),
            ConversionKind::Unsupported,
            self.importer.unsupported.clone(),
        );
    }

    fn native_inventory_findings(
        &mut self,
        findings: &[(ResourceKind, Option<&'static str>, podman_lens::InventoryFinding)],
    ) {
        let Some((section_kind, observation_state, finding)) = findings.first() else {
            return;
        };
        let code = finding.code();
        let stage = "acquisition";
        let reason = podman_native_summary(code);
        let resource = finding.resource().map(resource_locator);
        let subject = resource
            .clone()
            .unwrap_or_else(|| format!("podman.inventory.{}", resource_kind_name(*section_kind)));
        let native = NativeFinding::new("podman", "podman-lens", code.as_str(), stage, Severity::Warning, reason)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)));
        let mut diagnostic = diagnostic_with_context(
            self.importer.unsupported.clone(),
            Severity::Warning,
            reason,
            &subject,
            reason,
            "omitted",
            Some("partial"),
        )
        .with_field(DiagnosticField::new(
            "available_promotion",
            DiagnosticValue::plain("none"),
        ));

        let mut context = vec![
            DiagnosticField::new(
                "resource_kind",
                DiagnosticValue::plain(resource_kind_name(*section_kind)),
            ),
            DiagnosticField::new("observation_origin", DiagnosticValue::plain(stage)),
            DiagnosticField::new(
                "source_engine",
                DiagnosticValue::plain(self.source.observed_engine_version()),
            ),
            DiagnosticField::new("source_api", DiagnosticValue::plain(self.source.observed_api_version())),
            DiagnosticField::new("occurrence_count", DiagnosticValue::plain(findings.len().to_string())),
        ];
        if let Some(resource) = resource {
            context.push(DiagnosticField::new("resource", DiagnosticValue::plain(resource)));
        }
        if let Some(state) = observation_state {
            context.push(DiagnosticField::new(
                "observation_state",
                DiagnosticValue::plain(*state),
            ));
        }
        let paths = findings
            .iter()
            .filter_map(|(_, _, finding)| finding.field_path())
            .collect::<BTreeSet<_>>();
        self.append_native_field_context(&mut context, code, finding, &paths);
        let mut native = native;
        for field in context {
            native = native.with_field(field.clone());
            diagnostic = diagnostic.with_field(field);
        }
        self.diagnostics.push(diagnostic.with_native_finding(native));
        for _ in findings {
            self.push_loss(
                format!("podman.{stage}.{}", code.as_str()),
                ConversionKind::Unsupported,
                self.importer.unsupported.clone(),
            );
        }
    }

    fn append_native_field_context(
        &self,
        context: &mut Vec<DiagnosticField>,
        code: podman_lens::DiagnosticCode,
        finding: &podman_lens::InventoryFinding,
        paths: &BTreeSet<&str>,
    ) {
        match code {
            podman_lens::DiagnosticCode::UnknownFieldOverflow => {
                self.append_unknown_field_limit_context(context, finding);
                context.push(DiagnosticField::new(
                    "path_descriptor_purpose",
                    DiagnosticValue::plain("audit which Podman response fields were not typed or converted"),
                ));
                context.push(DiagnosticField::new(
                    "conversion_impact",
                    DiagnosticValue::plain(
                        "only the diagnostic path catalogue was truncated; typed observations used by conversion remain intact",
                    ),
                ));
            }
            podman_lens::DiagnosticCode::NativeFieldUnsupported => append_native_paths(context, paths),
            _ => {
                append_native_paths(context, paths);
                return;
            }
        }
        context.push(DiagnosticField::new(
            "native_value_policy",
            DiagnosticValue::plain("field paths retained; native values not retained"),
        ));
    }

    fn append_unknown_field_limit_context(
        &self,
        context: &mut Vec<DiagnosticField>,
        finding: &podman_lens::InventoryFinding,
    ) {
        context.push(DiagnosticField::new(
            "retention_limit_per_resource",
            DiagnosticValue::plain(MAX_UNKNOWN_FIELDS_PER_RECORD.to_string()),
        ));
        context.push(DiagnosticField::new(
            "retention_limit_per_inventory",
            DiagnosticValue::plain(MAX_UNKNOWN_FIELDS_PER_INVENTORY.to_string()),
        ));
        context.push(DiagnosticField::new(
            "discarded_native_path_count_at_least",
            DiagnosticValue::plain("1"),
        ));
        let Some(observation) = finding
            .resource()
            .and_then(|identity| self.source.inventory().observation(identity))
        else {
            return;
        };
        let retained = observation.header().unmodelled_fields();
        context.push(DiagnosticField::new(
            "retained_native_path_count",
            DiagnosticValue::plain(retained.len().to_string()),
        ));
        if !retained.is_empty() {
            context.push(DiagnosticField::new(
                "native_path_samples",
                DiagnosticValue::plain(
                    retained
                        .iter()
                        .map(podman_lens::UnmodelledField::path)
                        .take(8)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ));
            context.push(DiagnosticField::new(
                "native_path_samples_shown",
                DiagnosticValue::plain(retained.len().min(8).to_string()),
            ));
        }
    }

    fn invalid_observation(&mut self, subject: impl Into<String>, summary: &'static str, identity: &ResourceIdentity) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.invalid.clone(),
                Severity::Error,
                summary,
                &subject,
                summary,
                "rejected",
                None,
            )
            .with_field(DiagnosticField::new(
                "source_engine",
                DiagnosticValue::plain(self.source.observed_engine_version()),
            ))
            .with_field(DiagnosticField::new(
                "source_api",
                DiagnosticValue::plain(self.source.observed_api_version()),
            ))
            .with_field(DiagnosticField::new(
                "resource",
                DiagnosticValue::plain(resource_locator(identity)),
            )),
        );
        self.push_loss(subject, ConversionKind::Invalid, self.importer.invalid.clone());
    }

    fn exact(&mut self, subject: impl Into<String>) {
        self.outcomes
            .push(ConversionOutcome::exact(subject).with_origin(self.origin.clone()));
    }

    fn approximate_with_flag(&mut self, subject: impl Into<String>, summary: &'static str, flag: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.policy.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "approximated",
                Some("approximate"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain(flag),
            ))
            .with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain("effective"),
            )),
        );
        self.push_loss(subject, ConversionKind::Approximate, self.importer.policy.clone());
    }

    fn promotion_required(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.policy.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "not-promoted",
                Some("partial"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain("none"),
            ))
            .with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain("effective"),
            )),
        );
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.policy.clone());
    }

    fn promotion_required_with_flag(&mut self, subject: impl Into<String>, summary: &'static str, flag: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.policy.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "not-promoted",
                Some("partial"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain(flag),
            ))
            .with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain("effective"),
            )),
        );
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.policy.clone());
    }

    fn unsupported(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.unsupported.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "omitted",
                Some("partial"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain("none"),
            )),
        );
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.unsupported.clone());
    }

    fn unsupported_with_origin(
        &mut self,
        subject: impl Into<String>,
        summary: &'static str,
        origin: ObservationOrigin,
    ) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.unsupported.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "omitted",
                Some("partial"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain("none"),
            ))
            .with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain(observation_origin_name(origin)),
            )),
        );
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.unsupported.clone());
    }

    fn identity_conflict(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(diagnostic_with_context(
            self.importer.identity.clone(),
            Severity::Error,
            summary,
            &subject,
            summary,
            "rejected",
            None,
        ));
        self.push_loss(subject, ConversionKind::Invalid, self.importer.identity.clone());
    }

    fn secret_incomplete(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            diagnostic_with_context(
                self.importer.secret.clone(),
                Severity::Warning,
                summary,
                &subject,
                summary,
                "omitted",
                Some("partial"),
            )
            .with_field(DiagnosticField::new(
                "available_promotion",
                DiagnosticValue::plain("none"),
            )),
        );
        self.push_loss(subject, ConversionKind::Unsupported, self.importer.secret.clone());
    }

    fn invalid(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(diagnostic_with_context(
            self.importer.invalid.clone(),
            Severity::Error,
            summary,
            &subject,
            summary,
            "rejected",
            None,
        ));
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

fn diagnostic_with_context(
    code: DiagnosticCode,
    severity: Severity,
    summary: impl Into<String>,
    subject: &str,
    reason: &str,
    decision: &'static str,
    required_loss_policy: Option<&'static str>,
) -> Diagnostic {
    let diagnostic = Diagnostic::new(code, severity, summary)
        .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
        .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)))
        .with_field(DiagnosticField::new("decision", DiagnosticValue::plain(decision)));
    required_loss_policy.map_or(diagnostic.clone(), |policy| {
        diagnostic.with_field(DiagnosticField::new(
            "required_loss_policy",
            DiagnosticValue::plain(policy),
        ))
    })
}

fn append_native_paths(context: &mut Vec<DiagnosticField>, paths: &BTreeSet<&str>) {
    if paths.len() == 1 {
        if let Some(path) = paths.first() {
            context.push(DiagnosticField::new("native_path", DiagnosticValue::plain(*path)));
        }
    } else if !paths.is_empty() {
        context.push(DiagnosticField::new(
            "native_path_count",
            DiagnosticValue::plain(paths.len().to_string()),
        ));
        context.push(DiagnosticField::new(
            "native_path_samples",
            DiagnosticValue::plain(paths.iter().take(8).copied().collect::<Vec<_>>().join(", ")),
        ));
        context.push(DiagnosticField::new(
            "native_path_samples_shown",
            DiagnosticValue::plain(paths.len().min(8).to_string()),
        ));
    }
}

fn decoded_mount_options(options: &[String]) -> Result<(Option<SelinuxRelabel>, bool), ()> {
    // This can preserve only relabel intent exposed through typed `Mounts[].Options`.
    // Engines may retain the authored spelling solely in `HostConfig.Binds`; that
    // native field remains value-free/unmodelled until PodmanLens exposes it safely.
    let shared = options.iter().any(|option| option == "z");
    let private = options.iter().any(|option| option == "Z");
    if shared && private {
        return Err(());
    }
    let relabel = if shared {
        Some(SelinuxRelabel::Shared)
    } else if private {
        Some(SelinuxRelabel::Private)
    } else {
        None
    };
    let complete = options
        .iter()
        .all(|option| matches!(option.as_str(), "" | "ro" | "rw" | "bind" | "rbind" | "z" | "Z"));
    Ok((relabel, complete))
}

fn lease_range_spelling(start: IpAddr, end: IpAddr) -> Option<String> {
    match (start, end) {
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)) => Some(format!("{start}-{end}")),
        _ => None,
    }
}

fn podman_native_summary(code: podman_lens::DiagnosticCode) -> &'static str {
    match code {
        podman_lens::DiagnosticCode::UnknownFieldOverflow => {
            "PodmanLens truncated its diagnostic list of unmapped field paths; mapped Podman observations were not discarded"
        }
        podman_lens::DiagnosticCode::NativeFieldUnsupported => {
            "PodmanLens found native response fields without typed portable mappings; path descriptors were retained without values"
        }
        podman_lens::DiagnosticCode::AdvisoryLabelIncomplete => {
            "optional Podman application-grouping labels are incomplete; affected resources remain ungrouped"
        }
        podman_lens::DiagnosticCode::AdvisoryLabelConflict => {
            "optional Podman application-grouping labels disagree; affected resources remain ungrouped"
        }
        _ => podman_lens::Diagnostic::new(code).message(),
    }
}

fn is_compose_lifecycle_label(name: &str) -> bool {
    name.starts_with(COMPOSE_LIFECYCLE_LABEL_PREFIX)
}

const fn observation_state_name(state: ResourceObservationState) -> &'static str {
    match state {
        ResourceObservationState::Complete => "complete",
        ResourceObservationState::Unavailable => "unavailable",
        ResourceObservationState::Malformed => "malformed",
        _ => "unknown",
    }
}

const fn observation_origin_name(origin: ObservationOrigin) -> &'static str {
    match origin {
        ObservationOrigin::Configured => "configured",
        ObservationOrigin::Effective => "effective",
        ObservationOrigin::RuntimeAssigned => "runtime-assigned",
        ObservationOrigin::LocalResolution => "local-resolution",
        _ => "unknown",
    }
}

fn resource_locator(identity: &ResourceIdentity) -> String {
    let kind = resource_kind_name(identity.kind());
    let reference = identity.name().unwrap_or(identity.id());
    format!("{kind}:{reference}")
}

fn resource_kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Container => "container",
        ResourceKind::Image => "image",
        ResourceKind::Network => "network",
        ResourceKind::Pod => "pod",
        ResourceKind::Secret => "secret",
        ResourceKind::Volume => "volume",
        _ => "unknown",
    }
}

fn discovery_resource_locator(finding: &podman_lens::DiscoveryFinding) -> Option<String> {
    if let Some(resource) = finding.resource_identity() {
        return Some(resource_locator(resource));
    }
    if let Some(selector) = finding.selector() {
        return Some(format!(
            "{}:{}",
            resource_kind_name(selector.kind()),
            selector.reference()
        ));
    }
    finding
        .label_selector()
        .map(|selector| format!("label:{}", selector.name()))
}

fn invalid_inventory_finding(code: podman_lens::DiagnosticCode) -> bool {
    matches!(
        code,
        podman_lens::DiagnosticCode::InventoryHttpStatus
            | podman_lens::DiagnosticCode::InventoryJson
            | podman_lens::DiagnosticCode::InventoryShape
            | podman_lens::DiagnosticCode::ResourceUnavailable
            | podman_lens::DiagnosticCode::ResourceMalformed
            | podman_lens::DiagnosticCode::RelationshipConflict
            | podman_lens::DiagnosticCode::UnresolvedRelationship
            | podman_lens::DiagnosticCode::PodMembershipConflict
    )
}

fn invalid_discovery_finding(code: podman_lens::DiagnosticCode) -> bool {
    matches!(
        code,
        podman_lens::DiagnosticCode::SelectorUnresolved
            | podman_lens::DiagnosticCode::SelectorAmbiguous
            | podman_lens::DiagnosticCode::RelationshipAmbiguous
    )
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
