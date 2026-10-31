//! Conservative typed Docker observations to neutral intent.
//!
//! Compound intent retains reconstruction provenance from every consumed child.
//! Configured host port zero remains an ephemeral publication request; runtime
//! port allocations never become neutral desired state.
//! Bind relabel modes consume only `DockerLens`'s closed interpretation, with independent
//! access corroboration and same-host promotion. This preserves observed syntax, not
//! original Compose authorship or proof that the daemon enforced `SELinux` relabelling.
//! Stop behavior consumes only typed observations under the portable process policy;
//! missing values never infer image defaults. Finite whole-second timeouts preserve zero,
//! while negative/infinite and out-of-range values remain native evidence.
//! Unknown source daemon mode remains a warning, not lost portable intent or target
//! authority. Mode-sensitive mappings must independently require their source prerequisites.

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, NativeFinding, RuleId, Severity,
};
use boxferry_model::{
    Application, Command, Entrypoint, EnvironmentValue, EnvironmentVariable, ExposedPort,
    Healthcheck as NeutralHealthcheck, HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, Identifier,
    ImageReference, MetadataLabel, Mount, MountSource, Network, NetworkAttachment as NeutralNetworkAttachment, Port,
    ProtectedString, Protocol, Provenance, ResourceOwnership, RestartPolicy as NeutralRestartPolicy, SelinuxRelabel,
    Service, SourceId, Sourced, StopTimeout, Volume,
};
use docker_lens::{
    decoder::{
        CommandValue, ContainerObservation, DecodedInventory, EnvironmentAssignment, Healthcheck as NativeHealthcheck,
        HealthcheckTest, LabelObservation, MountAccess, MountKind, MountModeInterpretation, MountObservation,
        NetworkAttachment as NativeNetworkAttachment, NetworkModeKind, NetworkObservation, PortKey, PortObservation,
        RestartPolicy as NativeRestartPolicy, TransportProtocol, VolumeObservation,
    },
    finding::{Finding, FindingCode, Severity as NativeSeverity},
    observation::{Availability, BindRelabel, FieldPath, Observed, Origin},
    version::DaemonMode,
};

use crate::{
    DockerPromotionPolicy, DockerPublicLabelDecision, DockerPublicLabelDecisions, DockerPublicLabelResourceKind,
    DockerResourceAction, DockerResourceDecision, DockerResourceDecisions, DockerResourceKind, DockerSelection,
    DockerSource,
};

/// Source-only `DockerLens` importer. Target planning remains a separate checkpoint.
#[derive(Clone, Debug)]
pub struct DockerImporter {
    invalid: DiagnosticCode,
    unsupported: DiagnosticCode,
    reconstructed: DiagnosticCode,
    approximation: DiagnosticCode,
}

impl DockerImporter {
    /// Validates the adapter's static diagnostic codes.
    ///
    /// # Errors
    ///
    /// Returns an error only if a repository-owned code is malformed.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            invalid: RuleId::DockerSourceInvalid.definition().diagnostic_code()?,
            unsupported: RuleId::DockerInputUnsupported.definition().diagnostic_code()?,
            reconstructed: RuleId::DockerStateReconstructed.definition().diagnostic_code()?,
            approximation: RuleId::DockerInputApproximation.definition().diagnostic_code()?,
        })
    }
}

impl ImportAdapter for DockerImporter {
    type Source = DockerSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let Ok(source_id) = SourceId::new("docker") else {
            return ImportResult::new(
                None,
                Vec::new(),
                vec![Diagnostic::new(
                    self.invalid.clone(),
                    Severity::Error,
                    "Docker provenance identity is invalid",
                )],
            );
        };
        Mapping {
            importer: self,
            origin: Provenance::runtime_observation(source_id),
            policy: source.promotion_policy(),
            decisions: source.resource_decisions().clone(),
            public_labels: source.public_label_decisions(),
            application: Application::new(source.application_name().clone()),
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
        }
        .map(source)
    }
}

struct Mapping<'a> {
    importer: &'a DockerImporter,
    origin: Provenance,
    policy: DockerPromotionPolicy,
    decisions: DockerResourceDecisions,
    public_labels: &'a DockerPublicLabelDecisions,
    application: Application,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl Mapping<'_> {
    fn map(mut self, source: &DockerSource) -> ImportResult {
        let inventory = source.inventory();
        let resources_valid = self.validate_resource_decisions(inventory);
        let labels_valid = self.validate_public_label_decisions(inventory);
        if !resources_valid || !labels_valid {
            return ImportResult::new(None, self.outcomes, self.diagnostics);
        }
        self.exact("application");
        if inventory.containers.is_empty() && inventory.networks.is_empty() && inventory.volumes.is_empty() {
            let discovery =
                source.selection() == DockerSelection::Discovery || !inventory.discovered_containers.is_empty();
            self.selection_required(if discovery {
                "Docker discovery contains list metadata, not inspected application resources"
            } else {
                "Docker selection contains no inspected application resources"
            });
        }
        for (index, container) in inventory.containers.iter().enumerate() {
            self.map_container(
                index,
                container,
                &inventory.containers,
                &inventory.networks,
                &inventory.volumes,
            );
        }
        for (index, network) in inventory.networks.iter().enumerate() {
            self.map_network(index, network, &inventory.networks);
        }
        for (index, volume) in inventory.volumes.iter().enumerate() {
            self.map_volume(index, volume);
        }
        for finding in &inventory.findings {
            self.native_finding(finding, inventory.version.daemon.mode);
        }
        ImportResult::new(Some(self.application), self.outcomes, self.diagnostics)
    }

    fn validate_public_label_decisions(&mut self, inventory: &DecodedInventory) -> bool {
        let mut valid = true;
        let mut copied_bytes = 0_usize;
        let declarations = self.public_labels;
        for (index, decision) in declarations.entries().iter().enumerate() {
            let subject = format!("application.public_label_decisions[{index}]");
            match selected_public_label(inventory, decision) {
                Ok((key, value)) => {
                    let total = copied_bytes
                        .checked_add(key.len())
                        .and_then(|sum| sum.checked_add(value.len()));
                    if let Some(total) = total.filter(|sum| *sum <= crate::public_labels::PUBLIC_LABEL_TOTAL_LIMIT) {
                        copied_bytes = total;
                    } else {
                        self.invalid(
                            subject,
                            "Docker public-label declarations exceed the BoxFerry aggregate reconstruction budget",
                        );
                        valid = false;
                    }
                }
                Err(summary) => {
                    self.invalid(subject, summary);
                    valid = false;
                }
            }
        }
        valid
    }

    fn map_public_labels(
        &mut self,
        subject: &str,
        kind: DockerPublicLabelResourceKind,
        source: Option<&[u8]>,
        observed: &Observed<Vec<LabelObservation>>,
    ) -> Vec<Sourced<MetadataLabel>> {
        let selected = |decision: &DockerPublicLabelDecision| {
            decision.kind() == kind && Some(decision.source().as_bytes()) == source
        };
        if !self.public_labels.entries().iter().any(selected) {
            self.unmapped(format!("{subject}.labels"), observed);
            return Vec::new();
        }
        let mut mapped = Vec::new();
        let mut unselected = false;
        if let Some(labels) = observed.value() {
            for (index, label) in labels.iter().enumerate() {
                let path = format!("{subject}.labels[{index}]");
                if !self
                    .public_labels
                    .entries()
                    .iter()
                    .any(|decision| selected(decision) && decision.key().as_bytes() == label.key.as_bytes())
                {
                    unselected = true;
                    continue;
                }
                let text = std::str::from_utf8(label.key.as_bytes()).ok().and_then(|key| {
                    label
                        .value
                        .value()
                        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
                        .map(|value| (key, value))
                });
                let Some((key, value)) = text else {
                    self.invalid(path, "Docker declared public label lacks representable observed text");
                    continue;
                };
                let Ok(name) = Identifier::new(key) else {
                    self.invalid(path, "Docker declared public label key cannot enter neutral metadata");
                    continue;
                };
                mapped.push(self.decision_sourced(MetadataLabel::new(name, ProtectedString::plain(value))));
                self.mapped(path.clone(), true);
                if let Some(outcome) = self.outcomes.last_mut() {
                    *outcome = ConversionOutcome::exact(path)
                        .with_origin(self.origin.clone())
                        .with_origin(Provenance::user_override(self.origin.source_id().clone()))
                        .with_origin(Provenance::conversion_decision(self.origin.source_id().clone()));
                }
            }
        }
        if unselected {
            self.unsupported(
                format!("{subject}.labels"),
                "Unselected Docker labels remain protected native evidence without exact public declarations",
                Some(observed.origin),
            );
        }
        mapped
    }

    fn validate_resource_decisions(&mut self, inventory: &DecodedInventory) -> bool {
        let mut valid = true;
        for decision in self.decisions.entries().to_vec() {
            match decision.kind() {
                DockerResourceKind::Network => {
                    let matched = inventory
                        .networks
                        .iter()
                        .filter(|network| runtime_network_id(&network.id) == Some(decision.source().as_bytes()))
                        .collect::<Vec<_>>();
                    if matched.len() != 1 {
                        self.invalid(
                            "application.resource_decisions",
                            "Docker network decision does not uniquely match selected inspected inventory",
                        );
                        valid = false;
                    } else if decision.action() == DockerResourceAction::Create {
                        let network = matched[0];
                        let observed_name = network
                            .name
                            .value()
                            .map(docker_lens::evidence::ProtectedValue::as_bytes);
                        if network.name.availability != Availability::Present
                            || network.name.origin != Origin::Effective
                            || observed_name.is_none_or(|name| {
                                !std::str::from_utf8(name).is_ok_and(crate::resource_decisions::valid_name)
                                    || name == b"bridge"
                                    || name == b"host"
                                    || name == b"none"
                            })
                        {
                            self.invalid(
                                "application.resource_decisions",
                                "Docker built-in or unidentified network cannot receive create intent",
                            );
                            valid = false;
                        } else if !selected_active_members_only(network, &inventory.containers) {
                            self.invalid(
                                "application.resource_decisions",
                                "Docker network has an active member outside provably selected containers",
                            );
                            valid = false;
                        }
                    }
                }
                DockerResourceKind::Volume => {
                    let matches = inventory
                        .volumes
                        .iter()
                        .filter(|volume| {
                            volume.name.availability == Availability::Present
                                && volume.name.origin == Origin::Effective
                                && volume
                                    .name
                                    .value()
                                    .is_some_and(|name| name.as_bytes() == decision.source().as_bytes())
                        })
                        .count();
                    if matches != 1 {
                        self.invalid(
                            "application.resource_decisions",
                            "Docker volume decision does not uniquely match selected inspected inventory",
                        );
                        valid = false;
                    }
                }
            }
        }
        valid
    }

    fn decision_for(&self, kind: DockerResourceKind, source: &[u8]) -> Option<&DockerResourceDecision> {
        self.decisions
            .entries()
            .iter()
            .find(|decision| decision.kind() == kind && decision.source().as_bytes() == source)
    }

    fn map_container(
        &mut self,
        index: usize,
        container: &ContainerObservation,
        containers: &[ContainerObservation],
        networks: &[NetworkObservation],
        volumes: &[VolumeObservation],
    ) {
        let subject = format!("services[{index}]");
        // The generated key is deliberately independent of the protected effective native name.
        let Ok(name) = Identifier::new(format!("docker-service-{}", index + 1)) else {
            self.invalid(subject, "generated service identity is invalid");
            return;
        };
        let mut service = Service::new(name);
        self.map_container_runtime_name(&subject, container, containers, &mut service);
        self.map_image(&subject, &container.image, &mut service);
        self.map_command(&subject, &container.command, &mut service);
        self.map_entrypoint(&subject, &container.entrypoint, &mut service);
        self.map_user(&subject, &container.user, &mut service);
        self.map_working_directory(&subject, &container.working_directory, &mut service);
        self.map_stop_signal(&subject, &container.runtime.stop_signal, &mut service);
        self.map_stop_timeout(&subject, &container.runtime.stop_timeout, &mut service);
        self.map_restart_policy(&subject, &container.restart_policy, &mut service);
        self.map_healthcheck(&subject, &container.healthcheck, &mut service);
        self.map_exposed_ports(&subject, &container.exposed_ports, &mut service);
        self.map_configured_ports(&subject, &container.configured_ports, &mut service);
        self.map_environment(&subject, &container.environment, &mut service);
        self.map_network_membership(&subject, container, networks, &mut service);
        self.map_mounts(&subject, &container.mounts, volumes, &mut service);
        for label in self.map_public_labels(
            &subject,
            DockerPublicLabelResourceKind::Container,
            container
                .id
                .value()
                .map(docker_lens::evidence::ProtectedValue::as_bytes),
            &container.labels,
        ) {
            service.add_label(label);
        }
        self.unmapped_container_fields(&subject, container);
        if self
            .application
            .add_service(Sourced::from_source(service, self.origin.clone()))
            .is_err()
        {
            self.invalid(subject, "Docker service conflicts with another neutral resource");
        }
    }

    fn map_container_runtime_name(
        &mut self,
        subject: &str,
        container: &ContainerObservation,
        containers: &[ContainerObservation],
        service: &mut Service,
    ) {
        let path = format!("{subject}.runtime_name");
        if !self.policy.promotes_portable_effective_settings() {
            self.unsupported(
                path,
                "inspected Docker runtime identity needs explicit portable source-state promotion",
                Some(container.name.origin),
            );
            return;
        }
        let selected = container
            .name
            .value()
            .map(docker_lens::evidence::ProtectedValue::as_bytes);
        let name = selected.and_then(|value| value.strip_prefix(b"/"));
        let correlated = container.name.availability == Availability::Present
            && container.name.origin == Origin::Effective
            && name.is_some_and(valid_portable_network_alias)
            && containers
                .iter()
                .filter(|candidate| candidate.reference == container.reference)
                .count()
                == 1
            && containers
                .iter()
                .filter(|candidate| {
                    candidate
                        .name
                        .value()
                        .is_some_and(|value| Some(value.as_bytes()) == selected)
                })
                .count()
                == 1;
        if !correlated {
            self.unsupported(
                path,
                "inspected Docker name is unavailable, invalid, or not uniquely correlated with the selected container",
                Some(container.name.origin),
            );
            return;
        }
        if let Some(value) = name.and_then(|value| std::str::from_utf8(value).ok()) {
            service.set_runtime_name(self.sourced(ProtectedString::plain(value), true));
            self.approximate_observed(
                path,
                "inspected Docker runtime name is effective state, not proof of an authored service identity",
                container.name.origin,
            );
        }
    }

    fn map_network(&mut self, index: usize, observed: &NetworkObservation, networks: &[NetworkObservation]) {
        let subject = format!("networks[{index}]");
        let Ok(name) = Identifier::new(format!("docker-network-{}", index + 1)) else {
            self.invalid(subject, "generated network identity is invalid");
            return;
        };
        let decision = runtime_network_id(&observed.id)
            .and_then(|id| self.decision_for(DockerResourceKind::Network, id))
            .cloned();
        let ownership = decision
            .as_ref()
            .map_or(ResourceOwnership::Uncertain, |choice| match choice.action() {
                DockerResourceAction::Create => ResourceOwnership::Application,
                DockerResourceAction::External => ResourceOwnership::External,
            });
        let mut network = Network::new(name, ownership);
        if decision.is_none() {
            self.unsupported(
                format!("{subject}.ownership"),
                "Docker inspection does not establish who should manage the network",
                None,
            );
        }
        if !matches!(observed.id.availability, Availability::Missing | Availability::Null) {
            self.observation_only(
                format!("{subject}.id"),
                "Docker inspected network ID is observation-only, not a portable resource name",
                observed.id.origin,
            );
        }
        self.map_bridge_network_settings(&subject, observed, networks, &mut network, decision.is_some());
        if let Some(decision) = &decision {
            network.set_runtime_name(self.decision_sourced(ProtectedString::plain(decision.target())));
            self.explicit_resource_decision(&subject);
        }
        self.unmapped(format!("{subject}.enable_ipv6"), &observed.enable_ipv6);
        self.unmapped(format!("{subject}.ipam_driver"), &observed.ipam_driver);
        self.unmapped(format!("{subject}.ipam_configs"), &observed.ipam_configs);
        self.unmapped(format!("{subject}.options"), &observed.options);
        let labels = self.map_public_labels(
            &subject,
            DockerPublicLabelResourceKind::Network,
            observed.id.value().map(docker_lens::evidence::ProtectedValue::as_bytes),
            &observed.labels,
        );
        if let Some(first) = labels.first() {
            let origins = first.origins().to_vec();
            network.set_labels_with_origins(labels, origins);
        }
        let mut sourced = Sourced::from_source(network, self.origin.clone());
        if decision.is_some() {
            self.add_decision_origins(&mut sourced);
        }
        if self.application.add_network(sourced).is_err() {
            self.invalid(subject, "Docker network conflicts with another neutral resource");
        }
    }

    fn map_bridge_network_settings(
        &mut self,
        subject: &str,
        observed: &NetworkObservation,
        networks: &[NetworkObservation],
        network: &mut Network,
        target_decided: bool,
    ) {
        let retain_unmapped = |mapping: &mut Self| {
            if !target_decided {
                mapping.unmapped(format!("{subject}.runtime_name"), &observed.name);
            }
            mapping.unmapped(format!("{subject}.driver"), &observed.driver);
            mapping.unmapped(format!("{subject}.internal"), &observed.internal);
        };
        if !self.policy.promotes_inspected_bridge_network_settings() {
            retain_unmapped(self);
            return;
        }
        let Some(id) = runtime_network_id(&observed.id) else {
            self.unsupported(
                format!("{subject}.settings"),
                "inspected bridge settings require a usable typed runtime network ID",
                Some(observed.id.origin),
            );
            retain_unmapped(self);
            return;
        };
        if networks
            .iter()
            .filter(|candidate| runtime_network_id(&candidate.id) == Some(id))
            .count()
            != 1
        {
            self.unsupported(
                format!("{subject}.settings"),
                "duplicate inspected Docker network ID makes bridge settings ambiguous",
                Some(observed.id.origin),
            );
            retain_unmapped(self);
            return;
        }
        if observed.driver.availability != Availability::Present
            || observed.driver.origin != Origin::Effective
            || observed
                .driver
                .value()
                .is_none_or(|driver| driver.as_bytes() != b"bridge")
        {
            self.unsupported(
                format!("{subject}.driver"),
                "only an effective inspected bridge driver is reviewed for neutral promotion",
                Some(observed.driver.origin),
            );
            self.unmapped(format!("{subject}.runtime_name"), &observed.name);
            self.unmapped(format!("{subject}.internal"), &observed.internal);
            return;
        }
        network.set_driver(self.sourced(ProtectedString::plain("bridge"), true));
        self.approximate_observed(
            format!("{subject}.driver"),
            "inspected Docker bridge driver is effective state, not authored network intent",
            observed.driver.origin,
        );

        if !target_decided {
            self.map_bridge_network_name(subject, observed, networks, id, network);
        }
        self.map_bridge_network_internal(subject, observed, network);
    }

    fn map_bridge_network_name(
        &mut self,
        subject: &str,
        observed: &NetworkObservation,
        networks: &[NetworkObservation],
        id: &[u8],
        network: &mut Network,
    ) {
        if !matches!(observed.name.availability, Availability::Missing | Availability::Null) {
            if observed.name.origin == Origin::Effective
                && observed.name.availability == Availability::Present
                && observed
                    .name
                    .value()
                    .is_some_and(|name| valid_native_network_name(name.as_bytes()))
                && !networks.iter().any(|candidate| {
                    runtime_network_id(&candidate.id) != Some(id)
                        && candidate.name.value().is_some_and(|name| {
                            observed
                                .name
                                .value()
                                .is_some_and(|selected| name.as_bytes() == selected.as_bytes())
                        })
                })
            {
                if let Some(value) = observed.name.value() {
                    // Native identity is protected even though the neutral resource key is synthetic.
                    if let Some(value) = self.protected_text(&format!("{subject}.runtime_name"), value) {
                        network.set_runtime_name(self.sourced(ProtectedString::plain(value), true));
                        self.approximate_observed(
                            format!("{subject}.runtime_name"),
                            "inspected Docker network name is effective state, not authored ownership",
                            observed.name.origin,
                        );
                    }
                }
            } else {
                self.unsupported(
                    format!("{subject}.runtime_name"),
                    "inspected Docker network name is unavailable, invalid, or ambiguous",
                    Some(observed.name.origin),
                );
            }
        }
    }

    fn map_bridge_network_internal(&mut self, subject: &str, observed: &NetworkObservation, network: &mut Network) {
        if !matches!(
            observed.internal.availability,
            Availability::Missing | Availability::Null
        ) {
            if observed.internal.origin == Origin::Effective && observed.internal.availability == Availability::Present
            {
                if let Some(value) = observed.internal.value() {
                    network.set_internal(self.sourced(*value, true));
                    self.approximate_observed(
                        format!("{subject}.internal"),
                        "inspected Docker internal flag is effective state, not authored network intent",
                        observed.internal.origin,
                    );
                } else {
                    self.unsupported(
                        format!("{subject}.internal"),
                        "inspected Docker internal flag has no usable value",
                        Some(observed.internal.origin),
                    );
                }
            } else {
                self.unsupported(
                    format!("{subject}.internal"),
                    "inspected Docker internal flag has no reviewed effective value",
                    Some(observed.internal.origin),
                );
            }
        }
    }

    fn map_volume(&mut self, index: usize, observed: &VolumeObservation) {
        let subject = format!("volumes[{index}]");
        let Ok(name) = Identifier::new(format!("docker-volume-{}", index + 1)) else {
            self.invalid(subject, "generated volume identity is invalid");
            return;
        };
        let decision = observed
            .name
            .value()
            .and_then(|name| self.decision_for(DockerResourceKind::Volume, name.as_bytes()))
            .cloned();
        let ownership = decision
            .as_ref()
            .map_or(ResourceOwnership::Uncertain, |choice| match choice.action() {
                DockerResourceAction::Create => ResourceOwnership::Application,
                DockerResourceAction::External => ResourceOwnership::External,
            });
        let mut volume = Volume::new(name, ownership);
        if let Some(decision) = &decision {
            volume.set_runtime_name(self.decision_sourced(ProtectedString::plain(decision.target())));
            self.explicit_resource_decision(&subject);
        } else {
            self.unsupported(
                format!("{subject}.ownership"),
                "Docker inspection does not establish who should manage the volume",
                None,
            );
            self.unmapped(format!("{subject}.runtime_name"), &observed.name);
        }
        self.unmapped(format!("{subject}.driver"), &observed.driver);
        self.unmapped(format!("{subject}.mountpoint"), &observed.mountpoint);
        self.unmapped(format!("{subject}.options"), &observed.options);
        let labels = self.map_public_labels(
            &subject,
            DockerPublicLabelResourceKind::Volume,
            observed
                .name
                .value()
                .map(docker_lens::evidence::ProtectedValue::as_bytes),
            &observed.labels,
        );
        if let Some(first) = labels.first() {
            let origins = first.origins().to_vec();
            volume.set_labels_with_origins(labels, origins);
        }
        let mut sourced = Sourced::from_source(volume, self.origin.clone());
        if decision.is_some() {
            self.add_decision_origins(&mut sourced);
        }
        if self.application.add_volume(sourced).is_err() {
            self.invalid(subject, "Docker volume conflicts with another neutral resource");
        }
    }

    fn map_image(
        &mut self,
        subject: &str,
        observed: &Observed<docker_lens::evidence::ProtectedValue>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.image");
        if observed.availability == Availability::Missing || observed.availability == Availability::Null {
            self.invalid(path, "selected Docker container has no image reference");
            return;
        }
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let Ok(text) = std::str::from_utf8(value.as_bytes()) else {
            self.invalid(path, "Docker image reference is not UTF-8");
            return;
        };
        if text
            .strip_prefix("sha256:")
            .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            self.unsupported(
                path,
                "Docker local image ID is not a portable named image; supply a named image reference",
                Some(observed.origin),
            );
            return;
        }
        let Ok(image) = ImageReference::parse(text.to_owned()) else {
            self.invalid(path, "Docker image reference is invalid for neutral intent");
            return;
        };
        service.set_image(self.sourced(image, promoted));
        self.mapped(path, promoted);
    }

    fn map_command(&mut self, subject: &str, observed: &Observed<CommandValue>, service: &mut Service) {
        let path = format!("{subject}.command");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let command = match value {
            CommandValue::Exec(parts) if parts.is_empty() => Command::Empty,
            CommandValue::Exec(parts) => {
                let Some(parts) = self.protected_arguments(&path, parts) else {
                    return;
                };
                Command::Exec(parts)
            }
            CommandValue::Shell(_) => {
                self.unsupported(
                    path,
                    "Docker shell command semantics need separate target review",
                    Some(observed.origin),
                );
                return;
            }
        };
        service.set_command(self.sourced(command, promoted));
        self.mapped(path, promoted);
    }

    fn map_entrypoint(&mut self, subject: &str, observed: &Observed<CommandValue>, service: &mut Service) {
        let path = format!("{subject}.entrypoint");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let entrypoint = match value {
            CommandValue::Exec(parts) if parts.is_empty() => Entrypoint::Empty,
            CommandValue::Exec(parts) => {
                let Some(parts) = self.protected_arguments(&path, parts) else {
                    return;
                };
                Entrypoint::Exec(parts)
            }
            CommandValue::Shell(_) => {
                self.unsupported(
                    path,
                    "Docker shell entrypoint semantics need separate target review",
                    Some(observed.origin),
                );
                return;
            }
        };
        service.set_entrypoint(self.sourced(entrypoint, promoted));
        self.mapped(path, promoted);
    }

    fn map_user(
        &mut self,
        subject: &str,
        observed: &Observed<docker_lens::evidence::ProtectedValue>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.user");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let Some(text) = self.protected_text(&path, value) else {
            return;
        };
        if text.is_empty() {
            self.unsupported(
                path,
                "empty effective Docker user does not establish a requested user",
                Some(observed.origin),
            );
            return;
        }
        service.set_user(self.sourced(ProtectedString::plain(text), promoted));
        self.mapped(path, promoted);
    }

    fn map_working_directory(
        &mut self,
        subject: &str,
        observed: &Observed<docker_lens::evidence::ProtectedValue>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.working_directory");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let Some(text) = self.protected_text(&path, value) else {
            return;
        };
        if text.is_empty() {
            self.unsupported(
                path,
                "empty effective Docker working directory does not establish intent",
                Some(observed.origin),
            );
            return;
        }
        service.set_working_directory(self.sourced(ProtectedString::plain(text), promoted));
        self.mapped(path, promoted);
    }

    fn map_stop_signal(
        &mut self,
        subject: &str,
        observed: &Observed<docker_lens::evidence::ProtectedValue>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.stop_signal");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let Some(text) = self.protected_text(&path, value) else {
            return;
        };
        if text.is_empty() {
            self.unsupported(
                path,
                "empty Docker stop signal does not establish a requested signal",
                Some(observed.origin),
            );
            return;
        }
        service.set_stop_signal(self.sourced(ProtectedString::plain(text), promoted));
        self.mapped(path, promoted);
    }

    fn map_stop_timeout(&mut self, subject: &str, observed: &Observed<i64>, service: &mut Service) {
        let path = format!("{subject}.stop_timeout");
        let Some((value, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        // Docker's reviewed target represents finite whole seconds as u32. In particular,
        // the native negative/infinite sentinel must never become a default or zero.
        let Ok(seconds) = u32::try_from(*value) else {
            self.unsupported(
                path,
                "Docker stop timeout is outside the reviewed finite whole-second range",
                Some(observed.origin),
            );
            return;
        };
        let Ok(timeout) = StopTimeout::new(format!("{seconds}s")) else {
            self.invalid(path, "Docker stop timeout cannot be represented as a neutral duration");
            return;
        };
        service.set_stop_timeout(self.sourced(timeout, promoted));
        self.mapped(path, promoted);
    }

    fn map_restart_policy(&mut self, subject: &str, observed: &Observed<NativeRestartPolicy>, service: &mut Service) {
        let path = format!("{subject}.restart_policy");
        let enabled = self.policy.promotes_portable_effective_settings();
        let flag = "DockerPromotionPolicy::with_portable_effective_settings";
        let Some((native, mut promoted)) = self.promotable(&path, observed, enabled, flag) else {
            return;
        };
        let Some((name, name_promoted)) = self.promotable(&format!("{path}.name"), &native.name, enabled, flag) else {
            self.unsupported(path, "Docker restart policy has no usable name", Some(observed.origin));
            return;
        };
        promoted |= name_promoted;
        let policy = match name.as_bytes() {
            // Reviewed Engine API schemas define a present empty Name as no restart.
            b"" | b"no" => NeutralRestartPolicy::Never,
            b"always" => NeutralRestartPolicy::Always,
            b"unless-stopped" => NeutralRestartPolicy::UnlessStopped,
            b"on-failure" => {
                let count_path = format!("{path}.maximum_retry_count");
                let Some((count, count_promoted)) =
                    self.promotable(&count_path, &native.maximum_retry_count, enabled, flag)
                else {
                    self.unsupported(
                        path,
                        "Docker on-failure retry limit is incomplete",
                        Some(observed.origin),
                    );
                    return;
                };
                promoted |= count_promoted;
                NeutralRestartPolicy::on_failure(std::num::NonZeroU64::new(u64::from(*count)))
            }
            _ => {
                self.unsupported(
                    path,
                    "Docker restart policy spelling is not reviewed",
                    Some(observed.origin),
                );
                return;
            }
        };
        if !matches!(policy, NeutralRestartPolicy::OnFailure { .. })
            && native.maximum_retry_count.value().is_some_and(|count| *count != 0)
        {
            self.unsupported(
                format!("{path}.maximum_retry_count"),
                "unexpected Docker retry count is not represented by this restart policy",
                Some(native.maximum_retry_count.origin),
            );
            return;
        }
        service.set_restart_policy(self.sourced(policy, promoted));
        self.mapped(path, promoted);
    }

    fn map_healthcheck(&mut self, subject: &str, observed: &Observed<NativeHealthcheck>, service: &mut Service) {
        let path = format!("{subject}.healthcheck");
        let enabled = self.policy.promotes_portable_effective_settings();
        let flag = "DockerPromotionPolicy::with_portable_effective_settings";
        let Some((native, promoted)) = self.promotable(&path, observed, enabled, flag) else {
            return;
        };
        let Some((test, test_promoted)) = self.promotable(&format!("{path}.test"), &native.test, enabled, flag) else {
            self.unsupported(
                path,
                "Docker healthcheck has no complete typed test",
                Some(observed.origin),
            );
            return;
        };
        let needs_default_review = !matches!(test, HealthcheckTest::None)
            && [
                native.interval_ns.availability,
                native.timeout_ns.availability,
                native.retries.availability,
            ]
            .into_iter()
            .any(|availability| matches!(availability, Availability::Missing | Availability::Null));
        // Engine exposes zero for optional startup timings. A zero grace period
        // is no grace; zero startup interval remains a native-default sentinel,
        // not proof of one portable effective interval. Preserve the spelling
        // while keeping that target-default uncertainty explicit.
        let startup_interval_default = native.start_interval_ns.availability == Availability::Present
            && native.start_interval_ns.value().is_some_and(|nanos| *nanos == 0);
        let mut healthcheck = NeutralHealthcheck::new();
        let shell = matches!(test, HealthcheckTest::CmdShell(_));
        match test {
            HealthcheckTest::Cmd(parts) if !parts.is_empty() => {
                let Some(parts) = self.protected_arguments(&format!("{path}.test"), parts) else {
                    return;
                };
                healthcheck.set_command(self.sourced(HealthcheckCommand::Exec(parts), promoted || test_promoted));
            }
            HealthcheckTest::CmdShell(command) => {
                let Some(shell_script) = self.protected_text(&format!("{path}.test"), command) else {
                    return;
                };
                healthcheck.set_command(self.sourced(
                    HealthcheckCommand::Shell(ProtectedString::plain(shell_script)),
                    promoted || test_promoted,
                ));
            }
            HealthcheckTest::None => {
                healthcheck.set_disabled(self.sourced(true, promoted || test_promoted));
            }
            HealthcheckTest::Cmd(_) | HealthcheckTest::Other(_) => {
                self.unsupported(
                    path,
                    "Docker healthcheck test is not reviewed for neutral execution",
                    Some(observed.origin),
                );
                return;
            }
        }
        let Ok(duration_promoted) = self.map_health_durations(&path, native, enabled, promoted, &mut healthcheck)
        else {
            return;
        };
        let mut combined_promoted = promoted || test_promoted || duration_promoted;
        if !matches!(native.retries.availability, Availability::Missing | Availability::Null) {
            let retry_path = format!("{path}.retries");
            let Some((retries, child_promoted)) = self.promotable(&retry_path, &native.retries, enabled, flag) else {
                return;
            };
            if *retries == 0 {
                self.unsupported(
                    retry_path,
                    "zero Docker health retries defer to native defaults",
                    Some(native.retries.origin),
                );
                return;
            }
            let Ok(retries) = HealthcheckRetries::new(retries.to_string()) else {
                self.invalid(retry_path, "Docker health retries are invalid for neutral intent");
                return;
            };
            healthcheck.set_retries(self.sourced(retries, promoted || child_promoted));
            combined_promoted |= child_promoted;
        }
        service.set_healthcheck(self.sourced(healthcheck, combined_promoted));
        if needs_default_review {
            self.unsupported(
                path.clone(),
                "Docker healthcheck interval, timeout, or retries rely on unproved target defaults",
                Some(observed.origin),
            );
        }
        if (shell && !needs_default_review) || startup_interval_default {
            let reason = match (shell, startup_interval_default) {
                (true, true) => {
                    "Docker shell healthcheck and zero startup interval require target shell-behavior and native-default review"
                }
                (true, false) => "Docker shell healthcheck requires target shell-behavior review",
                (false, true) => "zero Docker health startup interval defers to native target defaults",
                (false, false) => unreachable!("review required only for shell or startup defaults"),
            };
            self.approximate(path, reason);
        } else if !needs_default_review {
            self.mapped(path, combined_promoted);
        }
    }

    fn map_health_durations(
        &mut self,
        path: &str,
        native: &NativeHealthcheck,
        enabled: bool,
        promoted: bool,
        healthcheck: &mut NeutralHealthcheck,
    ) -> Result<bool, ()> {
        let mut combined_promoted = false;
        let fields = [
            (&native.interval_ns, "interval"),
            (&native.timeout_ns, "timeout"),
            (&native.start_period_ns, "start_period"),
            (&native.start_interval_ns, "start_interval"),
        ];
        for (field, name) in fields {
            let field_path = format!("{path}.{name}");
            let optional_startup_timing = matches!(name, "start_period" | "start_interval")
                && native
                    .test
                    .value()
                    .is_some_and(|test| !matches!(test, HealthcheckTest::None));
            let Ok(duration) = self.optional_health_duration(&field_path, field, enabled, optional_startup_timing)
            else {
                return Err(());
            };
            if let Some((duration, child_promoted)) = duration {
                combined_promoted |= child_promoted;
                let sourced = self.sourced(duration, promoted || child_promoted);
                match name {
                    "interval" => healthcheck.set_interval(sourced),
                    "timeout" => healthcheck.set_timeout(sourced),
                    "start_period" => healthcheck.set_start_period(sourced),
                    "start_interval" => healthcheck.set_start_interval(sourced),
                    _ => unreachable!("closed health duration field"),
                }
            }
        }
        Ok(combined_promoted)
    }

    fn optional_health_duration(
        &mut self,
        subject: &str,
        observed: &Observed<u64>,
        enabled: bool,
        allow_zero: bool,
    ) -> Result<Option<(HealthcheckDuration, bool)>, ()> {
        if matches!(observed.availability, Availability::Missing | Availability::Null) {
            return Ok(None);
        }
        let Some((nanos, promoted)) = self.promotable(
            subject,
            observed,
            enabled,
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return Err(());
        };
        if *nanos == 0 && !allow_zero {
            self.unsupported(
                subject.to_owned(),
                "zero Docker health duration defers to native defaults",
                Some(observed.origin),
            );
            return Err(());
        }
        let value = HealthcheckDuration::new(format!("{nanos}ns")).map_err(|_| {
            self.invalid(subject, "Docker health duration is invalid for neutral intent");
        })?;
        Ok(Some((value, promoted)))
    }

    fn map_exposed_ports(&mut self, subject: &str, observed: &Observed<Vec<PortKey>>, service: &mut Service) {
        let path = format!("{subject}.exposed_ports");
        let Some((ports, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let mut failed = false;
        for (index, port) in ports.iter().enumerate() {
            let field = format!("{path}[{index}]");
            let Ok(value) = ExposedPort::new(port.container_port, protocol(port.protocol)) else {
                self.invalid(field, "Docker exposed container port is invalid");
                failed = true;
                continue;
            };
            service.add_exposed_port(self.sourced(value, promoted));
            self.mapped(field, promoted);
        }
        if failed {
            if !service.exposed_ports().is_none_or(<[_]>::is_empty) {
                self.approximate(path, "only part of the Docker exposed-port set was retained");
            }
        } else {
            if ports.is_empty() {
                service.set_exposed_ports_with_origins(Vec::new(), self.sourced((), promoted).origins().to_vec());
            }
            self.mapped(path, promoted);
        }
    }

    fn map_configured_ports(
        &mut self,
        subject: &str,
        observed: &Observed<Vec<PortObservation>>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.configured_ports");
        let enabled = self.policy.promotes_portable_effective_settings();
        let flag = "DockerPromotionPolicy::with_portable_effective_settings";
        let Some((ports, promoted)) = self.promotable(&path, observed, enabled, flag) else {
            return;
        };
        let mut retained = 0_usize;
        let mut failed = false;
        let mut combined_promoted = promoted;
        for (port_index, port) in ports.iter().enumerate() {
            let port_path = format!("{path}[{port_index}]");
            let (row_retained, row_failed, row_promoted) =
                self.map_configured_port(&port_path, port, enabled, promoted, service);
            retained += row_retained;
            failed |= row_failed;
            combined_promoted |= row_promoted;
        }
        if failed {
            if retained > 0 {
                self.approximate(path, "only part of the Docker HostConfig publications was retained");
            } else {
                self.unsupported(
                    path,
                    "Docker HostConfig publications need manual review",
                    Some(observed.origin),
                );
            }
        } else {
            self.mapped(path, combined_promoted);
        }
    }

    fn map_configured_port(
        &mut self,
        port_path: &str,
        port: &PortObservation,
        enabled: bool,
        promoted: bool,
        service: &mut Service,
    ) -> (usize, bool, bool) {
        let flag = "DockerPromotionPolicy::with_portable_effective_settings";
        if port.key.container_port == 0 {
            self.invalid(port_path, "Docker configured container port is zero");
            return (0, true, false);
        }
        if matches!(port.bindings.availability, Availability::Missing | Availability::Null) {
            self.unsupported(
                port_path.to_owned(),
                "Docker port key has no host publication",
                Some(port.bindings.origin),
            );
            return (0, true, false);
        }
        let Some((bindings, bindings_promoted)) = self.promotable(port_path, &port.bindings, enabled, flag) else {
            return (0, true, false);
        };
        let mut retained = 0;
        let mut failed = false;
        let mut combined_promoted = false;
        for (binding_index, binding) in bindings.iter().enumerate() {
            let binding_path = format!("{port_path}.bindings[{binding_index}]");
            let Some((host_port, publication_promoted)) =
                self.promotable(&format!("{binding_path}.host_port"), &binding.host_port, enabled, flag)
            else {
                self.unsupported(
                    binding_path,
                    "Docker host port is not fully specified",
                    Some(binding.host_port.origin),
                );
                failed = true;
                continue;
            };
            let mut reconstructed = promoted || bindings_promoted || publication_promoted;
            let host_address = if matches!(binding.host_ip.availability, Availability::Missing | Availability::Null) {
                None
            } else {
                let Some((value, host_ip_promoted)) =
                    self.promotable(&format!("{binding_path}.host_ip"), &binding.host_ip, enabled, flag)
                else {
                    failed = true;
                    continue;
                };
                let Some(text) = self.protected_text(&binding_path, value) else {
                    failed = true;
                    continue;
                };
                reconstructed |= host_ip_promoted;
                if text.is_empty() {
                    None
                } else if text.parse::<std::net::IpAddr>().is_ok() {
                    Some(text)
                } else {
                    self.unsupported(
                        binding_path,
                        "Docker host address is not a portable IP address",
                        Some(binding.host_ip.origin),
                    );
                    failed = true;
                    continue;
                }
            };
            let Ok(value) = Port::new(
                port.key.container_port,
                Some(*host_port),
                host_address,
                protocol(port.key.protocol),
            ) else {
                self.invalid(binding_path, "Docker host publication is invalid for the neutral model");
                failed = true;
                continue;
            };
            service.add_port(self.sourced(value, reconstructed));
            self.mapped(binding_path, reconstructed);
            combined_promoted |= reconstructed;
            retained += 1;
        }
        (retained, failed, combined_promoted)
    }

    fn map_environment(
        &mut self,
        subject: &str,
        observed: &Observed<Vec<EnvironmentAssignment>>,
        service: &mut Service,
    ) {
        let path = format!("{subject}.environment");
        let Some((entries, promoted)) = self.promotable(
            &path,
            observed,
            self.policy.promotes_portable_effective_settings(),
            "DockerPromotionPolicy::with_portable_effective_settings",
        ) else {
            return;
        };
        let mut names = std::collections::HashSet::new();
        let mut incomplete = false;
        let mut literal_count = 0_usize;
        for (index, entry) in entries.iter().enumerate() {
            let field = format!("{path}[{index}]");
            let Some(name) = self.protected_text(&field, &entry.name) else {
                incomplete = true;
                continue;
            };
            let Ok(name) = Identifier::new(name) else {
                self.unsupported(
                    field,
                    "Docker environment name is not representable in the neutral model",
                    Some(observed.origin),
                );
                incomplete = true;
                continue;
            };
            if !names.insert(name.as_str().to_owned()) {
                self.unsupported(
                    field,
                    "duplicate Docker environment name needs explicit resolution",
                    Some(observed.origin),
                );
                incomplete = true;
                continue;
            }
            let value = if self.policy.promotes_protected_environment_values() {
                if let Some(value) = entry.value.as_ref() {
                    let Some(text) = self.protected_text(&field, value) else {
                        incomplete = true;
                        continue;
                    };
                    EnvironmentValue::Literal(ProtectedString::sensitive(text))
                } else {
                    self.unsupported(
                        field.clone(),
                        "Docker environment has no value; supply it on the target",
                        Some(observed.origin),
                    );
                    incomplete = true;
                    EnvironmentValue::Required
                }
            } else {
                self.unsupported(
                    field.clone(),
                    "Docker environment value withheld; supply the named value on the target",
                    Some(observed.origin),
                );
                incomplete = true;
                EnvironmentValue::Required
            };
            let literal = matches!(&value, EnvironmentValue::Literal(_));
            service.add_environment(self.sourced(EnvironmentVariable::new(name, value), promoted));
            if literal {
                self.mapped(field, promoted);
                literal_count += 1;
            }
        }
        if entries.is_empty() {
            self.mapped(path, promoted);
        } else if incomplete {
            if literal_count > 0 {
                self.approximate(
                    path,
                    "Docker environment snapshot contains named prerequisites or omitted entries",
                );
            } else {
                self.unsupported(
                    path,
                    "Docker environment values remain named prerequisites",
                    Some(observed.origin),
                );
            }
        } else {
            self.mapped(path, promoted);
        }
    }

    fn map_network_membership(
        &mut self,
        subject: &str,
        container: &ContainerObservation,
        networks: &[NetworkObservation],
        service: &mut Service,
    ) {
        let path = format!("{subject}.networks");
        let observed = &container.networks;
        if matches!(observed.availability, Availability::Missing | Availability::Null) {
            return;
        }
        // Redaction takes precedence over any attachment values retained as source evidence.
        let attachments = match observed.availability {
            Availability::Redacted => None,
            _ => observed.value(),
        };
        let Some(attachments) = attachments else {
            self.unsupported(
                path,
                "Docker network membership is unavailable or redacted",
                Some(observed.origin),
            );
            return;
        };
        for (index, attachment) in attachments.iter().enumerate() {
            self.report_network_attachment_fields(&format!("{path}[{index}]"), attachment);
        }
        if !self.policy.promotes_observed_network_membership() {
            for (index, attachment) in attachments.iter().enumerate() {
                self.unpromoted_attachment_aliases(
                    &format!("{path}[{index}]"),
                    attachment,
                    "Docker aliases require separately enabled, ID-correlated network membership",
                );
            }
            self.unsupported(
                path,
                "runtime Docker network membership needs separate source-state promotion",
                Some(observed.origin),
            );
            return;
        }
        if observed.origin != Origin::RuntimeAssigned
            || matches!(
                container.network_mode.availability,
                Availability::Redacted | Availability::Empty
            )
            || container.network_mode.value().is_some_and(|mode| {
                matches!(
                    mode.kind,
                    NetworkModeKind::Host | NetworkModeKind::None | NetworkModeKind::Container
                ) || !valid_native_network_name(mode.native_value.as_bytes())
            })
        {
            for (index, attachment) in attachments.iter().enumerate() {
                self.unpromoted_attachment_aliases(
                    &format!("{path}[{index}]"),
                    attachment,
                    "Docker aliases cannot be reconstructed with incompatible network mode or origin",
                );
            }
            self.unsupported(
                path,
                "Docker network mode or membership origin does not support reviewed topology reconstruction",
                Some(observed.origin),
            );
            return;
        }
        let mut retained = 0_usize;
        for (index, attachment) in attachments.iter().enumerate() {
            let field = format!("{path}[{index}]");
            if let Some(network_index) = self.match_inspected_network(&field, attachment, attachments, networks) {
                let Ok(name) = Identifier::new(format!("docker-network-{}", network_index + 1)) else {
                    self.invalid(field, "generated Docker network reference is invalid");
                    continue;
                };
                let aliases = self.map_attachment_aliases(&field, attachment, container);
                service.add_network(self.sourced(NeutralNetworkAttachment::new(name, aliases), true));
                self.approximate_observed(
                    field,
                    "Docker runtime network membership is reconstructed as uncertain current-state topology",
                    Origin::RuntimeAssigned,
                );
                retained += 1;
            } else {
                self.unpromoted_attachment_aliases(
                    &field,
                    attachment,
                    "Docker aliases require one uniquely ID-correlated inspected network",
                );
            }
        }
        if retained > 0 {
            self.approximate_observed(
                path,
                "Docker runtime network membership does not prove authored topology or resource ownership",
                Origin::RuntimeAssigned,
            );
        } else {
            self.unsupported(
                path,
                "Docker network membership has no uniquely ID-correlated inspected network",
                Some(observed.origin),
            );
        }
    }

    fn match_inspected_network(
        &mut self,
        field: &str,
        attachment: &NativeNetworkAttachment,
        attachments: &[NativeNetworkAttachment],
        networks: &[NetworkObservation],
    ) -> Option<usize> {
        let Some(id) = runtime_network_id(&attachment.network_id) else {
            self.unsupported(
                format!("{field}.network_id"),
                "Docker attachment has no usable runtime network ID",
                Some(attachment.network_id.origin),
            );
            return None;
        };
        if attachments
            .iter()
            .filter(|candidate| runtime_network_id(&candidate.network_id) == Some(id))
            .count()
            != 1
        {
            self.unsupported(
                field.to_owned(),
                "duplicate Docker attachment network ID is ambiguous",
                None,
            );
            return None;
        }
        let matches: Vec<_> = networks
            .iter()
            .enumerate()
            .filter(|(_, network)| runtime_network_id(&network.id) == Some(id))
            .collect();
        if matches.len() != 1 {
            self.unsupported(
                field.to_owned(),
                "Docker attachment needs exactly one inspected network with matching typed ID",
                Some(attachment.network_id.origin),
            );
            return None;
        }
        let (index, network) = matches[0];
        let name = usable_inspected_name(&network.name);
        if !valid_native_network_name(attachment.name.as_bytes())
            || name.is_some_and(|name| !valid_native_network_name(name.as_bytes()))
            || name.is_none_or(|name| name.as_bytes() != attachment.name.as_bytes())
            || networks.iter().any(|candidate| {
                usable_inspected_name(&candidate.name).is_some_and(|name| name.as_bytes() == attachment.name.as_bytes())
                    && runtime_network_id(&candidate.id) != Some(id)
            })
        {
            self.unsupported(
                field.to_owned(),
                "Docker attachment name conflicts with inspected network identity",
                Some(network.name.origin),
            );
            return None;
        }
        Some(index)
    }

    fn report_network_attachment_fields(&mut self, field: &str, attachment: &NativeNetworkAttachment) {
        for (name, observed) in [
            ("network_id", &attachment.network_id),
            ("ip_address", &attachment.ip_address),
            ("ipv6_address", &attachment.ipv6_address),
        ] {
            if !matches!(observed.availability, Availability::Missing | Availability::Null) {
                self.observation_only(
                    format!("{field}.{name}"),
                    "Docker runtime network identity or address is observation-only, not static intent",
                    observed.origin,
                );
            }
        }
        for (name, observed) in [
            ("requested_ipv4_address", &attachment.requested_ipv4_address),
            ("requested_ipv6_address", &attachment.requested_ipv6_address),
        ] {
            if !matches!(observed.availability, Availability::Missing | Availability::Null) {
                self.unsupported(
                    format!("{field}.{name}"),
                    "Docker requested static network address is not promoted by this source policy",
                    Some(observed.origin),
                );
            }
        }
    }

    fn unpromoted_attachment_aliases(
        &mut self,
        field: &str,
        attachment: &NativeNetworkAttachment,
        summary: &'static str,
    ) {
        if !matches!(
            attachment.aliases.availability,
            Availability::Missing | Availability::Null
        ) {
            self.unsupported(format!("{field}.aliases"), summary, Some(attachment.aliases.origin));
        }
    }

    fn map_attachment_aliases(
        &mut self,
        field: &str,
        attachment: &NativeNetworkAttachment,
        container: &ContainerObservation,
    ) -> Vec<Sourced<ProtectedString>> {
        let path = format!("{field}.aliases");
        if matches!(
            attachment.aliases.availability,
            Availability::Missing | Availability::Null
        ) {
            return Vec::new();
        }
        if !self.policy.promotes_observed_network_aliases() {
            self.unsupported(
                path,
                "Docker aliases require their separate source-state promotion switch",
                Some(attachment.aliases.origin),
            );
            return Vec::new();
        }
        if !matches!(
            attachment.aliases.availability,
            Availability::Empty | Availability::Present
        ) || attachment.aliases.origin != Origin::Effective
        {
            self.unsupported(
                path,
                "Docker aliases are unavailable, redacted, or not effective observations",
                Some(attachment.aliases.origin),
            );
            return Vec::new();
        }
        let Some(values) = attachment.aliases.value() else {
            self.unsupported(
                path,
                "Docker aliases have no usable observed values",
                Some(attachment.aliases.origin),
            );
            return Vec::new();
        };
        let mut aliases = Vec::new();
        for (index, alias) in values.iter().enumerate() {
            let alias_path = format!("{path}[{index}]");
            let bytes = alias.as_bytes();
            if !valid_portable_network_alias(bytes) {
                self.unsupported(
                    alias_path,
                    "Docker network alias has no reviewed portable spelling",
                    Some(attachment.aliases.origin),
                );
                continue;
            }
            if values[..index].iter().any(|previous| previous.as_bytes() == bytes) {
                self.unsupported(
                    alias_path,
                    "duplicate Docker network alias is ambiguous",
                    Some(attachment.aliases.origin),
                );
                continue;
            }
            if runtime_identity_alias(bytes, container) {
                self.unsupported(
                    alias_path,
                    "Docker runtime identity-like alias is not portable authored intent",
                    Some(attachment.aliases.origin),
                );
                continue;
            }
            if let Some(value) = self.protected_text(&alias_path, alias) {
                aliases.push(self.sourced(ProtectedString::plain(value), true));
                self.approximate_observed(
                    alias_path,
                    "Docker effective alias may be generated and does not prove authored DNS intent",
                    attachment.aliases.origin,
                );
            }
        }
        if !aliases.is_empty() {
            self.approximate_observed(
                path,
                "Docker effective alias set is reconstructed as uncertain current-state topology",
                attachment.aliases.origin,
            );
        }
        aliases
    }

    fn map_mounts(
        &mut self,
        subject: &str,
        observed: &Observed<Vec<MountObservation>>,
        volumes: &[VolumeObservation],
        service: &mut Service,
    ) {
        let path = format!("{subject}.mounts");
        let named = self.policy.promotes_named_volume_mounts();
        let binds = self.policy.promotes_same_host_bind_mounts();
        let Some((mounts, promoted)) = self.promotable(
            &path,
            observed,
            named || binds,
            "DockerPromotionPolicy::with_named_volume_mounts or with_same_host_bind_mounts",
        ) else {
            return;
        };
        let mut retained = 0_usize;
        let mut failed = false;
        let mut combined_promoted = promoted;
        for (index, mount) in mounts.iter().enumerate() {
            let field = format!("{path}[{index}]");
            let enabled = match mount.kind {
                MountKind::Volume => named,
                MountKind::Bind => binds,
                MountKind::Tmpfs | MountKind::Other => {
                    if Self::tmpfs_has_relabel(mount) {
                        self.invalid(
                            format!("{field}.mode"),
                            "Docker relabel intent requires a host-path bind; author a bind or remove relabelling",
                        );
                        failed = true;
                        continue;
                    }
                    self.unsupported(
                        field,
                        "Docker mount kind is not reviewed for neutral promotion",
                        Some(observed.origin),
                    );
                    failed = true;
                    continue;
                }
            };
            if !enabled {
                self.unsupported(
                    field,
                    "Docker mount kind needs its separate source-state permission",
                    Some(observed.origin),
                );
                failed = true;
                continue;
            }
            let flag = if mount.kind == MountKind::Volume {
                "DockerPromotionPolicy::with_named_volume_mounts"
            } else {
                "DockerPromotionPolicy::with_same_host_bind_mounts"
            };
            let Some((destination, destination_promoted)) = self.mount_destination(&field, mount, enabled, flag) else {
                failed = true;
                continue;
            };
            let access_path = format!("{field}.read_write");
            let Some((read_write, access_promoted)) = self.promotable(&access_path, &mount.read_write, enabled, flag)
            else {
                failed = true;
                continue;
            };
            let Some((source, source_promoted)) = self.mount_source(&field, mount, volumes, enabled, flag) else {
                failed = true;
                continue;
            };
            let Some((relabel, options_promoted)) = self.mount_options(&field, mount, *read_write, enabled, flag)
            else {
                failed = true;
                continue;
            };
            let Ok(value) = Mount::new(source, destination, !*read_write) else {
                self.invalid(field, "Docker mount cannot be represented by neutral storage intent");
                failed = true;
                continue;
            };
            // Acquired paths are literal native bytes, never service-execution
            // expressions or target-context dollar/percent syntax.
            let mut value = value.with_literal_paths();
            if let Some(relabel) = relabel {
                value.set_selinux_relabel(relabel);
            }
            let mount_promoted =
                promoted || destination_promoted || access_promoted || source_promoted || options_promoted;
            service.add_mount(self.sourced(value, mount_promoted));
            combined_promoted |= mount_promoted;
            retained += 1;
            self.mapped(field, mount_promoted);
        }
        if failed {
            if retained > 0 {
                self.approximate(path, "only part of the Docker mount set was retained");
            } else {
                self.unsupported(path, "Docker mounts need field-specific review", Some(observed.origin));
            }
        } else {
            self.mapped(path, combined_promoted);
        }
    }

    // A typed sibling never bypasses either raw or interpreted mode redaction.
    fn tmpfs_has_relabel(mount: &MountObservation) -> bool {
        mount.kind == MountKind::Tmpfs
            && mount.mode.availability != Availability::Redacted
            && mount.mode_interpretation.availability != Availability::Redacted
            && matches!(
                mount.mode_interpretation.value(),
                Some(MountModeInterpretation::Supported { relabel: Some(_), .. })
            )
    }

    // Extracted without semantic changes: retain the same availability/promotion checks,
    // protected UTF-8 validation, diagnostic field, destination origin and failure boundary.
    fn mount_destination(
        &mut self,
        field: &str,
        mount: &MountObservation,
        enabled: bool,
        flag: &'static str,
    ) -> Option<(String, bool)> {
        let path = format!("{field}.destination");
        let (destination, promoted) = self.promotable(&path, &mount.destination, enabled, flag)?;
        let destination = self.protected_text(&path, destination)?;
        if destination.is_empty() {
            self.unsupported(
                path,
                "Docker mount destination is empty",
                Some(mount.destination.origin),
            );
            return None;
        }
        Some((destination, promoted))
    }

    fn mount_source(
        &mut self,
        field: &str,
        mount: &MountObservation,
        volumes: &[VolumeObservation],
        enabled: bool,
        flag: &'static str,
    ) -> Option<(MountSource, bool)> {
        match mount.kind {
            MountKind::Volume => {
                let name_path = format!("{field}.name");
                let (name, promoted) = self.promotable(&name_path, &mount.name, enabled, flag)?;
                let matches: Vec<_> = volumes
                    .iter()
                    .enumerate()
                    .filter(|(_, volume)| {
                        usable_inspected_name(&volume.name)
                            .is_some_and(|candidate| candidate.as_bytes() == name.as_bytes())
                    })
                    .collect();
                if matches.len() != 1 {
                    self.unsupported(
                        name_path,
                        "Docker named mount needs exactly one selected inspected volume with the same typed name",
                        Some(mount.name.origin),
                    );
                    return None;
                }
                let Ok(name) = Identifier::new(format!("docker-volume-{}", matches[0].0 + 1)) else {
                    self.invalid(field, "generated Docker volume reference is invalid");
                    return None;
                };
                if !matches!(mount.source.availability, Availability::Missing | Availability::Null) {
                    self.observation_only(
                        format!("{field}.source"),
                        "Docker named-volume backing path is daemon-local observation, not mount intent",
                        mount.source.origin,
                    );
                }
                Some((MountSource::Volume(name), promoted))
            }
            MountKind::Bind => {
                let source_path = format!("{field}.source");
                let (source, promoted) = self.promotable(&source_path, &mount.source, enabled, flag)?;
                let source = self.protected_text(&source_path, source)?;
                if !std::path::Path::new(&source).is_absolute() {
                    self.unsupported(
                        source_path,
                        "Docker bind source needs an absolute same-host path",
                        Some(mount.source.origin),
                    );
                    return None;
                }
                Some((MountSource::HostPath(source), promoted))
            }
            MountKind::Tmpfs | MountKind::Other => unreachable!("closed mount kind checked above"),
        }
    }

    /// Never parse raw Mode bytes here: `DockerLens` retains them as protected native evidence.
    /// Supported Mode evidence proves access; native volume labels remain a reported limitation.
    fn mount_options(
        &mut self,
        field: &str,
        mount: &MountObservation,
        read_write: bool,
        enabled: bool,
        flag: &'static str,
    ) -> Option<(Option<SelinuxRelabel>, bool)> {
        let mode_path = format!("{field}.mode");
        if mount.mode.availability != mount.mode_interpretation.availability
            || mount.mode.origin != mount.mode_interpretation.origin
        {
            self.invalid(
                mode_path,
                "Docker raw and typed mount mode evidence disagree; reacquire coherent evidence",
            );
            return None;
        }
        if !matches!(mount.mode.availability, Availability::Present | Availability::Empty) {
            self.unsupported(
                mode_path,
                "Docker mount mode is absent, unavailable, or redacted; reacquire explicit supported mode evidence before promoting this mount",
                Some(mount.mode.origin),
            );
            return None;
        }
        let (mode, mode_promoted) = self.promotable(&mode_path, &mount.mode_interpretation, enabled, flag)?;
        // A valued redacted raw field must never be bypassed through its typed sibling.
        let Some(raw_mode) = mount.mode.value() else {
            self.unsupported(
                mode_path,
                "Docker raw mount mode is unavailable; reacquire complete evidence",
                Some(mount.mode.origin),
            );
            return None;
        };
        if raw_mode.is_empty() != (mount.mode.availability == Availability::Empty) {
            self.invalid(
                mode_path,
                "Docker mount mode availability contradicts its value; reacquire coherent evidence",
            );
            return None;
        }
        let MountModeInterpretation::Supported {
            access,
            relabel: native_relabel,
        } = mode
        else {
            self.unsupported(
                mode_path,
                "Docker mount mode contains unreviewed options; author a supported mount explicitly",
                Some(mount.mode.origin),
            );
            return None;
        };
        if mount.mode.availability == Availability::Empty && (access.is_some() || native_relabel.is_some()) {
            self.invalid(
                mode_path,
                "Docker empty mount mode carries nonempty typed intent; reacquire coherent evidence",
            );
            return None;
        }
        if access.is_some_and(|access| matches!(access, MountAccess::ReadWrite) != read_write) {
            self.invalid(
                mode_path,
                "Docker mount mode contradicts observed access; reacquire or author a consistent mount",
            );
            return None;
        }
        let mut relabel = None;
        if let Some(native_relabel) = native_relabel {
            if !matches!(mount.kind, MountKind::Bind | MountKind::Volume) {
                self.invalid(
                    mode_path,
                    "Docker relabel intent requires a host-path bind; author a bind or remove relabelling",
                );
                return None;
            }
            if mount.kind == MountKind::Bind {
                relabel = Some(match native_relabel {
                    BindRelabel::Shared => SelinuxRelabel::Shared,
                    BindRelabel::Private => SelinuxRelabel::Private,
                });
            }
        }
        let propagation_promoted = self.mount_propagation(field, mount, enabled, flag)?;
        let promoted = mode_promoted || propagation_promoted;
        if mount.kind == MountKind::Volume && native_relabel.is_some() {
            self.unsupported(
                mode_path,
                "Docker named-volume labeling evidence cannot be represented in neutral mount intent; review target volume labeling explicitly before approving partial conversion",
                Some(mount.mode.origin),
            );
        } else {
            self.mapped(mode_path, mode_promoted);
        }
        if relabel.is_some() {
            self.mapped(format!("{field}.selinux_relabel"), mode_promoted);
        }
        Some((relabel, promoted))
    }

    // Propagation retains its own fallible availability/value/default checks and outcomes.
    // Call it before mapping mode/relabel outcomes even when mode promotion is already true.
    fn mount_propagation(
        &mut self,
        field: &str,
        mount: &MountObservation,
        enabled: bool,
        flag: &'static str,
    ) -> Option<bool> {
        let mut promoted = false;
        // Only the inspected recursive-private default (or empty/absent spelling)
        // is compatible with the neutral model's lack of propagation intent.
        let propagation = &mount.propagation;
        if matches!(propagation.availability, Availability::Missing | Availability::Null)
            && propagation.value().is_some()
        {
            self.invalid(
                format!("{field}.propagation"),
                "Docker absent propagation carries a value; reacquire coherent evidence",
            );
            return None;
        }
        if !matches!(propagation.availability, Availability::Missing | Availability::Null) {
            let path = format!("{field}.propagation");
            let (value, propagation_promoted) = self.promotable(&path, propagation, enabled, flag)?;
            if value.is_empty() != (propagation.availability == Availability::Empty) {
                self.invalid(
                    path,
                    "Docker mount propagation availability contradicts its value; reacquire coherent evidence",
                );
                return None;
            }
            if !(value.is_empty() || mount.kind == MountKind::Bind && value.as_bytes() == b"rprivate") {
                self.unsupported(
                    path,
                    "Docker mount propagation is outside the reviewed default; author a supporting target explicitly",
                    Some(propagation.origin),
                );
                return None;
            }
            promoted |= propagation_promoted;
            self.mapped(path, propagation_promoted);
        }
        Some(promoted)
    }

    fn protected_text(&mut self, subject: &str, value: &docker_lens::evidence::ProtectedValue) -> Option<String> {
        match std::str::from_utf8(value.as_bytes()) {
            Ok(value) if !value.contains('\0') => Some(value.to_owned()),
            _ => {
                self.invalid(subject, "Docker text is not representable as neutral UTF-8 intent");
                None
            }
        }
    }

    fn protected_arguments(
        &mut self,
        subject: &str,
        values: &[docker_lens::evidence::ProtectedValue],
    ) -> Option<Vec<ProtectedString>> {
        values
            .iter()
            .map(|value| self.protected_text(subject, value).map(ProtectedString::plain))
            .collect()
    }

    fn unmapped_container_fields(&mut self, subject: &str, container: &ContainerObservation) {
        macro_rules! field {
            ($name:literal, $value:expr) => {
                self.unmapped(format!("{subject}.{}", $name), &$value);
            };
        }
        field!("image_id", container.image_id);
        field!("hostname", container.hostname);
        field!("runtime_ports", container.runtime_ports);
        field!("network_mode", container.network_mode);
        let runtime = &container.runtime;
        field!("read_only_rootfs", runtime.read_only_rootfs);
        field!("memory_bytes", runtime.memory_bytes);
        field!("pids_limit", runtime.pids_limit);
        field!("shm_size_bytes", runtime.shm_size_bytes);
        field!("ulimits", runtime.ulimits);
        field!("cap_add", runtime.cap_add);
        field!("cap_drop", runtime.cap_drop);
        field!("security_options", runtime.security_options);
        field!("userns_mode", runtime.userns_mode);
        field!("supplementary_groups", runtime.supplementary_groups);
        field!("sysctls", runtime.sysctls);
        field!("dns_servers", runtime.dns_servers);
        field!("dns_options", runtime.dns_options);
        field!("dns_search", runtime.dns_search);
        field!("extra_hosts", runtime.extra_hosts);
        field!("init", runtime.init);
        field!("logging", runtime.logging);
        field!("tmpfs", runtime.tmpfs);
        field!("binds", runtime.binds);
        field!("configured_mounts", runtime.configured_mounts);
        field!("volumes_from", runtime.volumes_from);
        field!("devices", runtime.devices);
    }

    fn unmapped<T>(&mut self, subject: String, observed: &Observed<T>) {
        if !matches!(observed.availability, Availability::Missing | Availability::Null) {
            self.unsupported(
                subject,
                "Docker field is retained as native evidence but not mapped by this source checkpoint",
                Some(observed.origin),
            );
        }
    }

    fn native_finding(&mut self, finding: &Finding, source_mode: DaemonMode) {
        let unknown_source_context = finding.severity == NativeSeverity::Warning
            && finding.code == FindingCode::CapabilityUnknown
            && finding.field == Some(FieldPath::DaemonMode)
            && finding.resource.is_none()
            && source_mode == DaemonMode::Unknown;
        let summary = if unknown_source_context {
            "Source daemon execution mode is unknown; retained context is not portable configuration or target authority"
        } else {
            "DockerLens reported a typed native finding"
        };
        let severity = match finding.severity {
            NativeSeverity::Info => Severity::Note,
            NativeSeverity::Warning => Severity::Warning,
            NativeSeverity::Error => Severity::Error,
        };
        let code = if severity == Severity::Error {
            self.importer.invalid.clone()
        } else {
            self.importer.unsupported.clone()
        };
        // Native Finding carries no phase. Do not infer acquisition/decoding from its code.
        let native = NativeFinding::new(
            "docker",
            "docker-lens",
            format!("{:?}", finding.code),
            "observation",
            severity,
            summary,
        );
        let field = finding.field.unwrap_or(FieldPath::Other);
        let subject = finding.resource.map_or_else(
            || format!("docker.native.{field:?}"),
            |resource| format!("docker.native.resources[{}].{field:?}", resource.local_index()),
        );
        self.diagnostics.push(
            Diagnostic::new(code.clone(), severity, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
                .with_native_finding(native),
        );
        if unknown_source_context {
            return;
        }
        if severity == Severity::Error {
            self.loss(subject, ConversionKind::Invalid, code);
        } else if severity == Severity::Warning {
            self.loss(subject, ConversionKind::Unsupported, code);
        }
    }

    fn exact(&mut self, subject: impl Into<String>) {
        self.outcomes
            .push(ConversionOutcome::exact(subject).with_origin(self.origin.clone()));
    }

    fn sourced<T>(&self, value: T, promoted: bool) -> Sourced<T> {
        let mut sourced = Sourced::from_source(value, self.origin.clone());
        if promoted {
            sourced.add_origin(Provenance::conversion_decision(self.origin.source_id().clone()));
        }
        sourced
    }

    fn decision_sourced<T>(&self, value: T) -> Sourced<T> {
        let mut sourced = Sourced::from_source(value, self.origin.clone());
        self.add_decision_origins(&mut sourced);
        sourced
    }

    fn add_decision_origins<T>(&self, sourced: &mut Sourced<T>) {
        sourced.add_origin(Provenance::user_override(self.origin.source_id().clone()));
        sourced.add_origin(Provenance::conversion_decision(self.origin.source_id().clone()));
    }

    fn explicit_resource_decision(&mut self, subject: &str) {
        self.outcomes.push(
            ConversionOutcome::exact(format!("{subject}.ownership"))
                .with_origin(self.origin.clone())
                .with_origin(Provenance::user_override(self.origin.source_id().clone()))
                .with_origin(Provenance::conversion_decision(self.origin.source_id().clone())),
        );
    }

    fn mapped(&mut self, subject: impl Into<String>, promoted: bool) {
        let subject = subject.into();
        if promoted {
            self.diagnostics.push(
                Diagnostic::new(
                    self.importer.reconstructed.clone(),
                    Severity::Note,
                    "reviewed effective Docker state reconstructed without a known behavior change",
                )
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
                .with_field(DiagnosticField::new(
                    "observation_origin",
                    DiagnosticValue::plain("effective"),
                ))
                .with_field(DiagnosticField::new(
                    "source_intent_certainty",
                    DiagnosticValue::plain("observed-state-only"),
                ))
                .with_field(DiagnosticField::new(
                    "remediation",
                    DiagnosticValue::plain(
                        "Review whether this setting was authored or inherited before maintaining it.",
                    ),
                )),
            );
            self.outcomes.push(
                ConversionOutcome::exact(subject)
                    .with_origin(self.origin.clone())
                    .with_origin(Provenance::conversion_decision(self.origin.source_id().clone())),
            );
        } else {
            self.exact(subject);
        }
    }

    fn approximate(&mut self, subject: String, summary: &'static str) {
        self.approximate_observed(subject, summary, Origin::Effective);
    }

    fn approximate_observed(&mut self, subject: String, summary: &'static str, origin: Origin) {
        self.diagnostics.push(
            Diagnostic::new(self.importer.approximation.clone(), Severity::Warning, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
                .with_field(DiagnosticField::new(
                    "observation_origin",
                    DiagnosticValue::plain(origin_name(origin)),
                ))
                .with_field(DiagnosticField::new(
                    "remediation",
                    DiagnosticValue::plain(
                        "Review this current-state assumption and repair the target behavior if needed.",
                    ),
                )),
        );
        if let Ok(outcome) = ConversionOutcome::loss(
            subject,
            ConversionKind::Approximate,
            self.importer.approximation.clone(),
        ) {
            self.outcomes.push(
                outcome
                    .with_origin(self.origin.clone())
                    .with_origin(Provenance::conversion_decision(self.origin.source_id().clone())),
            );
        }
    }

    fn promotable<'b, T>(
        &mut self,
        subject: &str,
        observed: &'b Observed<T>,
        enabled: bool,
        flag: &'static str,
    ) -> Option<(&'b T, bool)> {
        if matches!(observed.availability, Availability::Missing | Availability::Null) {
            return None;
        }
        if observed.availability == Availability::Redacted {
            self.unsupported(
                subject.to_owned(),
                "Docker field is unavailable or redacted",
                Some(observed.origin),
            );
            return None;
        }
        let Some(value) = observed.value() else {
            self.unsupported(
                subject.to_owned(),
                "Docker field is unavailable or redacted",
                Some(observed.origin),
            );
            return None;
        };
        match observed.origin {
            Origin::Configured => Some((value, false)),
            Origin::Effective if enabled => Some((value, true)),
            Origin::Effective => {
                self.diagnostics.push(
                    Diagnostic::new(
                        self.importer.unsupported.clone(),
                        Severity::Warning,
                        "effective Docker field requires explicit source-state promotion",
                    )
                    .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                    .with_field(DiagnosticField::new(
                        "available_promotion",
                        DiagnosticValue::plain(flag),
                    ))
                    .with_field(DiagnosticField::new(
                        "observation_origin",
                        DiagnosticValue::plain("effective"),
                    ))
                    .with_field(DiagnosticField::new(
                        "remediation",
                        DiagnosticValue::plain(
                            "Select the reviewed Docker promotion policy or author the field separately.",
                        ),
                    )),
                );
                self.loss(
                    subject.to_owned(),
                    ConversionKind::Unsupported,
                    self.importer.unsupported.clone(),
                );
                None
            }
            Origin::RuntimeAssigned | Origin::Unknown => {
                self.unsupported(
                    subject.to_owned(),
                    "Docker observation does not establish portable authored intent",
                    Some(observed.origin),
                );
                None
            }
        }
    }

    fn selection_required(&mut self, summary: &'static str) {
        let subject = "application.selection";
        self.diagnostics.push(
            Diagnostic::new(self.importer.unsupported.clone(), Severity::Warning, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                .with_field(DiagnosticField::new(
                    "remediation",
                    DiagnosticValue::plain("Select one or more inspected Docker resources before conversion."),
                )),
        );
        self.loss(
            subject.to_owned(),
            ConversionKind::Unsupported,
            self.importer.unsupported.clone(),
        );
    }

    fn unsupported(&mut self, subject: String, summary: &'static str, origin: Option<Origin>) {
        let mut diagnostic = Diagnostic::new(self.importer.unsupported.clone(), Severity::Warning, summary)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
            .with_field(DiagnosticField::new(
                "remediation",
                DiagnosticValue::plain("Review and author this field before accepting a target conversion."),
            ));
        if let Some(origin) = origin {
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "observation_origin",
                DiagnosticValue::plain(origin_name(origin)),
            ));
        }
        self.diagnostics.push(diagnostic);
        self.loss(subject, ConversionKind::Unsupported, self.importer.unsupported.clone());
    }

    fn observation_only(&mut self, subject: String, summary: &'static str, origin: Origin) {
        self.diagnostics.push(
            Diagnostic::new(self.importer.unsupported.clone(), Severity::Note, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                .with_field(DiagnosticField::new(
                    "observation_origin",
                    DiagnosticValue::plain(origin_name(origin)),
                ))
                .with_field(DiagnosticField::new(
                    "source_intent_certainty",
                    DiagnosticValue::plain("observation-only"),
                )),
        );
    }

    fn invalid(&mut self, subject: impl Into<String>, summary: &'static str) {
        let subject = subject.into();
        self.diagnostics.push(
            Diagnostic::new(self.importer.invalid.clone(), Severity::Error, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone()))),
        );
        self.loss(subject, ConversionKind::Invalid, self.importer.invalid.clone());
    }

    fn loss(&mut self, subject: String, kind: ConversionKind, code: DiagnosticCode) {
        if let Ok(outcome) = ConversionOutcome::loss(subject, kind, code) {
            self.outcomes.push(outcome.with_origin(self.origin.clone()));
        }
    }
}

fn public_label_collection<'a>(
    inventory: &'a DecodedInventory,
    decision: &DockerPublicLabelDecision,
) -> Result<&'a Observed<Vec<LabelObservation>>, &'static str> {
    let source = decision.source().as_bytes();
    match decision.kind() {
        DockerPublicLabelResourceKind::Container => {
            let mut matches = inventory
                .containers
                .iter()
                .filter(|container| container.id.value().is_some_and(|id| id.as_bytes() == source));
            let resource = matches
                .next()
                .ok_or("Docker public-label container is not selected inspected inventory")?;
            if matches.next().is_some() || canonical_container_id(&resource.id) != Some(source) {
                return Err("Docker public-label container identity is unavailable or ambiguous");
            }
            Ok(&resource.labels)
        }
        DockerPublicLabelResourceKind::Network => {
            let mut matches = inventory
                .networks
                .iter()
                .filter(|network| network.id.value().is_some_and(|id| id.as_bytes() == source));
            let resource = matches
                .next()
                .ok_or("Docker public-label network is not selected inspected inventory")?;
            if matches.next().is_some() || runtime_network_id(&resource.id) != Some(source) {
                return Err("Docker public-label network identity is unavailable or ambiguous");
            }
            Ok(&resource.labels)
        }
        DockerPublicLabelResourceKind::Volume => {
            let mut matches = inventory
                .volumes
                .iter()
                .filter(|volume| volume.name.value().is_some_and(|name| name.as_bytes() == source));
            let resource = matches
                .next()
                .ok_or("Docker public-label volume is not selected inspected inventory")?;
            if matches.next().is_some()
                || resource.name.availability != Availability::Present
                || resource.name.origin != Origin::Effective
            {
                return Err("Docker public-label volume identity is unavailable or ambiguous");
            }
            Ok(&resource.labels)
        }
    }
}

fn selected_public_label<'a>(
    inventory: &'a DecodedInventory,
    decision: &DockerPublicLabelDecision,
) -> Result<(&'a str, &'a str), &'static str> {
    let labels = public_label_collection(inventory, decision)?;
    if labels.availability != Availability::Present || labels.origin != Origin::Effective {
        return Err("Docker public-label collection has no reviewed effective availability and origin");
    }
    let labels = labels.value().ok_or("Docker public-label collection is unavailable")?;
    let mut matches = labels
        .iter()
        .filter(|label| label.key.as_bytes() == decision.key().as_bytes());
    let label = matches
        .next()
        .ok_or("Docker declared public label key is missing from inspected metadata")?;
    if matches.next().is_some() {
        return Err("Docker declared public label key is ambiguous");
    }
    if label.value.origin != Origin::Effective {
        return Err("Docker declared public label value has no reviewed effective origin");
    }
    let value = label
        .value
        .value()
        .ok_or("Docker declared public label value is unavailable")?;
    if !matches!(
        (label.value.availability, value.is_empty()),
        (Availability::Present, false) | (Availability::Empty, true)
    ) {
        return Err("Docker declared public label value availability is inconsistent or redacted");
    }
    if value.as_bytes().len() > crate::public_labels::PUBLIC_LABEL_VALUE_LIMIT {
        return Err("Docker declared public label value exceeds the BoxFerry reconstruction budget");
    }
    let key = std::str::from_utf8(label.key.as_bytes()).map_err(|_| "Docker public label key is not UTF-8")?;
    let value = std::str::from_utf8(value.as_bytes()).map_err(|_| "Docker public label value is not UTF-8")?;
    if key.contains('\0') || value.contains('\0') {
        return Err("Docker public label text cannot contain NUL");
    }
    Ok((key, value))
}

fn runtime_network_id(observed: &Observed<docker_lens::evidence::ProtectedValue>) -> Option<&[u8]> {
    if observed.availability != Availability::Present || observed.origin != Origin::RuntimeAssigned {
        return None;
    }
    observed
        .value()
        .map(docker_lens::evidence::ProtectedValue::as_bytes)
        .filter(|id| !id.is_empty() && std::str::from_utf8(id).is_ok_and(|id| !id.contains('\0')))
}

fn canonical_container_id(observed: &Observed<docker_lens::evidence::ProtectedValue>) -> Option<&[u8]> {
    if observed.availability != Availability::Present || observed.origin != Origin::RuntimeAssigned {
        return None;
    }
    observed
        .value()
        .map(docker_lens::evidence::ProtectedValue::as_bytes)
        .filter(|id| {
            id.len() == 64
                && id
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
        })
}

fn selected_active_members_only(network: &NetworkObservation, containers: &[ContainerObservation]) -> bool {
    let Some(active) = network.active_endpoints.value().filter(|_| {
        matches!(
            network.active_endpoints.availability,
            Availability::Empty | Availability::Present
        ) && network.active_endpoints.origin == Origin::RuntimeAssigned
    }) else {
        return false;
    };
    if network.active_endpoints.availability == Availability::Empty && !active.is_empty() {
        return false;
    }
    let mut selected = std::collections::BTreeSet::new();
    for container in containers {
        let Some(id) = canonical_container_id(&container.id) else {
            return false;
        };
        if !selected.insert(id) {
            return false;
        }
    }
    let mut members = std::collections::BTreeSet::new();
    active.iter().all(|endpoint| {
        canonical_container_id(&endpoint.container_id).is_some_and(|id| members.insert(id) && selected.contains(id))
    })
}

// DockerLens decodes inspected network/volume Names as effective identity evidence,
// not authored configuration. A retained value alone does not make redacted or
// otherwise unavailable caller-assembled observations usable for correlation.
fn usable_inspected_name(
    observed: &Observed<docker_lens::evidence::ProtectedValue>,
) -> Option<&docker_lens::evidence::ProtectedValue> {
    if observed.availability != Availability::Present || observed.origin != Origin::Effective {
        return None;
    }
    observed.value().filter(|name| !name.is_empty())
}

fn valid_native_network_name(name: &[u8]) -> bool {
    !name.is_empty() && std::str::from_utf8(name).is_ok_and(|name| !name.contains('\0'))
}

fn valid_portable_network_alias(alias: &[u8]) -> bool {
    alias.first().is_some_and(u8::is_ascii_alphanumeric)
        && alias[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.'))
}

fn runtime_identity_alias(alias: &[u8], container: &ContainerObservation) -> bool {
    let inspected_name = container
        .name
        .value()
        .map(docker_lens::evidence::ProtectedValue::as_bytes)
        .map(|name| name.strip_prefix(b"/").unwrap_or(name));
    inspected_name == Some(alias) || matches!(alias.len(), 12 | 64) && alias.iter().all(u8::is_ascii_hexdigit)
}

const fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Configured => "configured",
        Origin::Effective => "effective",
        Origin::RuntimeAssigned => "runtime-assigned",
        Origin::Unknown => "unknown",
    }
}

const fn protocol(protocol: TransportProtocol) -> Protocol {
    match protocol {
        TransportProtocol::Tcp => Protocol::Tcp,
        TransportProtocol::Udp => Protocol::Udp,
        TransportProtocol::Sctp => Protocol::Sctp,
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU16, time::Duration};

    use boxferry_engine::{ConversionKind, ImportAdapter, ImportResult, Severity};
    use boxferry_model::{
        Command, Entrypoint, EnvironmentValue, HealthcheckCommand, Identifier, MountSource, ProvenanceKind,
        ResourceOwnership, SelinuxRelabel, Sourced,
    };
    use docker_lens::{
        acquisition::{Budget, Limits, NativeId, ReadRequest},
        decoder::{
            ContainerObservation, DecodedInventory, DecodedVersion, Healthcheck as NativeHealthcheck, HealthcheckTest,
            HostBinding, MountAccess, MountKind, MountModeInterpretation, MountObservation,
            NetworkAttachment as NativeNetworkAttachment, NetworkModeKind, NetworkModeObservation, NetworkObservation,
            PortKey, PortObservation, RestartPolicy as NativeRestartPolicy, RuntimeObservation, TransportProtocol,
            VolumeObservation, decode_capture,
        },
        evidence::{HttpStatus, ProtectedValue},
        finding::{Finding, FindingCode, Severity as NativeSeverity},
        observation::{Availability, BindRelabel, FieldPath, Observed, Origin, ResourceRef},
        version::{ApiVersion, DaemonFacts, DaemonMode, ObservationId},
    };

    use crate::{
        DockerExporter, DockerImporter, DockerPromotionPolicy, DockerResourceDecision, DockerResourceDecisions,
        DockerSelection, DockerSource,
    };
    use boxferry_engine::{ExportAdapter, LossPolicy, PlatformVersion, TargetProfile, convert_imported};
    use docker_lens::version::{EngineBuild, TargetCapabilityCatalog};

    fn resource_decisions(specs: &[String]) -> Result<DockerResourceDecisions, String> {
        let mut decisions = DockerResourceDecisions::new();
        for spec in specs {
            decisions
                .add(
                    spec.parse::<DockerResourceDecision>()
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(decisions)
    }

    fn absent<T>() -> Observed<T> {
        Observed::unavailable(Availability::Missing, Origin::Unknown)
    }

    fn observed_image(value: &[u8], origin: Origin) -> Observed<ProtectedValue> {
        Observed::present(ProtectedValue::new(value.to_vec()), Availability::Present, origin)
    }

    fn assert_value_reconstruction<T>(value: &Sourced<T>, reconstructed: bool) {
        let kinds = value
            .origins()
            .iter()
            .map(boxferry_model::Provenance::kind)
            .collect::<Vec<_>>();
        let expected = if reconstructed {
            vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
        } else {
            vec![ProvenanceKind::RuntimeObservation]
        };
        assert_eq!(kinds, expected);
    }

    fn assert_exact_reconstruction(imported: &ImportResult, subject: &str, reconstructed: bool) {
        let outcomes = imported
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect::<Vec<_>>();
        assert_eq!(outcomes.len(), 1, "subject: {subject}");
        assert_eq!(outcomes[0].kind(), ConversionKind::Exact, "subject: {subject}");
        let kinds = outcomes[0]
            .origins()
            .iter()
            .map(boxferry_model::Provenance::kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            if reconstructed {
                vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
            } else {
                vec![ProvenanceKind::RuntimeObservation]
            },
            "subject: {subject}"
        );
        assert_eq!(
            imported.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFD0003"
                    && diagnostic.severity() == Severity::Note
                    && diagnostic
                        .fields()
                        .iter()
                        .any(|field| field.name() == "subject" && field.value().expose() == subject)
            }),
            reconstructed,
            "subject: {subject}"
        );
        assert!(!format!("{:?}", imported.diagnostics()).contains("child-private-canary"));
    }

    fn typed_publication(
        host_port: Observed<u16>,
        host_ip: Observed<ProtectedValue>,
        bindings_origin: Origin,
    ) -> PortObservation {
        PortObservation {
            key: PortKey {
                container_port: 8080,
                protocol: TransportProtocol::Tcp,
            },
            bindings: Observed::present(
                vec![HostBinding { host_ip, host_port }],
                Availability::Present,
                bindings_origin,
            ),
        }
    }

    fn typed_child<T>(
        field: &str,
        selected: &str,
        value: T,
        origin: Origin,
        availability: Availability,
    ) -> Observed<T> {
        if field == selected {
            if availability == Availability::Redacted {
                Observed::unavailable(availability, origin)
            } else {
                Observed::present(value, availability, origin)
            }
        } else {
            Observed::present(value, Availability::Present, Origin::Configured)
        }
    }

    fn valued_redacted_child<T>(value: T, redacted: bool, origin: Origin) -> Observed<T> {
        // Public construction can retain a value despite its Redacted state.
        Observed::present(
            value,
            if redacted {
                Availability::Redacted
            } else {
                Availability::Present
            },
            if redacted { origin } else { Origin::Configured },
        )
    }

    fn assert_refused_valued_redaction(imported: &ImportResult, compound: &str, leaf: &str) {
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == leaf && outcome.kind() == ConversionKind::Unsupported }),
            "refused leaf: {leaf}"
        );
        assert!(
            !imported
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject().starts_with(compound) && outcome.kind() == ConversionKind::Exact }),
            "refused compound: {compound}"
        );
        assert!(!imported.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFD0003"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().expose().starts_with(compound))
        }));
        assert!(!format!("{imported:?}").contains("valued-private-canary"));
        assert!(!format!("{imported:?}").contains("192.0.2.137"));
    }

    fn typed_healthcheck(
        child: &str,
        origin: Origin,
        availability: Availability,
        disabled: bool,
    ) -> Observed<NativeHealthcheck> {
        Observed::present(
            NativeHealthcheck {
                test: typed_child(
                    "test",
                    child,
                    if disabled {
                        HealthcheckTest::None
                    } else {
                        HealthcheckTest::Cmd(vec![ProtectedValue::new(b"child-private-canary".to_vec())])
                    },
                    origin,
                    availability,
                ),
                interval_ns: typed_child("interval", child, 1_000_000_000, origin, availability),
                timeout_ns: typed_child("timeout", child, 2_000_000_000, origin, availability),
                start_period_ns: typed_child("start_period", child, 3_000_000_000, origin, availability),
                start_interval_ns: typed_child("start_interval", child, 4_000_000_000, origin, availability),
                retries: typed_child("retries", child, 3, origin, availability),
            },
            Availability::Present,
            if child == "outer" { origin } else { Origin::Configured },
        )
    }

    fn typed_mount(
        bind: bool,
        child: &str,
        origin: Origin,
        availability: Availability,
    ) -> Observed<Vec<MountObservation>> {
        Observed::present(
            vec![MountObservation {
                kind: if bind { MountKind::Bind } else { MountKind::Volume },
                source: if bind {
                    typed_child(
                        "logical_source",
                        child,
                        ProtectedValue::new(b"/same-host/child-private-canary".to_vec()),
                        origin,
                        availability,
                    )
                } else {
                    observed_image(b"/daemon-only/child-private-canary", Origin::Effective)
                },
                destination: typed_child(
                    "destination",
                    child,
                    ProtectedValue::new(b"/data".to_vec()),
                    origin,
                    availability,
                ),
                name: if bind {
                    absent()
                } else {
                    typed_child(
                        "logical_source",
                        child,
                        ProtectedValue::new(b"private-volume".to_vec()),
                        origin,
                        availability,
                    )
                },
                read_write: typed_child("read_write", child, false, origin, availability),
                mode: explicit_empty_mode(Origin::Configured),
                mode_interpretation: explicit_empty_mode_interpretation(Origin::Configured),
                propagation: absent(),
            }],
            Availability::Present,
            if child == "outer" { origin } else { Origin::Configured },
        )
    }

    fn container(image: Observed<ProtectedValue>) -> ContainerObservation {
        ContainerObservation {
            reference: ResourceRef::new(1),
            id: observed_image(
                b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Origin::RuntimeAssigned,
            ),
            name: observed_image(b"/private-native-name", Origin::Effective),
            image,
            image_id: absent(),
            labels: absent(),
            user: absent(),
            working_directory: absent(),
            hostname: absent(),
            environment: absent(),
            exposed_ports: absent(),
            configured_ports: absent(),
            runtime_ports: absent(),
            mounts: absent(),
            networks: absent(),
            network_mode: absent(),
            command: absent(),
            entrypoint: absent(),
            healthcheck: absent(),
            restart_policy: absent(),
            runtime: RuntimeObservation {
                read_only_rootfs: absent(),
                memory_bytes: absent(),
                pids_limit: absent(),
                shm_size_bytes: absent(),
                ulimits: absent(),
                cap_add: absent(),
                cap_drop: absent(),
                security_options: absent(),
                userns_mode: absent(),
                supplementary_groups: absent(),
                sysctls: absent(),
                dns_servers: absent(),
                dns_options: absent(),
                dns_search: absent(),
                extra_hosts: absent(),
                init: absent(),
                stop_signal: absent(),
                stop_timeout: absent(),
                logging: absent(),
                tmpfs: absent(),
                binds: absent(),
                configured_mounts: absent(),
                volumes_from: absent(),
                devices: absent(),
            },
        }
    }

    fn import_stop_behavior(
        signal: Observed<ProtectedValue>,
        timeout: Observed<i64>,
        policy: DockerPromotionPolicy,
    ) -> Result<ImportResult, String> {
        let mut observed = container(absent());
        observed.runtime.stop_signal = signal;
        observed.runtime.stop_timeout = timeout;
        let source = source(vec![observed])?.with_promotion_policy(policy);
        Ok(DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source))
    }

    #[test]
    fn typed_stop_behavior_maps_finite_values_and_reconstruction_provenance() -> Result<(), String> {
        for (origin, policy, reconstructed) in [
            (Origin::Configured, DockerPromotionPolicy::conservative(), false),
            (Origin::Effective, DockerPromotionPolicy::portable_application(), true),
        ] {
            for (seconds, spelling) in [(0, "0s"), (20, "20s"), (i64::from(u32::MAX), "4294967295s")] {
                let imported = import_stop_behavior(
                    observed_image(b"SIGUSR1", origin),
                    Observed::present(seconds, Availability::Present, origin),
                    policy,
                )?;
                let service = imported.application().ok_or("missing application")?.services()[0].value();
                let signal = service.stop_signal().ok_or("missing stop signal")?;
                let timeout = service.stop_timeout().ok_or("missing stop timeout")?;
                assert_eq!(signal.value().expose(), "SIGUSR1");
                assert!(!signal.value().is_sensitive());
                assert_eq!(timeout.value().as_str(), spelling);
                assert_value_reconstruction(signal, reconstructed);
                assert_value_reconstruction(timeout, reconstructed);
                assert_exact_reconstruction(&imported, "services[0].stop_signal", reconstructed);
                assert_exact_reconstruction(&imported, "services[0].stop_timeout", reconstructed);
            }
        }
        Ok(())
    }

    #[test]
    fn typed_stop_behavior_never_promotes_absent_redacted_or_unavailable_values() -> Result<(), String> {
        let portable = DockerPromotionPolicy::portable_application();
        for availability in [
            Availability::Missing,
            Availability::Null,
            Availability::Redacted,
            Availability::Present,
        ] {
            for retained_value in [false, true] {
                let signal = if retained_value {
                    Observed::present(
                        ProtectedValue::new(b"stop-private-canary".to_vec()),
                        availability,
                        Origin::Effective,
                    )
                } else {
                    Observed::unavailable(availability, Origin::Effective)
                };
                let timeout = if retained_value {
                    Observed::present(17, availability, Origin::Effective)
                } else {
                    Observed::unavailable(availability, Origin::Effective)
                };
                // Present with a usable value is covered by the positive table.
                if availability == Availability::Present && retained_value {
                    continue;
                }
                let imported = import_stop_behavior(signal, timeout, portable)?;
                let service = imported.application().ok_or("missing application")?.services()[0].value();
                assert!(service.stop_signal().is_none());
                assert!(service.stop_timeout().is_none());
                let expected = if matches!(availability, Availability::Missing | Availability::Null) {
                    None
                } else {
                    Some(ConversionKind::Unsupported)
                };
                for subject in ["services[0].stop_signal", "services[0].stop_timeout"] {
                    let outcomes = imported
                        .outcomes()
                        .iter()
                        .filter(|outcome| outcome.subject() == subject)
                        .map(boxferry_engine::ConversionOutcome::kind)
                        .collect::<Vec<_>>();
                    assert_eq!(outcomes, expected.into_iter().collect::<Vec<_>>());
                }
                assert!(!format!("{imported:?}").contains("stop-private-canary"));
            }
        }
        Ok(())
    }

    #[test]
    fn typed_stop_behavior_requires_reviewed_origin_and_explicit_effective_promotion() -> Result<(), String> {
        for (origin, policy) in [
            (Origin::Effective, DockerPromotionPolicy::conservative()),
            (
                Origin::Effective,
                DockerPromotionPolicy::portable_application().with_portable_effective_settings(false),
            ),
            (Origin::RuntimeAssigned, DockerPromotionPolicy::portable_application()),
            (Origin::Unknown, DockerPromotionPolicy::portable_application()),
        ] {
            let imported = import_stop_behavior(
                observed_image(b"stop-private-canary", origin),
                Observed::present(20, Availability::Present, origin),
                policy,
            )?;
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.stop_signal().is_none());
            assert!(service.stop_timeout().is_none());
            for subject in ["services[0].stop_signal", "services[0].stop_timeout"] {
                let outcomes = imported
                    .outcomes()
                    .iter()
                    .filter(|outcome| outcome.subject() == subject)
                    .collect::<Vec<_>>();
                assert_eq!(outcomes.len(), 1);
                assert_eq!(outcomes[0].kind(), ConversionKind::Unsupported);
                if origin == Origin::Effective {
                    assert!(imported.diagnostics().iter().any(|diagnostic| {
                        diagnostic
                            .fields()
                            .iter()
                            .any(|field| field.name() == "subject" && field.value().expose() == subject)
                            && diagnostic.fields().iter().any(|field| {
                                field.name() == "available_promotion"
                                    && field.value().expose()
                                        == "DockerPromotionPolicy::with_portable_effective_settings"
                            })
                    }));
                }
            }
            assert!(!format!("{imported:?}").contains("stop-private-canary"));
        }
        Ok(())
    }

    #[test]
    fn typed_stop_signal_rejects_empty_nul_and_invalid_utf8_without_value_disclosure() -> Result<(), String> {
        for (bytes, availability, expected) in [
            (b"".as_slice(), Availability::Empty, ConversionKind::Unsupported),
            (
                b"stop-private-canary\0".as_slice(),
                Availability::Present,
                ConversionKind::Invalid,
            ),
            (
                b"stop-private-canary\xff".as_slice(),
                Availability::Present,
                ConversionKind::Invalid,
            ),
        ] {
            let imported = import_stop_behavior(
                Observed::present(ProtectedValue::new(bytes.to_vec()), availability, Origin::Effective),
                absent(),
                DockerPromotionPolicy::portable_application(),
            )?;
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.stop_signal().is_none());
            let outcomes = imported
                .outcomes()
                .iter()
                .filter(|outcome| outcome.subject() == "services[0].stop_signal")
                .collect::<Vec<_>>();
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].kind(), expected);
            assert!(!format!("{imported:?}").contains("stop-private-canary"));
        }
        Ok(())
    }

    #[test]
    fn typed_stop_timeout_retains_negative_and_unrepresentable_values_as_unsupported() -> Result<(), String> {
        for seconds in [i64::MIN, -2, -1, i64::from(u32::MAX) + 1, i64::MAX] {
            let imported = import_stop_behavior(
                absent(),
                Observed::present(seconds, Availability::Present, Origin::Effective),
                DockerPromotionPolicy::portable_application(),
            )?;
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.stop_timeout().is_none());
            let outcomes = imported
                .outcomes()
                .iter()
                .filter(|outcome| outcome.subject() == "services[0].stop_timeout")
                .collect::<Vec<_>>();
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].kind(), ConversionKind::Unsupported);
            assert_eq!(outcomes[0].origins().len(), 1);
            assert!(imported.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFD0002"
                    && diagnostic
                        .fields()
                        .iter()
                        .any(|field| field.name() == "subject" && field.value().expose() == "services[0].stop_timeout")
            }));
            assert!(!format!("{:?}", imported.diagnostics()).contains(&seconds.to_string()));
        }
        Ok(())
    }

    #[test]
    fn decoded_config_stop_behavior_preserves_zero_and_negative_evidence() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (config, expected_signal, expected_timeout, unsupported_timeout) in [
            (
                r#""StopSignal":"SIGUSR1","StopTimeout":0"#,
                Some("SIGUSR1"),
                Some("0s"),
                false,
            ),
            (
                r#""StopSignal":"SIGTERM","StopTimeout":20"#,
                Some("SIGTERM"),
                Some("20s"),
                false,
            ),
            (
                r#""StopSignal":"SIGTERM","StopTimeout":4294967295"#,
                Some("SIGTERM"),
                Some("4294967295s"),
                false,
            ),
            (
                r#""StopSignal":"SIGTERM","StopTimeout":-1"#,
                Some("SIGTERM"),
                None,
                true,
            ),
            (r#""StopSignal":null,"StopTimeout":null"#, None, None, false),
            (r#""Image":"registry.invalid/app:1""#, None, None, false),
        ] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{{config}}}}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(
                service.stop_signal().map(|value| value.value().expose()),
                expected_signal
            );
            assert_eq!(
                service.stop_timeout().map(|value| value.value().as_str()),
                expected_timeout
            );
            if expected_signal.is_some() {
                assert_exact_reconstruction(&imported, "services[0].stop_signal", true);
            }
            if expected_timeout.is_some() {
                assert_exact_reconstruction(&imported, "services[0].stop_timeout", true);
            }
            assert_eq!(
                imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == "services[0].stop_timeout" && outcome.kind() == ConversionKind::Unsupported
                }),
                unsupported_timeout
            );
            if unsupported_timeout {
                assert_eq!(source.inventory().containers[0].runtime.stop_timeout.value(), Some(&-1));
            }
        }
        Ok(())
    }

    fn network() -> NetworkObservation {
        NetworkObservation {
            reference: ResourceRef::new(2),
            id: observed_image(
                b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                Origin::RuntimeAssigned,
            ),
            active_endpoints: Observed::present(Vec::new(), Availability::Empty, Origin::RuntimeAssigned),
            name: observed_image(b"private-network", Origin::Effective),
            driver: observed_image(b"bridge", Origin::Effective),
            internal: absent(),
            enable_ipv6: absent(),
            ipam_driver: absent(),
            ipam_configs: absent(),
            options: absent(),
            labels: absent(),
        }
    }

    fn network_attachment() -> NativeNetworkAttachment {
        NativeNetworkAttachment {
            name: ProtectedValue::new(b"private-network".to_vec()),
            network_id: observed_image(
                b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                Origin::RuntimeAssigned,
            ),
            ip_address: absent(),
            ipv6_address: absent(),
            requested_ipv4_address: absent(),
            requested_ipv6_address: absent(),
            aliases: absent(),
        }
    }

    fn volume() -> VolumeObservation {
        VolumeObservation {
            reference: ResourceRef::new(3),
            name: observed_image(b"private-volume", Origin::Effective),
            driver: observed_image(b"local", Origin::Effective),
            mountpoint: observed_image(b"/private/mountpoint", Origin::RuntimeAssigned),
            options: absent(),
            labels: absent(),
        }
    }

    fn named_mount() -> MountObservation {
        MountObservation {
            kind: MountKind::Volume,
            source: observed_image(b"/private/runtime/volume", Origin::Effective),
            destination: observed_image(b"/data", Origin::Effective),
            name: observed_image(b"private-volume", Origin::Effective),
            read_write: Observed::present(true, Availability::Present, Origin::Effective),
            mode: explicit_empty_mode(Origin::Effective),
            mode_interpretation: explicit_empty_mode_interpretation(Origin::Effective),
            propagation: absent(),
        }
    }

    fn bind_mount() -> MountObservation {
        MountObservation {
            kind: MountKind::Bind,
            source: observed_image(b"/private/same-host/data", Origin::Effective),
            destination: observed_image(b"/data", Origin::Effective),
            name: absent(),
            read_write: Observed::present(true, Availability::Present, Origin::Effective),
            mode: explicit_empty_mode(Origin::Effective),
            mode_interpretation: explicit_empty_mode_interpretation(Origin::Effective),
            propagation: absent(),
        }
    }

    fn explicit_empty_mode(origin: Origin) -> Observed<ProtectedValue> {
        Observed::present(ProtectedValue::new(Vec::new()), Availability::Empty, origin)
    }

    fn explicit_empty_mode_interpretation(origin: Origin) -> Observed<MountModeInterpretation> {
        Observed::present(
            MountModeInterpretation::Supported {
                access: None,
                relabel: None,
            },
            Availability::Empty,
            origin,
        )
    }

    fn mode_bind(read_write: bool, relabel: BindRelabel, origin: Origin) -> MountObservation {
        let mut mount = bind_mount();
        mount.read_write = Observed::present(read_write, Availability::Present, Origin::Effective);
        // Opaque synthetic raw evidence proves this seam consumes the typed result.
        // Separate decoded tests independently exercise actual native mode syntax.
        mount.mode = observed_image(b"protected-native-mode-canary", origin);
        mount.mode_interpretation = Observed::present(
            MountModeInterpretation::Supported {
                access: Some(if read_write {
                    MountAccess::ReadWrite
                } else {
                    MountAccess::ReadOnly
                }),
                relabel: Some(relabel),
            },
            Availability::Present,
            origin,
        );
        mount
    }

    #[test]
    fn inspected_mount_paths_are_literal_for_binds_and_named_volume_targets() -> Result<(), String> {
        for bind in [false, true] {
            let mut mount = if bind { bind_mount() } else { named_mount() };
            mount.destination = observed_image(b"/data/$TARGET%i", Origin::Effective);
            if bind {
                mount.source = observed_image(b"/srv/$ROOT%h", Origin::Effective);
            }
            let imported = import_one_mount(
                mount,
                DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
            )?;
            let service = imported.application().ok_or("missing mount application")?.services()[0].value();
            assert!(service.execution_environment().is_none());
            assert_eq!(service.mounts().len(), 1);
            let mount = service.mounts()[0].value();
            assert!(mount.paths_are_literal());
            assert_eq!(mount.target(), "/data/$TARGET%i");
            if bind {
                assert!(matches!(mount.source(), MountSource::HostPath(source) if source == "/srv/$ROOT%h"));
            } else {
                assert!(matches!(mount.source(), MountSource::Volume(name) if name.as_str() == "docker-volume-1"));
            }
        }
        Ok(())
    }

    fn import_one_mount(mount: MountObservation, policy: DockerPromotionPolicy) -> Result<ImportResult, String> {
        let volumes = if mount.kind == MountKind::Volume {
            vec![volume()]
        } else {
            Vec::new()
        };
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        observed.mounts = Observed::present(vec![mount], Availability::Present, Origin::Effective);
        let source = source_with_resources(vec![observed], Vec::new(), volumes, DockerSelection::Explicit)?
            .with_promotion_policy(policy);
        Ok(DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source))
    }

    fn labelled_volume(read_write: bool, relabel: BindRelabel, origin: Origin) -> MountObservation {
        let mode = mode_bind(read_write, relabel, origin);
        let mut mount = named_mount();
        mount.read_write = mode.read_write;
        mount.mode = mode.mode;
        mount.mode_interpretation = mode.mode_interpretation;
        mount
    }

    fn assert_volume_label_limitation(imported: &ImportResult, read_write: bool) -> Result<(), String> {
        let service = imported
            .application()
            .ok_or("missing named-volume application")?
            .services()[0]
            .value();
        assert_eq!(service.mounts().len(), 1);
        let mapped = &service.mounts()[0];
        assert!(matches!(mapped.value().source(), MountSource::Volume(name) if name.as_str() == "docker-volume-1"));
        assert_eq!(mapped.value().target(), "/data");
        assert_eq!(mapped.value().read_only(), !read_write);
        assert_eq!(mapped.value().selinux_relabel(), None);
        assert_value_reconstruction(mapped, true);
        let subject = "services[0].mounts[0].mode";
        let outcomes = imported
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect::<Vec<_>>();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].kind(), ConversionKind::Unsupported);
        let diagnostics = imported
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().expose() == subject)
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code().as_str(), "BFD0002");
        assert_eq!(diagnostics[0].severity(), Severity::Warning);
        assert!(diagnostics[0].summary().contains("review target volume labeling"));
        assert!(!diagnostics[0].summary().contains("implicit"));
        assert!(!diagnostics[0].summary().contains("authored"));
        assert!(
            !imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "services[0].mounts[0].selinux_relabel")
        );
        let diagnostic_text = format!("{:?}{:?}", imported.diagnostics(), imported.outcomes());
        for protected in ["private-volume", "/private/", "protected-native-mode-canary"] {
            assert!(!diagnostic_text.contains(protected));
        }
        Ok(())
    }

    #[test]
    fn explicit_empty_exposed_ports_retain_configured_and_promoted_collection_origins() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (origin, policy, promoted) in [
            (Origin::Configured, DockerPromotionPolicy::conservative(), false),
            (Origin::Effective, DockerPromotionPolicy::portable_application(), true),
        ] {
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            observed.exposed_ports = Observed::present(Vec::new(), Availability::Empty, origin);
            let imported = adapter.import(&source(vec![observed])?.with_promotion_policy(policy));
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(service.exposed_ports().map(<[_]>::len), Some(0));
            assert_eq!(
                service
                    .exposed_ports_origins()
                    .iter()
                    .map(boxferry_model::Provenance::kind)
                    .collect::<Vec<_>>(),
                if promoted {
                    vec![ProvenanceKind::RuntimeObservation, ProvenanceKind::ConversionDecision]
                } else {
                    vec![ProvenanceKind::RuntimeObservation]
                }
            );
            assert!(
                service
                    .exposed_ports_origins()
                    .iter()
                    .all(|origin| origin.source_id().as_str() == "docker")
            );
            assert_exact_reconstruction(&imported, "services[0].exposed_ports", promoted);
        }
        Ok(())
    }

    #[test]
    fn empty_exposed_ports_do_not_bypass_source_consent_or_unavailable_observations() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (availability, policy, unsupported) in [
            (Availability::Empty, DockerPromotionPolicy::conservative(), true),
            (
                Availability::Missing,
                DockerPromotionPolicy::portable_application(),
                false,
            ),
            (Availability::Null, DockerPromotionPolicy::portable_application(), false),
            (
                Availability::Redacted,
                DockerPromotionPolicy::portable_application(),
                true,
            ),
        ] {
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            // Deliberately retain Some(empty): availability must still govern use.
            observed.exposed_ports = Observed::present(Vec::new(), availability, Origin::Effective);
            let imported = adapter.import(&source(vec![observed])?.with_promotion_policy(policy));
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.exposed_ports().is_none());
            assert!(service.exposed_ports_origins().is_empty());
            assert!(service.image().is_some());
            assert_eq!(
                imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == "services[0].exposed_ports" && outcome.kind() == ConversionKind::Unsupported
                }),
                unsupported
            );
        }
        Ok(())
    }

    #[test]
    fn inspected_resource_names_need_present_effective_identity_before_links_or_mounts() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (availability, origin, usable) in [
            (Availability::Present, Origin::Effective, true),
            (Availability::Redacted, Origin::Effective, false),
            (Availability::Missing, Origin::Effective, false),
            (Availability::Null, Origin::Effective, false),
            (Availability::Empty, Origin::Effective, false),
            (Availability::Present, Origin::Configured, false),
            (Availability::Present, Origin::RuntimeAssigned, false),
            (Availability::Present, Origin::Unknown, false),
        ] {
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            observed.user = observed_image(b"1000", Origin::Configured);
            observed.networks = Observed::present(
                vec![network_attachment()],
                Availability::Present,
                Origin::RuntimeAssigned,
            );
            observed.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
            let mut inspected_network = network();
            inspected_network.name =
                Observed::present(ProtectedValue::new(b"private-network".to_vec()), availability, origin);
            let mut inspected_volume = volume();
            inspected_volume.name =
                Observed::present(ProtectedValue::new(b"private-volume".to_vec()), availability, origin);
            let source = source_with_resources(
                vec![observed],
                vec![inspected_network],
                vec![inspected_volume],
                DockerSelection::Explicit,
            )?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(
                service.networks().len(),
                usize::from(usable),
                "{availability:?}/{origin:?}"
            );
            assert_eq!(
                service.mounts().len(),
                usize::from(usable),
                "{availability:?}/{origin:?}"
            );
            assert_eq!(service.image().map(|image| image.value().as_str()), Some("app:1"));
            assert_eq!(service.user().map(|user| user.value().expose()), Some("1000"));
            for subject in ["services[0].networks[0]", "services[0].mounts[0].name"] {
                assert_eq!(
                    imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported
                    }),
                    !usable,
                    "{subject}: {availability:?}/{origin:?}"
                );
            }
            let diagnostics = format!("{:?}", imported.diagnostics());
            assert!(!diagnostics.contains("private-network"));
            assert!(!diagnostics.contains("private-volume"));
        }
        Ok(())
    }

    fn source(containers: Vec<ContainerObservation>) -> Result<DockerSource, String> {
        source_with_resources(containers, Vec::new(), Vec::new(), DockerSelection::Unspecified)
    }

    fn source_with_resources(
        containers: Vec<ContainerObservation>,
        networks: Vec<NetworkObservation>,
        volumes: Vec<VolumeObservation>,
        selection: DockerSelection,
    ) -> Result<DockerSource, String> {
        source_with_findings(containers, networks, volumes, selection, Vec::new())
    }

    fn source_with_findings(
        containers: Vec<ContainerObservation>,
        networks: Vec<NetworkObservation>,
        volumes: Vec<VolumeObservation>,
        selection: DockerSelection,
        findings: Vec<Finding>,
    ) -> Result<DockerSource, String> {
        let observation_id = ObservationId::fresh().map_err(|error| format!("{error:?}"))?;
        let name = Identifier::new("reviewed-app").map_err(|error| error.to_string())?;
        Ok(DockerSource::new(
            name,
            DecodedInventory {
                observation_id,
                version: DecodedVersion {
                    daemon: DaemonFacts {
                        observation_id,
                        release: None,
                        api_version: None,
                        minimum_api_version: None,
                        mode: DaemonMode::Unknown,
                        capabilities: Vec::new(),
                    },
                    resource_support: docker_lens::resource_support::ResourceSupportObservation::missing(
                        observation_id,
                    ),
                    client_release: None,
                    distribution_package_revision: None,
                    requested_api_versions: Vec::new(),
                },
                containers,
                discovered_containers: Vec::new(),
                selected_roots: Vec::new(),
                networks,
                volumes,
                findings,
            },
        )
        .with_selection(selection))
    }

    fn decoded_effective_source() -> Result<DockerSource, String> {
        const INSPECT: &str = r#"{
            "Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "Name":"/private-native-name",
            "Config":{
                "Image":"registry.invalid/app:1",
                "Cmd":["/bin/echo","private-command-argument"],
                "Entrypoint":["/entrypoint"],
                "User":"1000",
                "WorkingDir":"/srv/app",
                "Env":["TOKEN=private-environment-value","MODE=prod"],
                "ExposedPorts":{"8080/tcp":{}},
                "Healthcheck":{"Test":["CMD","/bin/true"],"Interval":1000000000,"Timeout":1000000000,"Retries":3}
            },
            "HostConfig":{
                "PortBindings":{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"18080"},{"HostIp":"::1","HostPort":"18081"}]},
                "RestartPolicy":{"Name":"unless-stopped","MaximumRetryCount":0}
            },
            "Mounts":[]
        }"#;
        decoded_source_fixture(INSPECT, None, None)
    }

    fn decoded_source_fixture(
        inspect: &str,
        volume: Option<&str>,
        network: Option<&str>,
    ) -> Result<DockerSource, String> {
        decoded_source_fixture_at_api(inspect, volume, network, 49)
    }

    fn decoded_source_fixture_at_api(
        inspect: &str,
        volume: Option<&str>,
        network: Option<&str>,
        api_minor: u16,
    ) -> Result<DockerSource, String> {
        const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut budget = Budget::new(Limits {
            max_requests: 4,
            max_selected_resources: 1,
            max_expansions: 3,
            max_response_bytes: 8192,
            max_total_bytes: 16384,
            max_elapsed: Duration::from_secs(2),
        })
        .map_err(|error| format!("{error:?}"))?;
        let status = HttpStatus::new(200).map_err(|error| format!("{error:?}"))?;
        let api = ApiVersion::new(NonZeroU16::new(1).ok_or("invalid fixture API major")?, api_minor);
        let version = format!(r#"{{"Version":"29.7.1","ApiVersion":"1.{api_minor}","MinAPIVersion":"1.40"}}"#);
        budget
            .record_request(ReadRequest::DaemonVersion, None, None)
            .map_err(|error| format!("{error:?}"))?;
        budget
            .read_response(status, version.as_bytes())
            .map_err(|error| format!("{error:?}"))?;
        budget.record_selection(1).map_err(|error| format!("{error:?}"))?;
        budget
            .record_request(
                ReadRequest::InspectContainer(NativeId::new(ID.to_owned()).ok_or("invalid fixture container ID")?),
                Some(ResourceRef::new(1)),
                Some(api),
            )
            .map_err(|error| format!("{error:?}"))?;
        budget
            .read_response(status, inspect.as_bytes())
            .map_err(|error| format!("{error:?}"))?;
        if let Some(volume) = volume {
            budget
                .record_request(
                    ReadRequest::InspectVolume(
                        NativeId::new("private-volume".to_owned()).ok_or("invalid fixture volume ID")?,
                    ),
                    Some(ResourceRef::new(2)),
                    Some(api),
                )
                .map_err(|error| format!("{error:?}"))?;
            budget
                .read_response(status, volume.as_bytes())
                .map_err(|error| format!("{error:?}"))?;
        }
        if let Some(network) = network {
            budget
                .record_request(
                    ReadRequest::InspectNetwork(
                        NativeId::new("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned())
                            .ok_or("invalid fixture network ID")?,
                    ),
                    Some(ResourceRef::new(if volume.is_some() { 3 } else { 2 })),
                    Some(api),
                )
                .map_err(|error| format!("{error:?}"))?;
            budget
                .read_response(status, network.as_bytes())
                .map_err(|error| format!("{error:?}"))?;
        }
        let capture = budget.into_capture().map_err(|error| format!("{error:?}"))?;
        let inventory = decode_capture(&capture).map_err(|error| format!("{error:?}"))?;
        let name = Identifier::new("reviewed-app").map_err(|error| error.to_string())?;
        Ok(DockerSource::new(name, inventory).with_selection(DockerSelection::Explicit))
    }

    #[test]
    fn configured_image_maps_without_exposing_effective_native_name() -> Result<(), String> {
        let source = source(vec![container(observed_image(
            b"registry.invalid/app:1",
            Origin::Configured,
        ))])?;
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let application = imported.application().ok_or("missing application")?;
        assert_eq!(application.services().len(), 1);
        let service = application.services()[0].value();
        assert_eq!(service.name().as_str(), "docker-service-1");
        assert_eq!(
            service.image().map(|image| image.value().as_str()),
            Some("registry.invalid/app:1")
        );
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == "services[0].image" && outcome.kind() == ConversionKind::Exact })
        );
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].runtime_name" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{imported:?}").contains("private-native-name"));
        Ok(())
    }

    #[test]
    fn actual_decoder_effective_fields_need_policy_and_retain_decision_provenance() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let conservative_source = decoded_effective_source()?;
        assert_eq!(
            conservative_source.inventory().containers[0].image.origin,
            Origin::Effective
        );
        let conservative = adapter.import(&conservative_source);
        let service = conservative
            .application()
            .ok_or("missing conservative application")?
            .services()[0]
            .value();
        assert!(service.image().is_none());
        assert!(service.command().is_none());
        assert!(conservative.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].image" && outcome.kind() == ConversionKind::Unsupported
        }));

        let portable_source =
            decoded_effective_source()?.with_promotion_policy(DockerPromotionPolicy::portable_application());
        let portable = adapter.import(&portable_source);
        assert_portable_effective_fields(&portable)
    }

    fn assert_portable_effective_fields(portable: &ImportResult) -> Result<(), String> {
        let service = portable.application().ok_or("missing portable application")?.services()[0].value();
        assert_eq!(
            service.image().map(|image| image.value().as_str()),
            Some("registry.invalid/app:1")
        );
        assert!(service.command().is_some());
        assert!(service.entrypoint().is_some());
        assert!(service.user().is_some());
        assert!(service.working_directory().is_some());
        assert!(service.restart_policy().is_some());
        assert!(service.healthcheck().is_some());
        assert!(
            matches!(service.command().map(Sourced::value), Some(Command::Exec(parts)) if parts.iter().all(|part| !part.is_sensitive()))
        );
        assert!(
            matches!(service.entrypoint().map(Sourced::value), Some(Entrypoint::Exec(parts)) if parts.iter().all(|part| !part.is_sensitive()))
        );
        assert!(service.user().is_some_and(|value| !value.value().is_sensitive()));
        assert!(
            service
                .working_directory()
                .is_some_and(|value| !value.value().is_sensitive())
        );
        assert!(
            matches!(service.healthcheck().and_then(|value| value.value().command()).map(Sourced::value), Some(HealthcheckCommand::Exec(parts)) if parts.iter().all(|part| !part.is_sensitive()))
        );
        let runtime_name = service.runtime_name().ok_or("missing promoted Docker runtime name")?;
        assert_eq!(runtime_name.value().expose(), "private-native-name");
        assert!(!runtime_name.value().is_sensitive());
        assert!(
            runtime_name
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
        );
        assert!(
            runtime_name
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        );
        assert!(portable.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].runtime_name" && outcome.kind() == ConversionKind::Approximate
        }));
        assert_eq!(service.ports().len(), 2);
        assert_eq!(service.exposed_ports().map(<[_]>::len), Some(1));
        assert_eq!(service.environment().len(), 2);
        assert!(
            service
                .environment()
                .iter()
                .all(|entry| matches!(entry.value().value(), EnvironmentValue::Required))
        );
        let image = service.image().ok_or("missing promoted image")?;
        assert!(
            image
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
        );
        assert!(
            image
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        );
        assert!(portable.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].image"
                && outcome.kind() == ConversionKind::Exact
                && outcome
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        }));
        assert!(
            portable
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
        );
        assert!(!format!("{portable:?}").contains("private-environment-value"));
        assert!(!format!("{:?}", portable.diagnostics()).contains("private-command-argument"));
        assert!(!format!("{:?}", portable.diagnostics()).contains("private-native-name"));
        Ok(())
    }

    #[test]
    fn protected_environment_literal_needs_separate_source_permission() -> Result<(), String> {
        let source = decoded_effective_source()?.with_promotion_policy(
            DockerPromotionPolicy::portable_application().with_protected_environment_values(true),
        );
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert!(service.environment().iter().all(|entry| {
            matches!(entry.value().value(), EnvironmentValue::Literal(value) if value.is_sensitive())
        }));
        assert!(!format!("{imported:?}").contains("private-environment-value"));
        Ok(())
    }

    #[test]
    fn local_image_id_is_not_a_portable_named_image() -> Result<(), String> {
        let image = observed_image(
            b"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Origin::Effective,
        );
        let source =
            source(vec![container(image)])?.with_promotion_policy(DockerPromotionPolicy::portable_application());
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        assert!(
            imported.application().ok_or("missing application")?.services()[0]
                .value()
                .image()
                .is_none()
        );
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].image" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{imported:?}").contains("sha256:aaaa"));
        Ok(())
    }

    #[test]
    fn effective_or_invalid_image_cannot_become_neutral_intent() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for image in [
            observed_image(b"private.registry/app:1", Origin::Effective),
            observed_image(b"\xff", Origin::Configured),
        ] {
            let imported = adapter.import(&source(vec![container(image)])?);
            let application = imported.application().ok_or("missing application")?;
            assert!(application.services()[0].value().image().is_none());
            assert!(!format!("{imported:?}").contains("private.registry"));
            assert!(imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].image" && outcome.kind() != ConversionKind::Exact
            }));
        }
        Ok(())
    }

    #[test]
    fn explicit_empty_field_and_no_selected_resource_are_reported() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let mut observed = container(observed_image(b"app:1", Origin::Configured));
        observed.configured_ports = Observed::present(Vec::new(), Availability::Empty, Origin::Configured);
        let imported = adapter.import(&source(vec![observed])?);
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].configured_ports" && outcome.kind() == ConversionKind::Exact
        }));
        let empty = adapter.import(&source(Vec::new())?);
        assert!(empty.outcomes().iter().any(|outcome| {
            outcome.subject() == "application.selection" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(empty.diagnostics().iter().any(|diagnostic| {
            diagnostic.severity() == Severity::Warning && diagnostic.summary().contains("no inspected")
        }));
        let discovery = adapter.import(&source_with_resources(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            DockerSelection::Discovery,
        )?);
        assert!(
            discovery
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.summary().contains("discovery contains list metadata"))
        );
        Ok(())
    }

    #[test]
    fn standalone_network_and_volume_retain_uncertain_ownership_without_native_names() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let network_source = source_with_resources(Vec::new(), vec![network()], Vec::new(), DockerSelection::Explicit)?;
        let network_imported = adapter.import(&network_source);
        let network_app = network_imported.application().ok_or("missing network application")?;
        assert!(network_app.services().is_empty());
        assert_eq!(network_app.networks()[0].value().name().as_str(), "docker-network-1");
        assert_eq!(
            network_app.networks()[0].value().ownership(),
            ResourceOwnership::Uncertain
        );
        assert!(network_imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "networks[0].ownership" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{network_imported:?}").contains("private-network"));

        let volume_source = source_with_resources(Vec::new(), Vec::new(), vec![volume()], DockerSelection::Explicit)?;
        let volume_imported = adapter.import(&volume_source);
        let volume_app = volume_imported.application().ok_or("missing volume application")?;
        assert!(volume_app.services().is_empty());
        assert_eq!(volume_app.volumes()[0].value().name().as_str(), "docker-volume-1");
        assert_eq!(
            volume_app.volumes()[0].value().ownership(),
            ResourceOwnership::Uncertain
        );
        assert!(volume_imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "volumes[0].mountpoint" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{volume_imported:?}").contains("private-volume"));
        assert!(!format!("{volume_imported:?}").contains("/private/mountpoint"));
        Ok(())
    }

    #[test]
    fn decoded_network_membership_needs_policy_and_remains_approximate() -> Result<(), String> {
        const INSPECT: &str = r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{"Image":"app:1"},"HostConfig":{"NetworkMode":"bridge"},"NetworkSettings":{"Networks":{"private-network":{"NetworkID":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","IPAddress":"10.0.0.4","IPAMConfig":{"IPv4Address":"10.0.0.10"},"Aliases":["private-alias"]}}}}"#;
        const NETWORK: &str = r#"{"Id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","Name":"private-network","Driver":"bridge","Internal":true}"#;
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let conservative = adapter.import(&decoded_source_fixture(INSPECT, None, Some(NETWORK))?);
        assert!(
            conservative.application().ok_or("missing application")?.services()[0]
                .value()
                .networks()
                .is_empty()
        );
        assert!(conservative.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Unsupported
        }));
        for child in ["requested_ipv4_address", "aliases"] {
            assert!(conservative.outcomes().iter().any(|outcome| {
                outcome.subject() == format!("services[0].networks[0].{child}")
                    && outcome.kind() == ConversionKind::Unsupported
            }));
        }

        let source = decoded_source_fixture(INSPECT, None, Some(NETWORK))?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
        assert_eq!(source.inventory().networks[0].id.origin, Origin::RuntimeAssigned);
        let imported = adapter.import(&source);
        let application = imported.application().ok_or("missing application")?;
        let service = application.services()[0].value();
        assert_eq!(service.networks().len(), 1);
        assert_eq!(service.networks()[0].value().network().as_str(), "docker-network-1");
        assert_eq!(service.networks()[0].value().aliases(), ["private-alias"]);
        assert_eq!(service.networks()[0].value().alias_sensitivities(), [false]);
        assert!(
            service.networks()[0].value().alias_origins()[0]
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        );
        assert_eq!(
            application.networks()[0].value().ownership(),
            ResourceOwnership::Uncertain
        );
        let network = application.networks()[0].value();
        assert_eq!(
            network.runtime_name().map(|value| value.value().expose()),
            Some("private-network")
        );
        assert!(
            network
                .runtime_name()
                .is_some_and(|value| !value.value().is_sensitive())
        );
        assert_eq!(network.driver().map(|value| value.value().expose()), Some("bridge"));
        assert_eq!(network.internal().map(|value| *value.value()), Some(true));
        assert!(
            service.networks()[0]
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        );
        assert_reviewed_network_outcomes(&imported);
        Ok(())
    }

    fn assert_reviewed_network_outcomes(imported: &ImportResult) {
        for subject in ["services[0].networks[0]", "services[0].networks"] {
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Approximate })
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Exact })
            );
        }
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0]"
                && outcome.kind() == ConversionKind::Approximate
                && outcome
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
                && outcome
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        }));
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0].requested_ipv4_address"
                && outcome.kind() == ConversionKind::Unsupported
        }));
        for subject in [
            "services[0].networks[0].aliases[0]",
            "services[0].networks[0].aliases",
            "networks[0].runtime_name",
            "networks[0].driver",
            "networks[0].internal",
        ] {
            assert!(
                imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == subject
                        && outcome.kind() == ConversionKind::Approximate
                        && outcome
                            .origins()
                            .iter()
                            .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
                        && outcome
                            .origins()
                            .iter()
                            .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
                }),
                "subject: {subject}"
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Exact }),
                "subject: {subject}"
            );
        }
        assert!(
            imported
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.summary().contains("observation-only") })
        );
        // Promoted ordinary names may appear in the neutral model and private
        // native artifact. Diagnostics still must not echo observed values.
        let report = format!("{:?}", imported.diagnostics());
        assert!(!report.contains("private-network"));
        assert!(!report.contains("private-alias"));
        assert!(!report.contains("10.0.0.4"));
        assert!(!report.contains("10.0.0.10"));
    }

    #[test]
    fn network_promotion_switches_are_independent() -> Result<(), String> {
        let make_source = || -> Result<DockerSource, String> {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut attachment = network_attachment();
            attachment.aliases = Observed::present(
                vec![ProtectedValue::new(b"private-alias".to_vec())],
                Availability::Present,
                Origin::Effective,
            );
            observed.networks = Observed::present(vec![attachment], Availability::Present, Origin::RuntimeAssigned);
            let mut inspected = network();
            inspected.internal = Observed::present(true, Availability::Present, Origin::Effective);
            source_with_resources(vec![observed], vec![inspected], Vec::new(), DockerSelection::Explicit)
        };
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;

        let conservative = adapter.import(&make_source()?);
        let app = conservative.application().ok_or("missing conservative application")?;
        assert!(app.services()[0].value().networks().is_empty());
        assert!(app.networks()[0].value().runtime_name().is_none());
        assert!(app.networks()[0].value().driver().is_none());
        assert!(app.networks()[0].value().internal().is_none());
        assert!(conservative.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0].aliases" && outcome.kind() == ConversionKind::Unsupported
        }));

        let no_aliases =
            adapter.import(&make_source()?.with_promotion_policy(
                DockerPromotionPolicy::portable_application().with_observed_network_aliases(false),
            ));
        let app = no_aliases.application().ok_or("missing no-alias application")?;
        assert_eq!(app.services()[0].value().networks().len(), 1);
        assert!(app.services()[0].value().networks()[0].value().aliases().is_empty());
        assert_eq!(
            app.networks()[0].value().internal().map(|value| *value.value()),
            Some(true)
        );
        assert!(no_aliases.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0].aliases" && outcome.kind() == ConversionKind::Unsupported
        }));

        let no_bridge_settings = adapter.import(&make_source()?.with_promotion_policy(
            DockerPromotionPolicy::portable_application().with_inspected_bridge_network_settings(false),
        ));
        let app = no_bridge_settings
            .application()
            .ok_or("missing no-settings application")?;
        assert_eq!(
            app.services()[0].value().networks()[0].value().aliases(),
            ["private-alias"]
        );
        assert!(app.networks()[0].value().runtime_name().is_none());
        assert!(app.networks()[0].value().driver().is_none());
        assert!(app.networks()[0].value().internal().is_none());

        let no_membership = adapter.import(&make_source()?.with_promotion_policy(
            DockerPromotionPolicy::portable_application().with_observed_network_membership(false),
        ));
        let app = no_membership.application().ok_or("missing no-membership application")?;
        assert!(app.services()[0].value().networks().is_empty());
        assert!(app.networks()[0].value().runtime_name().is_some());
        assert!(no_membership.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0].aliases" && outcome.kind() == ConversionKind::Unsupported
        }));
        Ok(())
    }

    #[test]
    fn alias_child_failures_keep_safe_aliases_and_other_links() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        let mut first = network_attachment();
        first.aliases = Observed::present(
            vec![
                ProtectedValue::new(b"private-native-name".to_vec()),
                ProtectedValue::new(b"abcdef012345".to_vec()),
                ProtectedValue::new(b"safe-alias".to_vec()),
                ProtectedValue::new(b"safe-alias".to_vec()),
                ProtectedValue::new(vec![0xff]),
                ProtectedValue::new(Vec::new()),
                ProtectedValue::new(b"bad alias".to_vec()),
            ],
            Availability::Present,
            Origin::Effective,
        );
        let mut second = network_attachment();
        second.name = ProtectedValue::new(b"second-network".to_vec());
        second.network_id = observed_image(b"second-network-id", Origin::RuntimeAssigned);
        second.aliases = Observed::unavailable(Availability::Redacted, Origin::Effective);
        observed.networks = Observed::present(vec![first, second], Availability::Present, Origin::RuntimeAssigned);
        let mut second_network = network();
        second_network.id = observed_image(b"second-network-id", Origin::RuntimeAssigned);
        second_network.name = observed_image(b"second-network", Origin::Effective);
        let source = source_with_resources(
            vec![observed],
            vec![network(), second_network],
            Vec::new(),
            DockerSelection::Explicit,
        )?
        .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let links = imported.application().ok_or("missing application")?.services()[0]
            .value()
            .networks();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].value().aliases(), ["safe-alias"]);
        assert_eq!(links[0].value().alias_sensitivities(), [false]);
        assert!(links[1].value().aliases().is_empty());
        for index in [0, 1, 3, 4, 5, 6] {
            assert!(
                imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == format!("services[0].networks[0].aliases[{index}]")
                        && outcome.kind() == ConversionKind::Unsupported
                }),
                "alias index: {index}"
            );
        }
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[1].aliases" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[0].aliases[2]" && outcome.kind() == ConversionKind::Approximate
        }));
        let diagnostics = format!("{:?}", imported.diagnostics());
        assert!(!diagnostics.contains("safe-alias"));
        assert!(!diagnostics.contains("private-native-name"));
        Ok(())
    }

    #[test]
    fn unreviewed_bridge_settings_remain_losses_without_losing_other_service_fields() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for case in [
            "overlay",
            "missing-id",
            "duplicate-id",
            "invalid-name",
            "redacted-internal",
        ] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            observed.networks = Observed::present(
                vec![network_attachment()],
                Availability::Present,
                Origin::RuntimeAssigned,
            );
            let mut inspected = network();
            inspected.internal = Observed::present(true, Availability::Present, Origin::Effective);
            match case {
                "overlay" => inspected.driver = observed_image(b"overlay", Origin::Effective),
                "missing-id" => inspected.id = absent(),
                "invalid-name" => inspected.name = observed_image(&[0xff], Origin::Effective),
                "redacted-internal" => {
                    inspected.internal = Observed::unavailable(Availability::Redacted, Origin::Effective);
                }
                "duplicate-id" => {}
                _ => unreachable!("closed bridge fixture"),
            }
            let networks = if case == "duplicate-id" {
                let mut duplicate = network();
                duplicate.name = observed_image(b"second-network", Origin::Effective);
                vec![inspected, duplicate]
            } else {
                vec![inspected]
            };
            let source = source_with_resources(vec![observed], networks, Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let app = imported.application().ok_or("missing application")?;
            let mapped = app.networks()[0].value();
            match case {
                "overlay" | "missing-id" | "duplicate-id" => {
                    assert!(mapped.driver().is_none(), "case: {case}");
                    assert!(mapped.runtime_name().is_none(), "case: {case}");
                    assert!(mapped.internal().is_none(), "case: {case}");
                }
                "invalid-name" => {
                    assert!(mapped.runtime_name().is_none());
                    assert!(mapped.internal().is_some());
                }
                "redacted-internal" => {
                    assert!(mapped.runtime_name().is_some());
                    assert!(mapped.internal().is_none());
                }
                _ => unreachable!("closed bridge fixture"),
            }
            let child = match case {
                "overlay" => "networks[0].driver",
                "missing-id" | "duplicate-id" => "networks[0].settings",
                "invalid-name" => "networks[0].runtime_name",
                "redacted-internal" => "networks[0].internal",
                _ => unreachable!("closed bridge fixture"),
            };
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| { outcome.subject() == child && outcome.kind() == ConversionKind::Unsupported }),
                "case: {case}"
            );
            assert!(app.services()[0].value().image().is_some(), "case: {case}");
        }
        Ok(())
    }

    #[test]
    fn shared_network_and_volume_links_do_not_establish_external_ownership() -> Result<(), String> {
        let containers = (0..2)
            .map(|index| {
                let mut observed = container(observed_image(b"app:1", Origin::Effective));
                observed.reference = ResourceRef::new(index + 1);
                observed.networks = Observed::present(
                    vec![network_attachment()],
                    Availability::Present,
                    Origin::RuntimeAssigned,
                );
                observed.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
                observed
            })
            .collect();
        let source = source_with_resources(containers, vec![network()], vec![volume()], DockerSelection::Explicit)?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let application = imported.application().ok_or("missing application")?;
        assert_eq!(
            application.networks()[0].value().ownership(),
            ResourceOwnership::Uncertain
        );
        assert_eq!(
            application.volumes()[0].value().ownership(),
            ResourceOwnership::Uncertain
        );
        for service in application.services() {
            assert_eq!(
                service.value().networks()[0].value().network().as_str(),
                "docker-network-1"
            );
            assert!(
                matches!(service.value().mounts()[0].value().source(), MountSource::Volume(name) if name.as_str() == "docker-volume-1")
            );
        }
        Ok(())
    }

    #[test]
    fn explicit_resource_decisions_remap_only_selected_source_keys() -> Result<(), String> {
        let network_id = "b".repeat(64);
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (action, ownership) in [
            ("create", ResourceOwnership::Application),
            ("external", ResourceOwnership::External),
        ] {
            let specs = [
                format!("network:{network_id}={action}:destination-network"),
                format!("volume:private-volume={action}:destination-volume"),
            ];
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            observed.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
            let source = source_with_resources(
                vec![observed],
                vec![network()],
                vec![volume()],
                DockerSelection::Explicit,
            )?
            .with_promotion_policy(DockerPromotionPolicy::portable_application())
            .with_resource_decisions(resource_decisions(&specs)?);
            let imported = adapter.import(&source);
            let app = imported.application().ok_or("resource decisions were not imported")?;
            let network = &app.networks()[0];
            let volume = &app.volumes()[0];
            assert_eq!(network.value().ownership(), ownership);
            assert_eq!(volume.value().ownership(), ownership);
            assert_eq!(
                network
                    .value()
                    .runtime_name()
                    .ok_or("target network name missing")?
                    .value()
                    .expose(),
                "destination-network"
            );
            assert_eq!(
                volume
                    .value()
                    .runtime_name()
                    .ok_or("target volume name missing")?
                    .value()
                    .expose(),
                "destination-volume"
            );
            assert!(
                network
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::UserOverride)
            );
            assert!(
                volume
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
            );
            assert!(matches!(app.services()[0].value().mounts()[0].value().source(),
                MountSource::Volume(name) if name.as_str() == "docker-volume-1"));
            let diagnostic_text = format!("{:?}", imported.diagnostics());
            assert!(!diagnostic_text.contains("private-volume") && !diagnostic_text.contains("destination-volume"));
            assert!(!diagnostic_text.contains("destination-network") && !diagnostic_text.contains(&network_id));
        }
        Ok(())
    }

    #[test]
    fn selected_named_volume_decision_flows_through_import_and_native_export() -> Result<(), String> {
        let profile = TargetCapabilityCatalog::reviewed()
            .profiles()
            .find(|profile| {
                matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed Docker profile missing")?
            .clone();
        let version = PlatformVersion::new(29, 8, 1);
        let target = TargetProfile::new("docker", version, Some(version)).map_err(|error| format!("{error:?}"))?;
        for (action, create_count, prerequisite_count) in [("create", 1, 0), ("external", 0, 1)] {
            let mut observed = container(observed_image(b"busybox:1", Origin::Effective));
            observed.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
            let source = source_with_resources(vec![observed], Vec::new(), vec![volume()], DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application())
                .with_resource_decisions(resource_decisions(&[format!(
                    "volume:private-volume={action}:destination-volume"
                )])?);
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            let app = imported.application().ok_or("selected volume import failed")?;
            let planned = DockerExporter::new(profile.clone())
                .map_err(|error| format!("{error:?}"))?
                .plan(app, &target)
                .map_err(|error| format!("{error:?}"))?;
            let output = planned
                .candidate()
                .ok_or("explicit volume destination did not produce native plan")?;
            let complete = String::from_utf8(output.complete_bytes().map_err(|error| format!("{error:?}"))?)
                .map_err(|error| error.to_string())?;
            if complete.matches("/volumes/create").count() != create_count
                || complete.contains("/networks/create")
                || output.volume_prerequisites().len() != prerequisite_count
                || (action == "create" && !complete.contains("\"Name\":\"destination-volume\""))
                || (action == "external" && !complete.contains("\"identity\":\"destination-volume\""))
                || !complete.contains("\"Source\":\"destination-volume\"")
                || complete.contains("private-volume")
                || complete.contains("docker-volume-1")
            {
                return Err(format!("wrong native volume operation for {action}"));
            }
        }
        Ok(())
    }

    #[test]
    fn selected_network_decision_flows_through_import_and_native_export() -> Result<(), String> {
        let profile = TargetCapabilityCatalog::reviewed()
            .profiles()
            .find(|profile| {
                matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed Docker profile missing")?
            .clone();
        let version = PlatformVersion::new(29, 8, 1);
        let target = TargetProfile::new("docker", version, Some(version)).map_err(|error| format!("{error:?}"))?;
        let source_id = "b".repeat(64);
        for (action, create_count, prerequisite_count) in [("create", 1, 0), ("external", 0, 1)] {
            let mut observed = container(observed_image(b"busybox:1", Origin::Effective));
            observed.networks = Observed::present(
                vec![network_attachment()],
                Availability::Present,
                Origin::RuntimeAssigned,
            );
            let source = source_with_resources(vec![observed], vec![network()], Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application())
                .with_resource_decisions(resource_decisions(&[format!(
                    "network:{source_id}={action}:destination-network"
                )])?);
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            let app = imported.application().ok_or("selected network import failed")?;
            assert_eq!(
                app.services()[0].value().networks()[0].value().network().as_str(),
                "docker-network-1"
            );
            let planned = DockerExporter::new(profile.clone())
                .map_err(|error| format!("{error:?}"))?
                .plan(app, &target)
                .map_err(|error| format!("{error:?}"))?;
            let output = planned
                .candidate()
                .ok_or("explicit network destination did not produce native plan")?;
            let complete = String::from_utf8(output.complete_bytes().map_err(|error| format!("{error:?}"))?)
                .map_err(|error| error.to_string())?;
            if complete.matches("/networks/create").count() != create_count
                || complete.contains("/volumes/create")
                || output.network_prerequisites().len() != prerequisite_count
                || !complete.contains("\"destination-network\"")
                || !complete.contains("\"EndpointsConfig\":{\"destination-network\":")
                || (action == "create" && !complete.contains("\"Name\":\"destination-network\""))
                || (action == "external" && !complete.contains("\"identity\":\"destination-network\""))
                || complete.contains("private-network")
                || complete.contains("docker-network-1")
                || complete.contains(&source_id)
            {
                return Err(format!("wrong native network operation for {action}"));
            }
        }
        Ok(())
    }

    #[test]
    fn resource_decisions_reject_unselected_ambiguous_builtin_and_shared_sources() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let id = "b".repeat(64);
        let spec = [format!("network:{id}=create:destination")];
        let test = |networks: Vec<NetworkObservation>, volumes: Vec<VolumeObservation>, specs: &[String]| {
            let source = source_with_resources(Vec::new(), networks, volumes, DockerSelection::Explicit)?
                .with_resource_decisions(resource_decisions(specs)?);
            let imported = adapter.import(&source);
            if imported.application().is_some() {
                return Err("invalid resource decision was accepted".to_owned());
            }
            let diagnostics = format!("{:?}", imported.diagnostics());
            if diagnostics.contains("destination")
                || diagnostics.contains("private-volume")
                || diagnostics.contains(&id)
            {
                return Err("resource decision leaked native identity".to_owned());
            }
            Ok::<(), String>(())
        };
        test(Vec::new(), Vec::new(), &spec)?;
        test(vec![network(), network()], Vec::new(), &spec)?;
        for case in ["missing-id", "malformed-id", "wrong-origin"] {
            let mut inspected = network();
            match case {
                "missing-id" => inspected.id = absent(),
                "malformed-id" => inspected.id = observed_image(b"short-id", Origin::RuntimeAssigned),
                "wrong-origin" => inspected.id.origin = Origin::Effective,
                _ => unreachable!("closed source-key fixture"),
            }
            test(vec![inspected], Vec::new(), &spec)?;
        }
        let mut built_in = network();
        built_in.name = observed_image(b"bridge", Origin::Effective);
        test(vec![built_in], Vec::new(), &spec)?;
        let mut empty_name = network();
        empty_name.name = observed_image(b"", Origin::Effective);
        test(vec![empty_name], Vec::new(), &spec)?;
        let mut invalid_name = network();
        invalid_name.name = observed_image(b"invalid network name", Origin::Effective);
        test(vec![invalid_name], Vec::new(), &spec)?;
        let mut shared = network();
        shared.active_endpoints = Observed::present(
            vec![docker_lens::decoder::NetworkActiveEndpoint {
                container_id: observed_image(
                    b"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    Origin::RuntimeAssigned,
                ),
                endpoint: absent(),
            }],
            Availability::Present,
            Origin::RuntimeAssigned,
        );
        test(vec![shared], Vec::new(), &spec)?;
        let mut inconsistent_empty = network();
        inconsistent_empty.active_endpoints = Observed::present(
            vec![docker_lens::decoder::NetworkActiveEndpoint {
                container_id: observed_image(
                    b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    Origin::RuntimeAssigned,
                ),
                endpoint: absent(),
            }],
            Availability::Empty,
            Origin::RuntimeAssigned,
        );
        test(vec![inconsistent_empty], Vec::new(), &spec)?;
        let mut unobserved_membership = network();
        unobserved_membership.active_endpoints = absent();
        test(vec![unobserved_membership], Vec::new(), &spec)?;
        test(
            Vec::new(),
            vec![volume()],
            &["volume:other=external:destination".to_owned()],
        )?;
        test(
            Vec::new(),
            vec![volume(), volume()],
            &["volume:private-volume=create:destination".to_owned()],
        )?;
        Ok(())
    }

    #[test]
    fn network_create_accepts_only_exact_selected_active_container_ids() -> Result<(), String> {
        let selected_id = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut inspected = network();
        inspected.active_endpoints = Observed::present(
            vec![docker_lens::decoder::NetworkActiveEndpoint {
                container_id: observed_image(selected_id, Origin::RuntimeAssigned),
                endpoint: absent(),
            }],
            Availability::Present,
            Origin::RuntimeAssigned,
        );
        let decision = [format!("network:{}=create:destination-network", "b".repeat(64))];
        let source = source_with_resources(
            vec![container(observed_image(b"app:1", Origin::Effective))],
            vec![inspected],
            Vec::new(),
            DockerSelection::Explicit,
        )?
        .with_resource_decisions(resource_decisions(&decision)?);
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        assert_eq!(
            imported
                .application()
                .ok_or("selected member should be allowed")?
                .networks()[0]
                .value()
                .ownership(),
            ResourceOwnership::Application
        );
        let mut invalid_member = container(observed_image(b"app:1", Origin::Effective));
        invalid_member.id = observed_image(b"not-a-full-container-id", Origin::RuntimeAssigned);
        let mut inspected = network();
        inspected.active_endpoints = Observed::present(
            vec![docker_lens::decoder::NetworkActiveEndpoint {
                container_id: observed_image(selected_id, Origin::RuntimeAssigned),
                endpoint: absent(),
            }],
            Availability::Present,
            Origin::RuntimeAssigned,
        );
        let source = source_with_resources(
            vec![invalid_member],
            vec![inspected],
            Vec::new(),
            DockerSelection::Explicit,
        )?
        .with_resource_decisions(resource_decisions(&decision)?);
        let rejected = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        assert!(rejected.application().is_none());
        for case in ["wrong-origin", "duplicate-active-member"] {
            let mut inspected = network();
            inspected.active_endpoints = Observed::present(
                vec![docker_lens::decoder::NetworkActiveEndpoint {
                    container_id: observed_image(selected_id, Origin::RuntimeAssigned),
                    endpoint: absent(),
                }],
                Availability::Present,
                Origin::RuntimeAssigned,
            );
            match case {
                "wrong-origin" => {
                    inspected.active_endpoints.origin = Origin::Effective;
                }
                "duplicate-active-member" => {
                    inspected.active_endpoints = Observed::present(
                        vec![
                            docker_lens::decoder::NetworkActiveEndpoint {
                                container_id: observed_image(selected_id, Origin::RuntimeAssigned),
                                endpoint: absent(),
                            },
                            docker_lens::decoder::NetworkActiveEndpoint {
                                container_id: observed_image(selected_id, Origin::RuntimeAssigned),
                                endpoint: absent(),
                            },
                        ],
                        Availability::Present,
                        Origin::RuntimeAssigned,
                    );
                }
                _ => unreachable!("closed active-member fixture"),
            }
            let source = source_with_resources(
                vec![container(observed_image(b"app:1", Origin::Effective))],
                vec![inspected],
                Vec::new(),
                DockerSelection::Explicit,
            )?
            .with_resource_decisions(resource_decisions(&decision)?);
            let rejected = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            assert!(
                rejected.application().is_none(),
                "invalid active membership was accepted: {case}"
            );
        }
        Ok(())
    }

    #[test]
    fn observed_empty_network_member_map_allows_new_destination_intent_without_ownership_claim() -> Result<(), String> {
        let inspected = network();
        assert_eq!(inspected.active_endpoints.availability, Availability::Empty);
        let decision = [format!("network:{}=create:new-destination", "b".repeat(64))];
        let source = source_with_resources(
            vec![container(observed_image(b"app:1", Origin::Effective))],
            vec![inspected],
            Vec::new(),
            DockerSelection::Explicit,
        )?
        .with_resource_decisions(resource_decisions(&decision)?);
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let app = imported
            .application()
            .ok_or("explicit new-destination decision was rejected")?;
        assert_eq!(app.networks()[0].value().ownership(), ResourceOwnership::Application);
        assert_eq!(
            app.networks()[0]
                .value()
                .runtime_name()
                .ok_or("destination missing")?
                .value()
                .expose(),
            "new-destination"
        );
        assert!(
            app.networks()[0]
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::UserOverride)
        );
        Ok(())
    }

    fn assert_external_constraint_losses_survive_every_policy(
        imported: &ImportResult,
        exporter: &DockerExporter,
        target: &TargetProfile,
    ) -> Result<(), String> {
        let network = &imported
            .application()
            .ok_or("external resource application missing")?
            .networks()[0];
        for policy in [
            LossPolicy::ExactOnly,
            LossPolicy::AllowApproximate,
            LossPolicy::AllowPartial,
        ] {
            let result =
                convert_imported(imported.clone(), exporter, target, policy).map_err(|error| format!("{error:?}"))?;
            assert_eq!(result.is_blocked(), policy != LossPolicy::AllowPartial);
            for subject in ["networks[0].enable_ipv6", "networks[0].options", "volumes[0].driver"] {
                assert!(
                    result.outcomes().iter().any(|outcome| {
                        outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported
                    }),
                    "import loss must survive conversion"
                );
            }
            assert_external_isolation_retained(result.outcomes(), result.diagnostics(), network)?;
            assert!(
                result
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "networks[0].internal"
                        && outcome.kind() == ConversionKind::Approximate)
            );
            assert!(
                !result
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "networks[0].internal"
                        && outcome.kind() == ConversionKind::Exact)
            );
            if policy == LossPolicy::AllowPartial {
                let output = result
                    .output()
                    .ok_or("partial review artifact missing despite admitted target intent")?;
                assert_admitted_external_output(output, network)?;
                assert_eq!(output.volume_prerequisites().len(), 1);
                if !imported
                    .application()
                    .ok_or("external resource application missing")?
                    .services()
                    .is_empty()
                {
                    assert_dependent_external_network_membership(output)?;
                }
            } else {
                assert!(result.output().is_none());
            }
        }
        Ok(())
    }

    fn assert_dependent_service_retains_external_constraint_and_source_policy_losses(
        imported: ImportResult,
        exporter: &DockerExporter,
        target: &TargetProfile,
    ) -> Result<(), String> {
        let (application, outcomes, diagnostics) = imported.into_parts();
        let mut application = application.ok_or("external resource application missing")?;
        let mut dependent =
            boxferry_model::Service::new(Identifier::new("dependent").map_err(|error| error.to_string())?);
        dependent.set_image(Sourced::generated(
            boxferry_model::ImageReference::parse("app:1").map_err(|error| error.to_string())?,
        ));
        dependent.add_network(Sourced::generated(boxferry_model::NetworkAttachment::new(
            application.networks()[0].value().name().clone(),
            Vec::new(),
        )));
        application
            .add_service(Sourced::generated(dependent))
            .map_err(|error| error.to_string())?;
        let attached = ImportResult::new(Some(application), outcomes, diagnostics);
        let plan = exporter
            .plan(attached.application().ok_or("attached application missing")?, target)
            .map_err(|error| format!("{error:?}"))?;
        let output = plan
            .candidate()
            .ok_or("admitted dependent service target candidate missing")?;
        assert_admitted_external_output(
            output,
            &attached.application().ok_or("attached application missing")?.networks()[0],
        )?;
        assert_dependent_external_network_membership(output)?;
        assert!(!plan.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.dependent.networks.0" && outcome.kind() == ConversionKind::Invalid
        }));
        assert_external_constraint_losses_survive_every_policy(&attached, exporter, target)
    }

    fn assert_dependent_external_network_membership(output: &crate::DockerOutput) -> Result<(), String> {
        let [network] = output.network_prerequisites() else {
            return Err("dependent external prerequisite inventory differs".into());
        };
        // Independently authored destination; do not derive the expected membership from rendered requests.
        assert_eq!(network.identity(), b"destination-network");
        assert_eq!(network.expected_internal, Some(false));
        let catalog = TargetCapabilityCatalog::reviewed();
        let expected_profile = catalog
            .profiles()
            .find(|profile| {
                matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed upstream/rootful Docker profile missing")?;
        assert_eq!(expected_profile.release().as_str(), "29.8.1");
        assert_eq!(output.native_profile(), expected_profile);
        assert_eq!(output.evidence().record_key(), expected_profile.evidence_key());
        // Fixture input selects upstream 29.8.1/rootful; its reviewed rendering API is independently fixed.
        let request = "{\"method\":\"POST\",\"path\":\"/v1.56/containers/create?name=dependent\",\"body\":{\"Image\":\"app:1\",\"HostConfig\":{\"NetworkMode\":\"destination-network\"},\"NetworkingConfig\":{\"EndpointsConfig\":{\"destination-network\":{}}}}}";
        // Exact bytes prove one retained dependent request, one exact attachment and no resource creation.
        assert_eq!(output.bytes(), format!("{request}\n").as_bytes());
        let complete = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
        let text = std::str::from_utf8(&complete).map_err(|error| error.to_string())?;
        assert!(text.contains(&format!("\"requests\":[{request}],\"prerequisites\":[")));
        Ok(())
    }

    fn assert_external_isolation_retained(
        outcomes: &[boxferry_engine::ConversionOutcome],
        diagnostics: &[boxferry_engine::Diagnostic],
        network: &Sourced<boxferry_model::Network>,
    ) -> Result<(), String> {
        let parent = format!("networks.{}", network.value().name().as_str());
        let subject = format!("{parent}.internal");
        let decisions = outcomes
            .iter()
            .filter(|outcome| outcome.subject() == subject)
            .collect::<Vec<_>>();
        let [decision] = decisions.as_slice() else {
            return Err("retained external isolation decision must occur once".into());
        };
        assert_eq!(decision.kind(), ConversionKind::Exact);
        assert!(decision.diagnostic().is_none());
        assert!(
            network
                .origins()
                .iter()
                .all(|origin| decision.origins().contains(origin))
        );
        let internal = network
            .value()
            .internal()
            .ok_or("explicit internal constraint missing")?;
        assert_eq!(network.value().internal().map(|field| *field.value()), Some(false));
        assert!(
            internal
                .origins()
                .iter()
                .all(|origin| decision.origins().contains(origin))
        );
        let prerequisite = format!("{parent}.prerequisite");
        assert!(
            outcomes
                .iter()
                .any(|outcome| outcome.subject() == prerequisite && outcome.kind() == ConversionKind::Exact)
        );
        assert!(!diagnostics.iter().any(|diagnostic| {
            diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "subject" && field.value().redacted() == subject)
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "native_category" && field.value().redacted() == "missing_capability")
        }));
        assert!(!format!("{diagnostics:?}").contains("private-network"));
        Ok(())
    }

    fn assert_admitted_external_output(
        output: &crate::DockerOutput,
        network: &Sourced<boxferry_model::Network>,
    ) -> Result<(), String> {
        let [prerequisite] = output.network_prerequisites() else {
            return Err("admitted external prerequisite inventory changed".into());
        };
        assert_eq!(
            prerequisite.expected_internal,
            network.value().internal().map(|field| *field.value())
        );
        assert_eq!(prerequisite.expected_driver, docker_lens::target::NetworkDriver::Bridge);
        assert_eq!(prerequisite.identity(), b"destination-network");
        let bytes = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
        let complete = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        let internal = network.value().internal().map(|field| *field.value());
        let schema = if internal.is_some() { 3 } else { 1 };
        assert!(complete.starts_with(&format!(
            "{{\"schema_version\":{schema},\"context\":{{\"kind\":\"target\""
        )));
        if let Some(value) = internal {
            assert!(complete.contains(&format!("\"expected_internal\":{value}")));
        } else {
            assert!(!complete.contains("\"expected_internal\""));
        }
        assert!(!complete.contains("private-network"));
        Ok(())
    }

    #[test]
    fn external_resource_decision_keeps_effective_constraints_or_reports_them_unsupported() -> Result<(), String> {
        let mut inspected = network();
        inspected.internal = Observed::present(false, Availability::Present, Origin::Effective);
        inspected.enable_ipv6 = Observed::present(true, Availability::Present, Origin::Effective);
        inspected.options = Observed::present(
            vec![docker_lens::decoder::MapEntryObservation {
                key: ProtectedValue::new(b"com.docker.network.driver.mtu".to_vec()),
                value: observed_image(b"1500", Origin::Effective),
            }],
            Availability::Present,
            Origin::Effective,
        );
        let source = source_with_resources(Vec::new(), vec![inspected], vec![volume()], DockerSelection::Explicit)?
            .with_promotion_policy(DockerPromotionPolicy::portable_application())
            .with_resource_decisions(resource_decisions(&[
                format!("network:{}=external:destination-network", "b".repeat(64)),
                "volume:private-volume=external:destination-volume".to_owned(),
            ])?);
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let app = imported
            .application()
            .ok_or("explicit external resource import failed")?;
        assert_eq!(
            app.networks()[0].value().internal().map(|field| *field.value()),
            Some(false)
        );
        assert_eq!(app.networks()[0].value().ownership(), ResourceOwnership::External);
        let network = &app.networks()[0];
        assert!(network.value().ipv6().is_none() && network.value().driver_options().is_none());
        assert!(
            network
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::UserOverride)
        );
        let internal = network
            .value()
            .internal()
            .ok_or("explicit internal constraint missing")?;
        assert!(
            internal
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
        );
        assert!(
            internal
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
        );
        for subject in ["networks[0].enable_ipv6", "networks[0].options", "volumes[0].driver"] {
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported }),
                "missing explicit unsupported constraint: {subject}"
            );
        }
        let profile = TargetCapabilityCatalog::reviewed()
            .profiles()
            .find(|profile| {
                matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed Docker profile missing")?
            .clone();
        let version = PlatformVersion::new(29, 8, 1);
        let target = TargetProfile::new("docker", version, Some(version)).map_err(|error| format!("{error:?}"))?;
        let exporter = DockerExporter::new(profile).map_err(|error| format!("{error:?}"))?;
        let planned = exporter.plan(app, &target).map_err(|error| format!("{error:?}"))?;
        let output = planned
            .candidate()
            .ok_or("admitted external network/volume target candidate missing")?;
        assert_admitted_external_output(output, network)?;
        assert!(
            output.bytes().is_empty(),
            "external resources must not become native create requests"
        );
        assert_eq!(output.volume_prerequisites().len(), 1);
        assert_external_isolation_retained(planned.outcomes(), planned.diagnostics(), network)?;
        assert_eq!(
            app.networks()[0].value().internal().map(|field| *field.value()),
            Some(false)
        );
        assert_external_constraint_losses_survive_every_policy(&imported, &exporter, &target)?;
        assert_dependent_service_retains_external_constraint_and_source_policy_losses(imported, &exporter, &target)?;
        Ok(())
    }

    #[test]
    fn authorized_external_internal_observation_retains_false_true_absent_on_admitted_profiles() -> Result<(), String> {
        for internal in [None, Some(false), Some(true)] {
            let mut inspected = network();
            if let Some(value) = internal {
                inspected.internal = Observed::present(value, Availability::Present, Origin::Effective);
            }
            let source = source_with_resources(Vec::new(), vec![inspected], Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application())
                .with_resource_decisions(resource_decisions(&[format!(
                    "network:{}=external:destination-network",
                    "b".repeat(64)
                )])?);
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            let app = imported.application().ok_or("authorized external network missing")?;
            let network = &app.networks()[0];
            assert_authorized_external_internal_source(network, internal, imported.outcomes())?;
            for profile in TargetCapabilityCatalog::reviewed().profiles() {
                let version = match profile.identity().build() {
                    EngineBuild::Upstream => PlatformVersion::new(29, 8, 1),
                    EngineBuild::DebianPackage(_) => PlatformVersion::new(20, 10, 5),
                };
                let target =
                    TargetProfile::new("docker", version, Some(version)).map_err(|error| format!("{error:?}"))?;
                let exporter = DockerExporter::new(profile.clone()).map_err(|error| format!("{error:?}"))?;
                let plan = exporter.plan(app, &target).map_err(|error| format!("{error:?}"))?;
                let output = plan
                    .candidate()
                    .ok_or("admitted authorized external prerequisite missing")?;
                assert_eq!(output.native_profile(), profile);
                assert_eq!(output.evidence().record_key(), profile.evidence_key());
                assert_admitted_external_output(output, network)?;
                if internal.is_some() {
                    let subject = format!("networks.{}.internal", network.value().name().as_str());
                    let decision = plan
                        .outcomes()
                        .iter()
                        .find(|outcome| outcome.subject() == subject)
                        .ok_or("retained external internal decision missing")?;
                    assert_eq!(decision.kind(), ConversionKind::Exact);
                    assert!(decision.diagnostic().is_none());
                    assert!(
                        network
                            .origins()
                            .iter()
                            .all(|origin| decision.origins().contains(origin))
                    );
                    assert!(
                        network
                            .value()
                            .internal()
                            .ok_or("retained internal missing")?
                            .origins()
                            .iter()
                            .all(|origin| decision.origins().contains(origin))
                    );
                } else {
                    assert_eq!(output.network_prerequisites()[0].expected_internal, None);
                    assert!(
                        !plan.outcomes().iter().any(|outcome| outcome.subject()
                            == format!("networks.{}.internal", network.value().name().as_str()))
                    );
                }
                for policy in [
                    LossPolicy::ExactOnly,
                    LossPolicy::AllowApproximate,
                    LossPolicy::AllowPartial,
                ] {
                    let result = convert_imported(imported.clone(), &exporter, &target, policy)
                        .map_err(|error| format!("{error:?}"))?;
                    assert_eq!(result.is_blocked(), policy == LossPolicy::ExactOnly);
                    if policy == LossPolicy::ExactOnly {
                        assert!(result.output().is_none());
                    } else {
                        assert_admitted_external_output(
                            result
                                .output()
                                .ok_or("approved approximate external artifact missing")?,
                            network,
                        )?;
                    }
                }
                assert!(!format!("{:?}", plan.diagnostics()).contains("private-network"));
            }
        }
        Ok(())
    }

    fn assert_authorized_external_internal_source(
        network: &Sourced<boxferry_model::Network>,
        expected: Option<bool>,
        outcomes: &[boxferry_engine::ConversionOutcome],
    ) -> Result<(), String> {
        assert_eq!(network.value().internal().map(|field| *field.value()), expected);
        assert_eq!(network.value().ownership(), ResourceOwnership::External);
        assert!(
            network
                .origins()
                .iter()
                .any(|origin| origin.kind() == ProvenanceKind::UserOverride)
        );
        if expected.is_some() {
            assert!(
                outcomes
                    .iter()
                    .any(|outcome| outcome.subject() == "networks[0].internal"
                        && outcome.kind() == ConversionKind::Approximate)
            );
            assert!(
                !outcomes
                    .iter()
                    .any(|outcome| outcome.subject() == "networks[0].internal"
                        && outcome.kind() == ConversionKind::Exact)
            );
            let field = network.value().internal().ok_or("authorized internal field missing")?;
            assert!(
                field
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
            );
            assert!(
                field
                    .origins()
                    .iter()
                    .any(|origin| origin.kind() == ProvenanceKind::ConversionDecision)
            );
        }
        Ok(())
    }

    #[test]
    fn missing_conflicting_or_ambiguous_network_identity_never_links() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for case in [
            "missing-attachment",
            "redacted-attachment",
            "malformed-attachment",
            "empty-attachment",
            "wrong-origin",
            "missing-inspection",
            "redacted-inspection",
            "malformed-inspection",
            "wrong-inspection-origin",
            "wrong-id",
            "wrong-name",
            "missing-name",
            "malformed-name",
            "duplicate-id",
            "duplicate-attachment",
        ] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut attachment = network_attachment();
            let mut inspected = network();
            match case {
                "missing-attachment" => attachment.network_id = absent(),
                "redacted-attachment" => {
                    attachment.network_id = Observed::unavailable(Availability::Redacted, Origin::RuntimeAssigned);
                }
                "malformed-attachment" => attachment.network_id = observed_image(b"\xff", Origin::RuntimeAssigned),
                "empty-attachment" => attachment.network_id = observed_image(b"", Origin::RuntimeAssigned),
                "wrong-origin" => attachment.network_id.origin = Origin::Configured,
                "missing-inspection" => inspected.id = absent(),
                "redacted-inspection" => {
                    inspected.id = Observed::unavailable(Availability::Redacted, Origin::RuntimeAssigned);
                }
                "malformed-inspection" => inspected.id = observed_image(b"\xff", Origin::RuntimeAssigned),
                "wrong-inspection-origin" => inspected.id.origin = Origin::Configured,
                "wrong-id" => inspected.id = observed_image(b"different-id", Origin::RuntimeAssigned),
                "wrong-name" => inspected.name = observed_image(b"different-name", Origin::Effective),
                "missing-name" => inspected.name = absent(),
                "malformed-name" => {
                    attachment.name = ProtectedValue::new(vec![0xff]);
                    inspected.name = observed_image(b"\xff", Origin::Effective);
                }
                _ => {}
            }
            let attachments = if case == "duplicate-attachment" {
                vec![attachment, network_attachment()]
            } else {
                vec![attachment]
            };
            observed.networks = Observed::present(attachments, Availability::Present, Origin::RuntimeAssigned);
            let networks = if case == "duplicate-id" {
                vec![inspected, network()]
            } else {
                vec![inspected]
            };
            let source = source_with_resources(vec![observed], networks, Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.networks().is_empty(), "case: {case}");
            assert!(service.image().is_some(), "case: {case}");
            assert!(
                imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Unsupported
                }),
                "case: {case}"
            );
            assert!(
                !format!("{:?}", imported.diagnostics()).contains("private-network"),
                "case: {case}"
            );
        }
        Ok(())
    }

    #[test]
    fn network_membership_switch_and_partial_set_preserve_safe_fields() -> Result<(), String> {
        let make_source = || -> Result<DockerSource, String> {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut unmatched = network_attachment();
            unmatched.network_id = observed_image(b"unmatched-id", Origin::RuntimeAssigned);
            unmatched.aliases = Observed::present(
                vec![ProtectedValue::new(b"unmatched-private-alias".to_vec())],
                Availability::Present,
                Origin::Effective,
            );
            observed.networks = Observed::present(
                vec![network_attachment(), unmatched],
                Availability::Present,
                Origin::RuntimeAssigned,
            );
            source_with_resources(vec![observed], vec![network()], Vec::new(), DockerSelection::Explicit)
        };
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let disabled = adapter.import(&make_source()?.with_promotion_policy(
            DockerPromotionPolicy::portable_application().with_observed_network_membership(false),
        ));
        assert!(
            disabled.application().ok_or("missing application")?.services()[0]
                .value()
                .networks()
                .is_empty()
        );

        let imported =
            adapter.import(&make_source()?.with_promotion_policy(DockerPromotionPolicy::portable_application()));
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert_eq!(service.networks().len(), 1);
        assert!(service.image().is_some());
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[1]" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks[1].aliases" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{imported:?}").contains("unmatched-private-alias"));
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Approximate
        }));
        Ok(())
    }

    #[test]
    fn incompatible_mode_reports_unmapped_network_children_before_refusing_link() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for mode in ["host", "malformed", "redacted"] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut attachment = network_attachment();
            attachment.requested_ipv4_address = observed_image(b"10.0.0.10", Origin::Effective);
            attachment.aliases = Observed::present(
                vec![ProtectedValue::new(b"private-alias".to_vec())],
                Availability::Present,
                Origin::Effective,
            );
            observed.networks = Observed::present(vec![attachment], Availability::Present, Origin::RuntimeAssigned);
            observed.network_mode = match mode {
                "host" => Observed::present(
                    NetworkModeObservation {
                        kind: NetworkModeKind::Host,
                        native_value: ProtectedValue::new(b"host".to_vec()),
                    },
                    Availability::Present,
                    Origin::Effective,
                ),
                "malformed" => Observed::present(
                    NetworkModeObservation {
                        kind: NetworkModeKind::Named,
                        native_value: ProtectedValue::new(vec![0xff]),
                    },
                    Availability::Present,
                    Origin::Effective,
                ),
                "redacted" => Observed::unavailable(Availability::Redacted, Origin::Effective),
                _ => unreachable!("closed network mode fixture"),
            };
            let source = source_with_resources(vec![observed], vec![network()], Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.networks().is_empty());
            assert!(service.image().is_some());
            assert!(imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Unsupported
            }));
            for child in ["requested_ipv4_address", "aliases"] {
                assert!(
                    imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == format!("services[0].networks[0].{child}")
                            && outcome.kind() == ConversionKind::Unsupported
                    }),
                    "mode: {mode}"
                );
            }
            assert!(!format!("{imported:?}").contains("private-alias"), "mode: {mode}");
        }
        Ok(())
    }

    #[test]
    fn redacted_network_membership_retains_other_service_fields() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        observed.networks = Observed::unavailable(Availability::Redacted, Origin::RuntimeAssigned);
        let source = source_with_resources(vec![observed], vec![network()], Vec::new(), DockerSelection::Explicit)?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert!(service.image().is_some());
        assert!(service.networks().is_empty());
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Unsupported
        }));
        Ok(())
    }

    #[test]
    fn valued_redacted_network_membership_does_not_promote_attachments_or_aliases() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for availability in [Availability::Present, Availability::Redacted] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut attachment = network_attachment();
            attachment.aliases = Observed::present(
                vec![ProtectedValue::new(b"retained-redacted-alias".to_vec())],
                Availability::Present,
                Origin::Effective,
            );
            // The attachment and inspected resource have matching IDs, so only redaction blocks linking.
            observed.networks = Observed::present(vec![attachment], availability, Origin::RuntimeAssigned);
            let source = source_with_resources(vec![observed], vec![network()], Vec::new(), DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.image().is_some());
            if availability == Availability::Present {
                assert_eq!(service.networks().len(), 1);
                assert_eq!(service.networks()[0].value().aliases(), ["retained-redacted-alias"]);
            } else {
                assert!(service.networks().is_empty());
                assert!(imported.outcomes().iter().any(|outcome| {
                    outcome.subject() == "services[0].networks" && outcome.kind() == ConversionKind::Unsupported
                }));
                assert!(!format!("{imported:?}").contains("retained-redacted-alias"));
            }
        }
        Ok(())
    }

    #[test]
    fn named_mount_requires_one_selected_inspected_volume() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        observed.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
        let source = source_with_resources(vec![observed], Vec::new(), vec![volume()], DockerSelection::Explicit)?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert_eq!(service.mounts().len(), 1);
        assert!(
            matches!(service.mounts()[0].value().source(), MountSource::Volume(name) if name.as_str() == "docker-volume-1")
        );
        assert!(!format!("{imported:?}").contains("/private/runtime/volume"));

        let mut ambiguous = container(observed_image(b"app:1", Origin::Effective));
        ambiguous.mounts = Observed::present(vec![named_mount()], Availability::Present, Origin::Effective);
        let source = source_with_resources(
            vec![ambiguous],
            Vec::new(),
            vec![volume(), volume()],
            DockerSelection::Explicit,
        )?
        .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let imported = adapter.import(&source);
        let service = imported
            .application()
            .ok_or("missing ambiguous application")?
            .services()[0]
            .value();
        assert!(service.mounts().is_empty());
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].mounts[0].name" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{imported:?}").contains("private-volume"));
        Ok(())
    }

    #[test]
    fn decoded_named_volume_backing_path_is_observation_only() -> Result<(), String> {
        let source = decoded_source_fixture(
            r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{"Image":"app:1"},"Mounts":[{"Type":"volume","Name":"private-volume","Source":"/private/daemon/backing","Destination":"/data","RW":true,"Mode":""}]}"#,
            Some(r#"{"Name":"private-volume","Driver":"local","Mountpoint":"/private/daemon/backing"}"#),
            None,
        )?
        .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert!(matches!(service.mounts()[0].value().source(), MountSource::Volume(_)));
        assert!(!imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].mounts[0].source" && outcome.kind() == ConversionKind::Exact
        }));
        assert!(
            imported
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.summary().contains("daemon-local observation") })
        );
        assert!(!format!("{imported:?}").contains("/private/daemon/backing"));
        Ok(())
    }

    #[test]
    fn redacted_mount_options_refuse_the_mount_and_set() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (name, mut mount, volumes, policy) in [
            (
                "mode",
                named_mount(),
                vec![volume()],
                DockerPromotionPolicy::portable_application(),
            ),
            (
                "propagation",
                bind_mount(),
                Vec::new(),
                DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
            ),
        ] {
            if name == "mode" {
                mount.mode = Observed::unavailable(Availability::Redacted, Origin::Effective);
                mount.mode_interpretation = Observed::unavailable(Availability::Redacted, Origin::Effective);
            } else {
                mount.propagation = Observed::unavailable(Availability::Redacted, Origin::Effective);
            }
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            observed.mounts = Observed::present(vec![mount], Availability::Present, Origin::Effective);
            let source = source_with_resources(vec![observed], Vec::new(), volumes, DockerSelection::Explicit)?
                .with_promotion_policy(policy);
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert!(service.mounts().is_empty());
            assert!(imported.outcomes().iter().any(|outcome| {
                outcome.subject() == format!("services[0].mounts[0].{name}")
                    && outcome.kind() == ConversionKind::Unsupported
            }));
            assert!(imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].mounts" && outcome.kind() == ConversionKind::Unsupported
            }));
            assert!(!imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].mounts" && outcome.kind() == ConversionKind::Exact
            }));
        }
        Ok(())
    }

    #[test]
    fn decoded_volume_label_modes_preserve_required_storage_at_api_boundaries() -> Result<(), String> {
        for api_minor in [41, 56] {
            for (mode, read_write) in [
                ("z", true),
                ("Z", false),
                ("ro,z", false),
                ("rw,z", true),
                ("ro,Z", false),
                ("rw,Z", true),
            ] {
                let inspect = format!(
                    r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"volume","Name":"private-volume","Source":"/private/daemon/backing-canary","Destination":"/data","Mode":"{mode}","RW":{read_write},"Propagation":""}}]}}"#
                );
                let source = decoded_source_fixture_at_api(
                    &inspect,
                    Some(r#"{"Name":"private-volume","Driver":"local","Mountpoint":"/private/daemon/backing-canary"}"#),
                    None,
                    api_minor,
                )?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
                let raw = &source.inventory().containers[0]
                    .mounts
                    .value()
                    .ok_or("missing native mount")?[0];
                assert_eq!(
                    raw.mode.value().ok_or("missing protected mode")?.as_bytes(),
                    mode.as_bytes()
                );
                assert_eq!(
                    raw.name.value().ok_or("missing protected name")?.as_bytes(),
                    b"private-volume"
                );
                assert_eq!(
                    raw.source.value().ok_or("missing protected backing path")?.as_bytes(),
                    b"/private/daemon/backing-canary"
                );
                assert_eq!(raw.read_write.value(), Some(&read_write));
                let imported = DockerImporter::new()
                    .map_err(|error| error.to_string())?
                    .import(&source);
                assert_volume_label_limitation(&imported, read_write)?;
                assert!(!format!("{imported:?}").contains("backing-canary"));
            }
        }
        Ok(())
    }

    #[test]
    fn decoded_volume_label_evidence_does_not_infer_binds_authorship() -> Result<(), String> {
        for api_minor in [41, 56] {
            for (bind, mode) in [
                ("private-volume:/data", "z"),
                ("private-volume:/data:z", "z"),
                ("private-volume:/data:Z", "Z"),
            ] {
                let inspect = format!(
                    r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"HostConfig":{{"Binds":["{bind}"],"Mounts":[{{"Type":"volume","Source":"private-volume","Target":"/data"}}]}},"Mounts":[{{"Type":"volume","Name":"private-volume","Source":"/private/daemon/backing-canary","Destination":"/data","Mode":"{mode}","RW":true,"Propagation":""}}]}}"#
                );
                let source = decoded_source_fixture_at_api(
                    &inspect,
                    Some(r#"{"Name":"private-volume","Driver":"local"}"#),
                    None,
                    api_minor,
                )?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
                let observed = &source.inventory().containers[0];
                assert_eq!(
                    observed.runtime.binds.value().ok_or("missing protected Binds")?[0].as_bytes(),
                    bind.as_bytes()
                );
                assert_eq!(
                    observed.mounts.value().ok_or("missing native mount")?[0]
                        .mode
                        .value()
                        .ok_or("missing protected mode")?
                        .as_bytes(),
                    mode.as_bytes()
                );
                let imported = DockerImporter::new()
                    .map_err(|error| error.to_string())?
                    .import(&source);
                assert_volume_label_limitation(&imported, true)?;
                for subject in ["services[0].binds", "services[0].configured_mounts"] {
                    assert!(
                        imported.outcomes().iter().any(
                            |outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported
                        )
                    );
                }
                assert!(!format!("{imported:?}").contains("private-volume"));
                assert!(!format!("{imported:?}").contains("backing-canary"));
            }
        }
        Ok(())
    }

    #[test]
    fn decoded_bind_modes_preserve_access_relabel_raw_evidence_and_promotion() -> Result<(), String> {
        for (mode, read_write, relabel) in [
            ("ro,z", false, SelinuxRelabel::Shared),
            ("rw,z", true, SelinuxRelabel::Shared),
            ("ro,Z", false, SelinuxRelabel::Private),
            ("rw,Z", true, SelinuxRelabel::Private),
            ("z", false, SelinuxRelabel::Shared),
            ("Z", true, SelinuxRelabel::Private),
        ] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"bind","Source":"/fake/private-source-canary","Destination":"/data","Mode":"{mode}","RW":{read_write},"Propagation":"rprivate"}}]}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true));
            let raw = source.inventory().containers[0]
                .mounts
                .value()
                .ok_or("missing native mount")?;
            assert_eq!(
                raw[0].mode.value().ok_or("missing raw mode")?.as_bytes(),
                mode.as_bytes()
            );
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(service.mounts().len(), 1);
            let mapped = &service.mounts()[0];
            assert_eq!(mapped.value().read_only(), !read_write);
            assert_eq!(mapped.value().selinux_relabel(), Some(relabel));
            assert_value_reconstruction(mapped, true);
            for subject in [
                "services[0].mounts[0].mode",
                "services[0].mounts[0].selinux_relabel",
                "services[0].mounts[0].propagation",
            ] {
                assert!(
                    imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Exact)
                );
                assert!(imported.diagnostics().iter().any(|diagnostic| {
                    diagnostic.code().as_str() == "BFD0003"
                        && diagnostic
                            .fields()
                            .iter()
                            .any(|field| field.name() == "subject" && field.value().expose() == subject)
                }));
            }
            assert!(
                !format!("{:?}{:?}", imported.diagnostics(), imported.outcomes()).contains("private-source-canary")
            );
            assert!(!format!("{:?}", raw[0].mode).contains(mode));
        }
        Ok(())
    }

    #[test]
    fn absent_or_null_mode_never_proves_a_plain_mount() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for availability in [Availability::Missing, Availability::Null] {
            for origin in [Origin::Configured, Origin::Effective] {
                for kind in [MountKind::Bind, MountKind::Volume] {
                    let mut mount = if kind == MountKind::Bind {
                        bind_mount()
                    } else {
                        named_mount()
                    };
                    mount.mode = Observed::unavailable(availability, origin);
                    mount.mode_interpretation = Observed::unavailable(availability, origin);
                    let imported = import_one_mount(mount, policy)?;
                    assert!(
                        imported.application().ok_or("missing application")?.services()[0]
                            .value()
                            .mounts()
                            .is_empty()
                    );
                    assert!(
                        imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                                && outcome.kind() == ConversionKind::Unsupported)
                    );
                    assert!(
                        !imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                                && outcome.kind() == ConversionKind::Exact)
                    );
                    assert!(!format!("{:?}{:?}", imported.diagnostics(), imported.outcomes()).contains("/private/"));
                }
            }
        }
        for mode_field in ["", ",\"Mode\":null"] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"bind","Source":"/fake/private-source-canary","Destination":"/data","RW":true{mode_field}}}]}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?.with_promotion_policy(policy);
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                        && outcome.kind() == ConversionKind::Unsupported)
            );
            assert!(!format!("{imported:?}").contains("canary"));
        }
        Ok(())
    }

    #[test]
    fn explicit_empty_mode_proves_plain_access_only_with_a_meaningful_origin() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for read_write in [false, true] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"bind","Source":"/fake/private-source-canary","Destination":"/data","Mode":"","RW":{read_write},"Propagation":"rprivate"}}]}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?.with_promotion_policy(policy);
            let raw = source.inventory().containers[0]
                .mounts
                .value()
                .ok_or("missing native mount")?;
            assert_eq!(raw[0].mode.availability, Availability::Empty);
            assert_eq!(raw[0].mode_interpretation.availability, Availability::Empty);
            assert!(raw[0].mode.value().ok_or("missing explicit raw mode")?.is_empty());
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            let mounts = imported.application().ok_or("missing application")?.services()[0]
                .value()
                .mounts();
            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].value().read_only(), !read_write);
            assert_eq!(mounts[0].value().selinux_relabel(), None);
            assert_value_reconstruction(&mounts[0], true);
            assert_exact_reconstruction(&imported, "services[0].mounts[0].mode", true);
        }
        for origin in [
            Origin::Configured,
            Origin::Effective,
            Origin::RuntimeAssigned,
            Origin::Unknown,
        ] {
            let mut mount = bind_mount();
            mount.mode = explicit_empty_mode(origin);
            mount.mode_interpretation = explicit_empty_mode_interpretation(origin);
            let imported = import_one_mount(mount, policy)?;
            let mounts = imported.application().ok_or("missing application")?.services()[0]
                .value()
                .mounts();
            assert_eq!(
                mounts.len(),
                usize::from(matches!(origin, Origin::Configured | Origin::Effective))
            );
        }
        Ok(())
    }

    #[test]
    fn contradictory_empty_mode_evidence_is_invalid_without_parsing_native_grammar() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for case in [
            "empty-raw-nonempty",
            "empty-typed-access",
            "empty-typed-relabel",
            "present-raw-empty",
        ] {
            let mut mount = bind_mount();
            match case {
                "empty-raw-nonempty" => {
                    mount.mode = Observed::present(
                        ProtectedValue::new(b"private-mode-canary".to_vec()),
                        Availability::Empty,
                        Origin::Effective,
                    );
                }
                "empty-typed-access" => {
                    mount.mode_interpretation = Observed::present(
                        MountModeInterpretation::Supported {
                            access: Some(MountAccess::ReadWrite),
                            relabel: None,
                        },
                        Availability::Empty,
                        Origin::Effective,
                    );
                }
                "empty-typed-relabel" => {
                    mount.mode_interpretation = Observed::present(
                        MountModeInterpretation::Supported {
                            access: None,
                            relabel: Some(BindRelabel::Private),
                        },
                        Availability::Empty,
                        Origin::Effective,
                    );
                }
                "present-raw-empty" => {
                    mount.mode = Observed::present(
                        ProtectedValue::new(Vec::new()),
                        Availability::Present,
                        Origin::Effective,
                    );
                    mount.mode_interpretation = Observed::present(
                        MountModeInterpretation::Supported {
                            access: None,
                            relabel: None,
                        },
                        Availability::Present,
                        Origin::Effective,
                    );
                }
                _ => unreachable!("closed empty mode control"),
            }
            let imported = import_one_mount(mount, policy)?;
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                        && outcome.kind() == ConversionKind::Invalid),
                "{case}"
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].selinux_relabel")
            );
            assert!(!format!("{imported:?}").contains("canary"));
        }
        Ok(())
    }

    #[test]
    fn propagation_availability_cannot_hide_a_value_or_fabricate_an_empty_one() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for (availability, value, accepted) in [
            (Availability::Missing, None, true),
            (Availability::Null, None, true),
            (Availability::Empty, Some(b"".as_slice()), true),
            (Availability::Present, Some(b"rprivate".as_slice()), true),
            (Availability::Missing, Some(b"rshared".as_slice()), false),
            (Availability::Null, Some(b"rshared".as_slice()), false),
            (
                Availability::Empty,
                Some(b"private-propagation-canary".as_slice()),
                false,
            ),
            (Availability::Present, Some(b"".as_slice()), false),
        ] {
            let mut mount = bind_mount();
            mount.propagation = value.map_or_else(
                || Observed::unavailable(availability, Origin::Effective),
                |value| Observed::present(ProtectedValue::new(value.to_vec()), availability, Origin::Effective),
            );
            let imported = import_one_mount(mount, policy)?;
            assert_eq!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .len(),
                usize::from(accepted)
            );
            if !accepted {
                assert!(
                    imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].propagation"
                            && outcome.kind() == ConversionKind::Invalid)
                );
                assert!(
                    !imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                            && outcome.kind() == ConversionKind::Exact)
                );
            }
            assert!(!format!("{:?}{:?}", imported.diagnostics(), imported.outcomes()).contains("rshared"));
            assert!(
                !format!("{:?}{:?}", imported.diagnostics(), imported.outcomes())
                    .contains("private-propagation-canary")
            );
        }
        Ok(())
    }

    #[test]
    fn typed_bind_mode_origin_and_same_host_permission_remain_independent() -> Result<(), String> {
        for origin in [
            Origin::Configured,
            Origin::Effective,
            Origin::RuntimeAssigned,
            Origin::Unknown,
        ] {
            let imported = import_one_mount(
                mode_bind(false, BindRelabel::Private, origin),
                DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
            )?;
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(
                service.mounts().len(),
                usize::from(matches!(origin, Origin::Configured | Origin::Effective))
            );
            if matches!(origin, Origin::RuntimeAssigned | Origin::Unknown) {
                assert!(
                    imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                            && outcome.kind() == ConversionKind::Unsupported)
                );
            }
            assert!(
                !format!("{:?}{:?}", imported.diagnostics(), imported.outcomes())
                    .contains("protected-native-mode-canary")
            );
        }
        for policy in [
            DockerPromotionPolicy::default(),
            DockerPromotionPolicy::portable_application(),
        ] {
            let imported = import_one_mount(mode_bind(true, BindRelabel::Shared, Origin::Effective), policy)?;
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
        }
        Ok(())
    }

    #[test]
    fn incoherent_unavailable_and_unreviewed_modes_never_promote_a_bind() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for case in [
            "access-conflict",
            "unsupported",
            "unavailable",
            "redacted",
            "valued-redacted",
            "mismatched-origin",
            "mismatched-availability",
            "raw-unavailable",
        ] {
            let mut mount = mode_bind(true, BindRelabel::Shared, Origin::Effective);
            match case {
                "access-conflict" => {
                    mount.read_write = Observed::present(false, Availability::Present, Origin::Effective);
                }
                "unsupported" => {
                    mount.mode_interpretation = Observed::present(
                        MountModeInterpretation::Unsupported,
                        Availability::Present,
                        Origin::Effective,
                    );
                }
                "unavailable" | "redacted" | "valued-redacted" => {
                    let availability = if case == "unavailable" {
                        Availability::Present
                    } else {
                        Availability::Redacted
                    };
                    mount.mode = Observed::unavailable(availability, Origin::Effective);
                    mount.mode_interpretation = Observed::unavailable(availability, Origin::Effective);
                    if case == "valued-redacted" {
                        mount.mode = Observed::present(
                            ProtectedValue::new(b"private-mode-canary".to_vec()),
                            availability,
                            Origin::Effective,
                        );
                        mount.mode_interpretation = Observed::present(
                            MountModeInterpretation::Supported {
                                access: None,
                                relabel: Some(BindRelabel::Shared),
                            },
                            availability,
                            Origin::Effective,
                        );
                    }
                }
                "mismatched-origin" => mount.mode_interpretation.origin = Origin::Configured,
                "mismatched-availability" => mount.mode_interpretation.availability = Availability::Empty,
                "raw-unavailable" => mount.mode = Observed::unavailable(Availability::Present, Origin::Effective),
                _ => unreachable!("closed mode failure control"),
            }
            let imported = import_one_mount(mount, policy)?;
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty(),
                "{case}"
            );
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                        && matches!(outcome.kind(), ConversionKind::Invalid | ConversionKind::Unsupported)),
                "{case}"
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].selinux_relabel")
            );
            assert!(!format!("{imported:?}").contains("canary"));
        }
        Ok(())
    }

    #[test]
    fn decoded_unknown_mode_and_rw_conflicts_remain_value_free_native_findings() -> Result<(), String> {
        for (mode, read_write, expected) in [
            ("ro,z", true, "NativeConflict"),
            ("rw,z", false, "NativeConflict"),
            ("ro,Z", true, "NativeConflict"),
            ("rw,Z", false, "NativeConflict"),
            ("rw,z,private-option-canary", true, "UnsupportedValue"),
            ("ro,Z,private-option-canary", false, "UnsupportedValue"),
        ] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"bind","Source":"/fake/private-source-canary","Destination":"/data","Mode":"{mode}","RW":{read_write}}}]}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true));
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
            assert!(imported.diagnostics().iter().any(|diagnostic| {
                diagnostic
                    .native_finding()
                    .is_some_and(|finding| finding.code() == expected)
            }));
            assert!(!format!("{imported:?}").contains("canary"));
        }
        Ok(())
    }

    #[test]
    fn supported_relabel_does_not_supply_missing_or_redacted_access() -> Result<(), String> {
        for relabel in [BindRelabel::Shared, BindRelabel::Private] {
            for (availability, valued) in [
                (Availability::Present, false),
                (Availability::Missing, false),
                (Availability::Redacted, false),
                (Availability::Redacted, true),
            ] {
                let mut mount = mode_bind(true, relabel, Origin::Effective);
                mount.read_write = if valued {
                    Observed::present(true, availability, Origin::Effective)
                } else {
                    Observed::unavailable(availability, Origin::Effective)
                };
                let imported = import_one_mount(
                    mount,
                    DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
                )?;
                assert!(
                    imported.application().ok_or("missing application")?.services()[0]
                        .value()
                        .mounts()
                        .is_empty()
                );
                assert!(
                    !imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].selinux_relabel")
                );
                assert!(!format!("{imported:?}").contains("protected-native-mode-canary"));
            }
        }
        Ok(())
    }

    fn corrupt_volume_mode_evidence(mount: &mut MountObservation, case: &str) {
        match case {
            "mode-missing" | "mode-null" | "mode-redacted" | "mode-valued-redacted" => {
                let availability = match case {
                    "mode-missing" => Availability::Missing,
                    "mode-null" => Availability::Null,
                    _ => Availability::Redacted,
                };
                if case == "mode-valued-redacted" {
                    mount.mode.availability = availability;
                    mount.mode_interpretation.availability = availability;
                } else {
                    mount.mode = Observed::unavailable(availability, Origin::Effective);
                    mount.mode_interpretation = Observed::unavailable(availability, Origin::Effective);
                }
            }
            "mode-missing-value" => mount.mode = Observed::unavailable(Availability::Present, Origin::Effective),
            "typed-missing-value" => {
                mount.mode_interpretation = Observed::unavailable(Availability::Present, Origin::Effective);
            }
            "mode-origin-mismatch" => mount.mode_interpretation.origin = Origin::Configured,
            "mode-availability-mismatch" => mount.mode_interpretation.availability = Availability::Empty,
            "raw-empty-present" => {
                mount.mode = Observed::present(
                    ProtectedValue::new(Vec::new()),
                    Availability::Present,
                    Origin::Effective,
                );
            }
            "raw-nonempty-empty" => {
                mount.mode.availability = Availability::Empty;
                mount.mode_interpretation.availability = Availability::Empty;
            }
            "empty-typed-label" => {
                mount.mode = explicit_empty_mode(Origin::Effective);
                mount.mode_interpretation.availability = Availability::Empty;
            }
            "unsupported-mode" => {
                mount.mode_interpretation = Observed::present(
                    MountModeInterpretation::Unsupported,
                    Availability::Present,
                    Origin::Effective,
                );
            }
            "mode-unknown-origin" | "mode-runtime-origin" => {
                let origin = if case == "mode-unknown-origin" {
                    Origin::Unknown
                } else {
                    Origin::RuntimeAssigned
                };
                mount.mode.origin = origin;
                mount.mode_interpretation.origin = origin;
            }
            _ => corrupt_volume_access_or_propagation(mount, case),
        }
    }

    fn corrupt_volume_access_or_propagation(mount: &mut MountObservation, case: &str) {
        match case {
            "access-missing"
            | "access-null"
            | "access-redacted"
            | "access-valued-redacted"
            | "access-missing-value" => {
                let availability = match case {
                    "access-missing" => Availability::Missing,
                    "access-null" => Availability::Null,
                    "access-missing-value" => Availability::Present,
                    _ => Availability::Redacted,
                };
                mount.read_write = if case == "access-valued-redacted" {
                    Observed::present(true, availability, Origin::Effective)
                } else {
                    Observed::unavailable(availability, Origin::Effective)
                };
            }
            "access-conflict" => mount.read_write = Observed::present(false, Availability::Present, Origin::Effective),
            "access-unknown-origin" => mount.read_write.origin = Origin::Unknown,
            "access-runtime-origin" => mount.read_write.origin = Origin::RuntimeAssigned,
            "propagation-missing-valued" | "propagation-null-valued" | "propagation-valued-redacted" => {
                let availability = match case {
                    "propagation-missing-valued" => Availability::Missing,
                    "propagation-null-valued" => Availability::Null,
                    _ => Availability::Redacted,
                };
                mount.propagation = Observed::present(
                    ProtectedValue::new(b"private-propagation-canary".to_vec()),
                    availability,
                    Origin::Effective,
                );
            }
            "propagation-redacted" => {
                mount.propagation = Observed::unavailable(Availability::Redacted, Origin::Effective);
            }
            "propagation-missing-value" => {
                mount.propagation = Observed::unavailable(Availability::Present, Origin::Effective);
            }
            "propagation-empty-present" => {
                mount.propagation = Observed::present(
                    ProtectedValue::new(Vec::new()),
                    Availability::Present,
                    Origin::Effective,
                );
            }
            "propagation-nonempty-empty" => {
                mount.propagation = Observed::present(
                    ProtectedValue::new(b"rprivate".to_vec()),
                    Availability::Empty,
                    Origin::Effective,
                );
            }
            "propagation-rprivate" | "propagation-rshared" => {
                mount.propagation = observed_image(
                    if case == "propagation-rprivate" {
                        b"rprivate"
                    } else {
                        b"rshared"
                    },
                    Origin::Effective,
                );
            }
            "propagation-unknown-origin" => mount.propagation = explicit_empty_mode(Origin::Unknown),
            "propagation-runtime-origin" => mount.propagation = explicit_empty_mode(Origin::RuntimeAssigned),
            _ => unreachable!("closed named-volume evidence guard"),
        }
    }

    #[test]
    fn volume_label_limitation_never_bypasses_fallible_mode_access_or_propagation() -> Result<(), String> {
        for relabel in [BindRelabel::Shared, BindRelabel::Private] {
            for case in [
                "mode-missing",
                "mode-null",
                "mode-redacted",
                "mode-valued-redacted",
                "mode-missing-value",
                "typed-missing-value",
                "mode-origin-mismatch",
                "mode-availability-mismatch",
                "raw-empty-present",
                "raw-nonempty-empty",
                "empty-typed-label",
                "unsupported-mode",
                "mode-unknown-origin",
                "mode-runtime-origin",
                "access-missing",
                "access-null",
                "access-redacted",
                "access-valued-redacted",
                "access-missing-value",
                "access-conflict",
                "access-unknown-origin",
                "access-runtime-origin",
                "propagation-missing-valued",
                "propagation-null-valued",
                "propagation-redacted",
                "propagation-valued-redacted",
                "propagation-missing-value",
                "propagation-empty-present",
                "propagation-nonempty-empty",
                "propagation-rprivate",
                "propagation-rshared",
                "propagation-unknown-origin",
                "propagation-runtime-origin",
            ] {
                let mut mount = labelled_volume(true, relabel, Origin::Effective);
                corrupt_volume_mode_evidence(&mut mount, case);
                let imported = import_one_mount(mount, DockerPromotionPolicy::portable_application())?;
                assert!(
                    imported.application().ok_or("missing guarded application")?.services()[0]
                        .value()
                        .mounts()
                        .is_empty(),
                    "{case}"
                );
                if matches!(case, "access-missing" | "access-null") {
                    // Missing/null access has no child diagnostic; the unchanged mount-set
                    // fallback reports the omitted required storage as unsupported.
                    assert!(
                        imported.outcomes().iter().any(|outcome| {
                            outcome.subject() == "services[0].mounts" && outcome.kind() == ConversionKind::Unsupported
                        }),
                        "{case}"
                    );
                    assert!(
                        imported.diagnostics().iter().any(|diagnostic| {
                            diagnostic.code().as_str() == "BFD0002"
                                && diagnostic.summary() == "Docker mounts need field-specific review"
                                && diagnostic.fields().iter().any(|field| {
                                    field.name() == "subject" && field.value().expose() == "services[0].mounts"
                                })
                        }),
                        "{case}"
                    );
                } else {
                    assert!(
                        imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject().starts_with("services[0].mounts[0]")
                                && matches!(outcome.kind(), ConversionKind::Invalid | ConversionKind::Unsupported)),
                        "{case}"
                    );
                }
                assert!(
                    !imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                            && outcome.kind() == ConversionKind::Exact),
                    "{case}"
                );
                assert!(
                    !imported
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.summary().contains("named-volume labeling evidence")),
                    "{case}"
                );
                assert!(!format!("{imported:?}").contains("canary"), "{case}");
            }
        }
        Ok(())
    }

    #[test]
    fn labelled_volume_still_requires_one_selected_logical_volume() -> Result<(), String> {
        for volumes in [Vec::new(), vec![volume(), volume()]] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            observed.mounts = Observed::present(
                vec![labelled_volume(true, BindRelabel::Shared, Origin::Effective)],
                Availability::Present,
                Origin::Effective,
            );
            let source = source_with_resources(vec![observed], Vec::new(), volumes, DockerSelection::Explicit)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = DockerImporter::new()
                .map_err(|error| error.to_string())?
                .import(&source);
            assert!(
                imported
                    .application()
                    .ok_or("missing correlation application")?
                    .services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].name"
                        && outcome.kind() == ConversionKind::Unsupported)
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].mounts[0].mode")
            );
            assert!(!format!("{imported:?}").contains("private-volume"));
        }
        Ok(())
    }

    #[test]
    fn decoded_volume_mode_conflicts_and_unknown_options_remain_native_findings() -> Result<(), String> {
        for api_minor in [41, 56] {
            for (mode, read_write, expected) in [
                ("ro,z", true, "NativeConflict"),
                ("rw,z", false, "NativeConflict"),
                ("ro,Z", true, "NativeConflict"),
                ("rw,Z", false, "NativeConflict"),
                ("rw,z,private-option-canary", true, "UnsupportedValue"),
                ("ro,Z,private-option-canary", false, "UnsupportedValue"),
            ] {
                let inspect = format!(
                    r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1"}},"Mounts":[{{"Type":"volume","Name":"private-volume","Source":"/private/daemon/backing-canary","Destination":"/data","Mode":"{mode}","RW":{read_write}}}]}}"#
                );
                let source = decoded_source_fixture_at_api(
                    &inspect,
                    Some(r#"{"Name":"private-volume","Driver":"local"}"#),
                    None,
                    api_minor,
                )?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
                let imported = DockerImporter::new()
                    .map_err(|error| error.to_string())?
                    .import(&source);
                assert!(
                    imported
                        .application()
                        .ok_or("missing native-finding application")?
                        .services()[0]
                        .value()
                        .mounts()
                        .is_empty()
                );
                assert!(imported.diagnostics().iter().any(|diagnostic| {
                    diagnostic
                        .native_finding()
                        .is_some_and(|finding| finding.code() == expected)
                }));
                assert!(
                    !imported
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.summary().contains("named-volume labeling evidence"))
                );
                assert!(!format!("{imported:?}").contains("canary"));
            }
        }
        Ok(())
    }

    #[test]
    fn named_volume_label_limitation_accepts_only_reviewed_empty_propagation() -> Result<(), String> {
        for propagation in [
            Observed::unavailable(Availability::Missing, Origin::Effective),
            Observed::unavailable(Availability::Null, Origin::Effective),
            explicit_empty_mode(Origin::Configured),
            explicit_empty_mode(Origin::Effective),
        ] {
            let mut mount = labelled_volume(false, BindRelabel::Private, Origin::Effective);
            mount.propagation = propagation;
            let imported = import_one_mount(mount, DockerPromotionPolicy::portable_application())?;
            assert_volume_label_limitation(&imported, false)?;
        }
        Ok(())
    }

    #[test]
    fn volume_label_loss_policy_retains_storage_and_never_approves_invalid_access() -> Result<(), String> {
        let profile = TargetCapabilityCatalog::reviewed()
            .profiles()
            .find(|profile| {
                matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed Docker profile missing")?
            .clone();
        let exporter = DockerExporter::new(profile).map_err(|error| format!("{error:?}"))?;
        let version = PlatformVersion::new(29, 8, 1);
        let target = TargetProfile::new("docker", version, Some(version)).map_err(|error| format!("{error:?}"))?;
        for relabel in [BindRelabel::Shared, BindRelabel::Private] {
            for invalid_access in [false, true] {
                let mut mount = labelled_volume(false, relabel, Origin::Effective);
                if invalid_access {
                    mount.read_write = Observed::present(true, Availability::Present, Origin::Effective);
                }
                let mut observed = container(observed_image(b"busybox:1", Origin::Effective));
                observed.mounts = Observed::present(vec![mount], Availability::Present, Origin::Effective);
                let source =
                    source_with_resources(vec![observed], Vec::new(), vec![volume()], DockerSelection::Explicit)?
                        .with_promotion_policy(DockerPromotionPolicy::portable_application())
                        .with_resource_decisions(resource_decisions(&[
                            "volume:private-volume=create:destination-volume".into(),
                        ])?);
                let imported = DockerImporter::new()
                    .map_err(|error| error.to_string())?
                    .import(&source);
                if !invalid_access {
                    assert_volume_label_limitation(&imported, false)?;
                }
                for policy in [
                    LossPolicy::ExactOnly,
                    LossPolicy::AllowApproximate,
                    LossPolicy::AllowPartial,
                ] {
                    let conversion = convert_imported(imported.clone(), &exporter, &target, policy);
                    if invalid_access {
                        let Err(boxferry_engine::ConversionError::Import(diagnostics)) = conversion else {
                            return Err("invalid mount access must fail source import under every loss policy".into());
                        };
                        assert!(imported.outcomes().iter().any(|outcome| {
                            outcome.subject() == "services[0].mounts[0].mode"
                                && outcome.kind() == ConversionKind::Invalid
                        }));
                        assert!(diagnostics.iter().any(|diagnostic| {
                            diagnostic.code().as_str() == "BFD0001"
                                && diagnostic.severity() == Severity::Error
                                && diagnostic.fields().iter().any(|field| {
                                    field.name() == "subject" && field.value().expose() == "services[0].mounts[0].mode"
                                })
                        }));
                        continue;
                    }
                    let result = conversion.map_err(|error| format!("{error:?}"))?;
                    assert_eq!(result.is_blocked(), policy != LossPolicy::AllowPartial);
                    assert!(
                        result
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "services[0].mounts[0].mode"
                                && outcome.kind() == ConversionKind::Unsupported)
                    );
                    if policy == LossPolicy::AllowPartial {
                        let output = result.output().ok_or("reviewed partial volume artifact missing")?;
                        let complete =
                            String::from_utf8(output.complete_bytes().map_err(|error| format!("{error:?}"))?)
                                .map_err(|error| error.to_string())?;
                        assert!(complete.contains("\"Type\":\"volume\""));
                        assert!(complete.contains("\"Source\":\"destination-volume\""));
                        assert!(complete.contains("\"Target\":\"/data\""));
                        assert!(complete.contains("\"ReadOnly\":true"));
                        assert!(!complete.contains("private-volume"));
                        assert!(!complete.contains("/private/runtime/volume"));
                    } else {
                        assert!(result.output().is_none());
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn relabelled_named_volume_retains_core_without_bind_permission() -> Result<(), String> {
        for relabel in [BindRelabel::Shared, BindRelabel::Private] {
            for read_write in [false, true] {
                for origin in [Origin::Configured, Origin::Effective] {
                    let imported = import_one_mount(
                        labelled_volume(read_write, relabel, origin),
                        DockerPromotionPolicy::portable_application(),
                    )?;
                    assert_volume_label_limitation(&imported, read_write)?;
                }
                for policy in [
                    DockerPromotionPolicy::conservative(),
                    DockerPromotionPolicy::portable_application().with_named_volume_mounts(false),
                    DockerPromotionPolicy::portable_application()
                        .with_named_volume_mounts(false)
                        .with_same_host_bind_mounts(true),
                ] {
                    let imported = import_one_mount(labelled_volume(read_write, relabel, Origin::Effective), policy)?;
                    assert!(
                        imported.application().ok_or("missing refused application")?.services()[0]
                            .value()
                            .mounts()
                            .is_empty()
                    );
                    assert!(
                        !imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "services[0].mounts[0].mode")
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn relabelled_tmpfs_and_unreviewed_bind_propagation_are_refused() -> Result<(), String> {
        let policy = DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true);
        for kind in [MountKind::Tmpfs, MountKind::Other] {
            let mut mount = mode_bind(true, BindRelabel::Private, Origin::Effective);
            mount.kind = kind;
            let imported = import_one_mount(mount, policy)?;
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .is_empty()
            );
            assert!(imported.outcomes().iter().any(|outcome| matches!(
                outcome.kind(),
                ConversionKind::Invalid | ConversionKind::Unsupported
            )
                && outcome.subject().starts_with("services[0].mounts[0]")));
        }
        for propagation in [
            b"".as_slice(),
            b"rprivate",
            b"private",
            b"rshared",
            b"private-propagation-canary",
        ] {
            let mut mount = mode_bind(false, BindRelabel::Shared, Origin::Effective);
            mount.propagation = Observed::present(
                ProtectedValue::new(propagation.to_vec()),
                if propagation.is_empty() {
                    Availability::Empty
                } else {
                    Availability::Present
                },
                Origin::Effective,
            );
            let imported = import_one_mount(mount, policy)?;
            let accepted = propagation.is_empty() || propagation == b"rprivate";
            assert_eq!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .mounts()
                    .len(),
                usize::from(accepted)
            );
            if !accepted {
                assert!(
                    imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].mounts[0].propagation"
                            && outcome.kind() == ConversionKind::Unsupported)
                );
                assert!(!format!("{imported:?}").contains("private-propagation-canary"));
            }
        }
        Ok(())
    }

    #[test]
    fn bind_mount_needs_separate_same_host_permission() -> Result<(), String> {
        let make_source = || -> Result<DockerSource, String> {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            observed.mounts = Observed::present(vec![bind_mount()], Availability::Present, Origin::Effective);
            source(vec![observed])
        };
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let portable =
            adapter.import(&make_source()?.with_promotion_policy(DockerPromotionPolicy::portable_application()));
        assert!(
            portable.application().ok_or("missing application")?.services()[0]
                .value()
                .mounts()
                .is_empty()
        );
        assert!(!format!("{portable:?}").contains("/private/same-host/data"));
        let authorized = adapter.import(
            &make_source()?
                .with_promotion_policy(DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true)),
        );
        let service = authorized
            .application()
            .ok_or("missing authorized application")?
            .services()[0]
            .value();
        assert!(
            matches!(service.mounts()[0].value().source(), MountSource::HostPath(path) if path == "/private/same-host/data")
        );
        Ok(())
    }

    #[test]
    fn fixed_and_ephemeral_hostconfig_requests_remain_distinct_from_runtime_ports() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        observed.configured_ports = Observed::present(
            vec![PortObservation {
                key: PortKey {
                    container_port: 8080,
                    protocol: TransportProtocol::Tcp,
                },
                bindings: Observed::present(
                    vec![
                        HostBinding {
                            host_ip: absent(),
                            host_port: Observed::present(18080, Availability::Present, Origin::Effective),
                        },
                        HostBinding {
                            host_ip: absent(),
                            host_port: Observed::present(0, Availability::Present, Origin::Effective),
                        },
                    ],
                    Availability::Present,
                    Origin::Effective,
                ),
            }],
            Availability::Present,
            Origin::Effective,
        );
        observed.runtime_ports = Observed::present(
            vec![PortObservation {
                key: PortKey {
                    container_port: 9999,
                    protocol: TransportProtocol::Tcp,
                },
                bindings: absent(),
            }],
            Availability::Present,
            Origin::RuntimeAssigned,
        );
        let source = source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application());
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert_eq!(service.ports().len(), 2);
        assert_eq!(service.ports()[0].value().published(), Some(18080));
        assert_eq!(service.ports()[1].value().published(), Some(0));
        assert_value_reconstruction(&service.ports()[1], true);
        assert_exact_reconstruction(&imported, "services[0].configured_ports", true);
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].runtime_ports" && outcome.kind() == ConversionKind::Unsupported
        }));
        Ok(())
    }

    #[test]
    fn configured_ephemeral_publication_keeps_host_ip_without_promotion() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Configured));
        observed.configured_ports = Observed::present(
            vec![typed_publication(
                Observed::present(0, Availability::Present, Origin::Configured),
                observed_image(b"127.0.0.1", Origin::Configured),
                Origin::Configured,
            )],
            Availability::Present,
            Origin::Configured,
        );
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source(vec![observed])?);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert_eq!(service.ports().len(), 1);
        assert_eq!(service.ports()[0].value().published(), Some(0));
        assert_eq!(service.ports()[0].value().host_address(), Some("127.0.0.1"));
        assert_value_reconstruction(&service.ports()[0], false);
        assert_exact_reconstruction(&imported, "services[0].configured_ports", false);
        Ok(())
    }

    #[test]
    fn mixed_port_origins_reconstruct_only_consumed_binding_children() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for child in ["none", "outer", "bindings", "host_port", "host_ip"] {
            let origin = |field| {
                if child == field {
                    Origin::Effective
                } else {
                    Origin::Configured
                }
            };
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            observed.configured_ports = Observed::present(
                vec![typed_publication(
                    Observed::present(0, Availability::Present, origin("host_port")),
                    observed_image(b"::1", origin("host_ip")),
                    origin("bindings"),
                )],
                Availability::Present,
                origin("outer"),
            );
            let plain = source(vec![observed])?;
            let conservative = adapter.import(&plain);
            let imported = adapter.import(&plain.with_promotion_policy(DockerPromotionPolicy::portable_application()));
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            assert_eq!(service.ports().len(), 1, "child: {child}");
            assert_eq!(service.ports()[0].value().published(), Some(0));
            assert_eq!(service.ports()[0].value().host_address(), Some("::1"));
            assert_value_reconstruction(&service.ports()[0], child != "none");
            for subject in [
                "services[0].configured_ports[0].bindings[0]",
                "services[0].configured_ports",
            ] {
                assert_exact_reconstruction(&imported, subject, child != "none");
            }
            if child != "none" {
                assert!(
                    conservative
                        .application()
                        .ok_or("missing conservative application")?
                        .services()[0]
                        .value()
                        .ports()
                        .is_empty()
                );
                assert!(
                    !conservative
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                );
            }
        }
        Ok(())
    }

    #[test]
    fn missing_redacted_unknown_and_runtime_port_children_never_become_publications() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for origin in [Origin::Unknown, Origin::RuntimeAssigned] {
            for child in ["outer", "bindings", "host_port", "host_ip"] {
                for host_port in [0, 49152] {
                    let selected = |field| if child == field { origin } else { Origin::Configured };
                    let mut observed = container(observed_image(b"app:1", Origin::Configured));
                    observed.configured_ports = Observed::present(
                        vec![typed_publication(
                            Observed::present(host_port, Availability::Present, selected("host_port")),
                            observed_image(b"127.0.0.1", selected("host_ip")),
                            selected("bindings"),
                        )],
                        Availability::Present,
                        selected("outer"),
                    );
                    observed.runtime_ports = Observed::present(
                        vec![typed_publication(
                            Observed::present(49152, Availability::Present, Origin::RuntimeAssigned),
                            observed_image(b"127.0.0.1", Origin::RuntimeAssigned),
                            Origin::RuntimeAssigned,
                        )],
                        Availability::Present,
                        Origin::RuntimeAssigned,
                    );
                    let imported = adapter.import(
                        &source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()),
                    );
                    assert!(
                        imported.application().ok_or("missing application")?.services()[0]
                            .value()
                            .ports()
                            .is_empty()
                    );
                    assert!(
                        !imported
                            .diagnostics()
                            .iter()
                            .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                    );
                    assert!(
                        imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "services[0].runtime_ports"
                                && outcome.kind() == ConversionKind::Unsupported)
                    );
                }
            }
        }
        for availability in [Availability::Missing, Availability::Null, Availability::Redacted] {
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            observed.configured_ports = Observed::present(
                vec![typed_publication(
                    Observed::unavailable(availability, Origin::Effective),
                    absent(),
                    Origin::Configured,
                )],
                Availability::Present,
                Origin::Configured,
            );
            let imported = adapter
                .import(&source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()));
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .ports()
                    .is_empty()
            );
            assert!(
                !imported
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
            );
        }
        Ok(())
    }

    #[test]
    fn explicitly_present_empty_restart_name_retains_no_restart_semantics_and_promotion_boundaries()
    -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        // Authored typed observations, not claims about daemon-emitted defaults.
        for (name, availability) in [
            (b"".as_slice(), Availability::Empty),
            (b"".as_slice(), Availability::Present),
            (b"no".as_slice(), Availability::Present),
        ] {
            for outer_origin in [Origin::Configured, Origin::Effective] {
                for name_origin in [Origin::Configured, Origin::Effective] {
                    for portable in [false, true] {
                        let mut observed = container(observed_image(b"app:1", Origin::Configured));
                        observed.restart_policy = Observed::present(
                            NativeRestartPolicy {
                                name: Observed::present(ProtectedValue::new(name.to_vec()), availability, name_origin),
                                maximum_retry_count: Observed::present(0, Availability::Present, Origin::Configured),
                            },
                            Availability::Present,
                            outer_origin,
                        );
                        let plain = source(vec![observed])?;
                        let selected = plain.with_promotion_policy(if portable {
                            DockerPromotionPolicy::portable_application()
                        } else {
                            DockerPromotionPolicy::conservative()
                        });
                        let imported = adapter.import(&selected);
                        let service =
                            imported.application().ok_or("missing restart application")?.services()[0].value();
                        let reconstructed = outer_origin == Origin::Effective || name_origin == Origin::Effective;
                        if portable || !reconstructed {
                            let policy = service.restart_policy().ok_or("present disabled restart was lost")?;
                            assert_eq!(*policy.value(), boxferry_model::RestartPolicy::Never);
                            assert_value_reconstruction(policy, reconstructed);
                            assert_exact_reconstruction(&imported, "services[0].restart_policy", reconstructed);
                        } else {
                            assert!(service.restart_policy().is_none());
                            assert!(!imported.outcomes().iter().any(|outcome| {
                                outcome.subject() == "services[0].restart_policy"
                                    && outcome.kind() == ConversionKind::Exact
                            }));
                            assert!(imported.outcomes().iter().any(|outcome| {
                                outcome.subject().starts_with("services[0].restart_policy")
                                    && outcome.kind() == ConversionKind::Unsupported
                            }));
                        }
                        let retained = selected.inventory().containers[0]
                            .restart_policy
                            .value()
                            .ok_or("missing native restart")?;
                        assert_eq!(retained.name.availability, availability);
                        assert_eq!(retained.name.origin, name_origin);
                        assert_eq!(retained.name.value().ok_or("missing explicit name")?.as_bytes(), name);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn unavailable_restart_names_are_not_inferred_from_empty_availability() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for availability in [
            Availability::Missing,
            Availability::Null,
            Availability::Redacted,
            Availability::Empty,
        ] {
            for name_origin in [Origin::Configured, Origin::Effective] {
                for portable in [false, true] {
                    let mut observed = container(observed_image(b"app:1", Origin::Configured));
                    observed.restart_policy = Observed::present(
                        NativeRestartPolicy {
                            name: Observed::unavailable(availability, name_origin),
                            maximum_retry_count: Observed::present(0, Availability::Present, Origin::Configured),
                        },
                        Availability::Present,
                        Origin::Configured,
                    );
                    let selected = source(vec![observed])?.with_promotion_policy(if portable {
                        DockerPromotionPolicy::portable_application()
                    } else {
                        DockerPromotionPolicy::conservative()
                    });
                    let imported = adapter.import(&selected);
                    assert!(
                        imported.application().ok_or("missing restart application")?.services()[0]
                            .value()
                            .restart_policy()
                            .is_none()
                    );
                    assert!(!imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == "services[0].restart_policy" && outcome.kind() == ConversionKind::Exact
                    }));
                    assert!(imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == "services[0].restart_policy"
                            && outcome.kind() == ConversionKind::Unsupported
                    }));
                    let retained = selected.inventory().containers[0]
                        .restart_policy
                        .value()
                        .ok_or("missing native restart")?;
                    assert_eq!(retained.name.availability, availability);
                    assert!(retained.name.value().is_none());
                }
            }
        }
        Ok(())
    }

    #[test]
    fn disabled_restart_names_reject_nonzero_retry_counts() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for name in [b"".as_slice(), b"no".as_slice()] {
            for retry_count in [1, 3] {
                for origin in [Origin::Configured, Origin::Effective] {
                    let mut observed = container(observed_image(b"app:1", Origin::Configured));
                    observed.restart_policy = Observed::present(
                        NativeRestartPolicy {
                            name: observed_image(name, origin),
                            maximum_retry_count: Observed::present(retry_count, Availability::Present, origin),
                        },
                        Availability::Present,
                        origin,
                    );
                    let imported = adapter.import(
                        &source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()),
                    );
                    assert!(
                        imported.application().ok_or("missing restart application")?.services()[0]
                            .value()
                            .restart_policy()
                            .is_none()
                    );
                    assert!(!imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == "services[0].restart_policy" && outcome.kind() == ConversionKind::Exact
                    }));
                    assert!(imported.outcomes().iter().any(|outcome| {
                        outcome.subject() == "services[0].restart_policy.maximum_retry_count"
                            && outcome.kind() == ConversionKind::Unsupported
                    }));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn mixed_restart_origins_retain_only_consumed_policy_provenance() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for child in ["none", "outer", "name", "retry"] {
            let origin = |field| {
                if child == field {
                    Origin::Effective
                } else {
                    Origin::Configured
                }
            };
            let mut observed = container(observed_image(b"app:1", Origin::Configured));
            observed.restart_policy = Observed::present(
                NativeRestartPolicy {
                    name: observed_image(b"on-failure", origin("name")),
                    maximum_retry_count: Observed::present(3, Availability::Present, origin("retry")),
                },
                Availability::Present,
                origin("outer"),
            );
            let plain = source(vec![observed])?;
            let conservative = adapter.import(&plain);
            let imported = adapter.import(&plain.with_promotion_policy(DockerPromotionPolicy::portable_application()));
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            let policy = service.restart_policy().ok_or("missing restart policy")?;
            assert_eq!(
                *policy.value(),
                boxferry_model::RestartPolicy::on_failure(std::num::NonZeroU64::new(3))
            );
            assert_value_reconstruction(policy, child != "none");
            assert_exact_reconstruction(&imported, "services[0].restart_policy", child != "none");
            if child != "none" {
                assert!(
                    conservative
                        .application()
                        .ok_or("missing conservative application")?
                        .services()[0]
                        .value()
                        .restart_policy()
                        .is_none()
                );
                assert!(
                    !conservative
                        .diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                );
            }
        }
        let mut observed = container(observed_image(b"app:1", Origin::Configured));
        observed.restart_policy = Observed::present(
            NativeRestartPolicy {
                name: observed_image(b"always", Origin::Configured),
                maximum_retry_count: Observed::present(0, Availability::Present, Origin::Effective),
            },
            Availability::Present,
            Origin::Configured,
        );
        let imported = adapter.import(&source(vec![observed])?);
        let policy = imported.application().ok_or("missing application")?.services()[0]
            .value()
            .restart_policy()
            .ok_or("missing always policy")?;
        assert_value_reconstruction(policy, false);
        assert_exact_reconstruction(&imported, "services[0].restart_policy", false);
        Ok(())
    }

    #[test]
    fn mixed_health_origins_retain_aggregate_and_individual_child_provenance() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for disabled in [false, true] {
            for child in [
                "none",
                "outer",
                "test",
                "interval",
                "timeout",
                "start_period",
                "start_interval",
                "retries",
            ] {
                let mut observed = container(observed_image(b"app:1", Origin::Configured));
                observed.healthcheck = typed_healthcheck(child, Origin::Effective, Availability::Present, disabled);
                let plain = source(vec![observed])?;
                let conservative = adapter.import(&plain);
                let imported =
                    adapter.import(&plain.with_promotion_policy(DockerPromotionPolicy::portable_application()));
                let health = imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .healthcheck()
                    .ok_or("missing healthcheck")?;
                assert_value_reconstruction(health, child != "none");
                assert_exact_reconstruction(&imported, "services[0].healthcheck", child != "none");
                if disabled {
                    assert_value_reconstruction(
                        health.value().disabled().ok_or("missing disabled flag")?,
                        matches!(child, "outer" | "test"),
                    );
                } else {
                    assert_value_reconstruction(
                        health.value().command().ok_or("missing command")?,
                        matches!(child, "outer" | "test"),
                    );
                }
                assert_value_reconstruction(
                    health.value().interval().ok_or("missing interval")?,
                    matches!(child, "outer" | "interval"),
                );
                assert_value_reconstruction(
                    health.value().retries().ok_or("missing retries")?,
                    matches!(child, "outer" | "retries"),
                );
                if child != "none" {
                    assert!(
                        conservative
                            .application()
                            .ok_or("missing conservative application")?
                            .services()[0]
                            .value()
                            .healthcheck()
                            .is_none()
                    );
                    assert!(
                        !conservative
                            .diagnostics()
                            .iter()
                            .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn unusable_restart_and_health_children_do_not_gain_reconstruction_provenance() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for origin in [Origin::Unknown, Origin::RuntimeAssigned, Origin::Effective] {
            for availability in [Availability::Present, Availability::Redacted] {
                if origin == Origin::Effective && availability == Availability::Present {
                    continue;
                }
                for child in [
                    "restart_name",
                    "restart_retry",
                    "health_test",
                    "health_interval",
                    "health_retry",
                ] {
                    let mut observed = container(observed_image(b"app:1", Origin::Configured));
                    let text = if availability == Availability::Redacted {
                        Observed::unavailable(availability, origin)
                    } else {
                        observed_image(b"on-failure", origin)
                    };
                    observed.restart_policy = Observed::present(
                        NativeRestartPolicy {
                            name: if child == "restart_name" {
                                text
                            } else {
                                observed_image(b"on-failure", Origin::Configured)
                            },
                            maximum_retry_count: if child == "restart_retry" {
                                if availability == Availability::Redacted {
                                    Observed::unavailable(availability, origin)
                                } else {
                                    Observed::present(3, availability, origin)
                                }
                            } else {
                                Observed::present(3, Availability::Present, Origin::Configured)
                            },
                        },
                        Availability::Present,
                        Origin::Configured,
                    );
                    let health_child = match child {
                        "health_test" => "test",
                        "health_interval" => "interval",
                        "health_retry" => "retries",
                        _ => "none",
                    };
                    observed.healthcheck = typed_healthcheck(health_child, origin, availability, false);
                    let imported = adapter.import(
                        &source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()),
                    );
                    let service = imported.application().ok_or("missing application")?.services()[0].value();
                    let subject = if child.starts_with("restart") {
                        assert!(service.restart_policy().is_none());
                        "services[0].restart_policy"
                    } else {
                        assert!(service.healthcheck().is_none());
                        "services[0].healthcheck"
                    };
                    assert!(
                        !imported
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Exact)
                    );
                    assert!(
                        !imported
                            .diagnostics()
                            .iter()
                            .any(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                    );
                    assert!(!format!("{:?}", imported.diagnostics()).contains("child-private-canary"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn mixed_mount_origins_retain_only_destination_access_and_logical_source_provenance() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for bind in [false, true] {
            for child in ["none", "outer", "destination", "read_write", "logical_source"] {
                let mut observed = container(observed_image(b"app:1", Origin::Configured));
                observed.mounts = typed_mount(bind, child, Origin::Effective, Availability::Present);
                let plain = source_with_resources(
                    vec![observed],
                    Vec::new(),
                    if bind { Vec::new() } else { vec![volume()] },
                    DockerSelection::Explicit,
                )?;
                let conservative = adapter.import(&plain);
                assert!(
                    conservative
                        .application()
                        .ok_or("missing conservative application")?
                        .services()[0]
                        .value()
                        .mounts()
                        .is_empty()
                );
                let imported = adapter.import(&plain.with_promotion_policy(
                    DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
                ));
                let service = imported.application().ok_or("missing application")?.services()[0].value();
                assert_eq!(service.mounts().len(), 1);
                let mount = &service.mounts()[0];
                assert_eq!(mount.value().target(), "/data");
                assert!(mount.value().read_only());
                if bind {
                    assert!(
                        matches!(mount.value().source(), MountSource::HostPath(path) if path == "/same-host/child-private-canary")
                    );
                } else {
                    assert!(
                        matches!(mount.value().source(), MountSource::Volume(name) if name.as_str() == "docker-volume-1")
                    );
                }
                assert_value_reconstruction(mount, child != "none");
                for subject in ["services[0].mounts[0]", "services[0].mounts"] {
                    assert_exact_reconstruction(&imported, subject, child != "none");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn unusable_mount_children_are_rejected_without_reconstruction_notes() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for bind in [false, true] {
            for child in ["destination", "read_write", "logical_source"] {
                for origin in [Origin::Unknown, Origin::RuntimeAssigned, Origin::Effective] {
                    for redacted in [false, true] {
                        if origin == Origin::Effective && !redacted {
                            continue;
                        }
                        let mut observed = container(observed_image(b"app:1", Origin::Configured));
                        observed.mounts = typed_mount(
                            bind,
                            child,
                            origin,
                            if redacted {
                                Availability::Redacted
                            } else {
                                Availability::Present
                            },
                        );
                        let source = source_with_resources(
                            vec![observed],
                            Vec::new(),
                            if bind { Vec::new() } else { vec![volume()] },
                            DockerSelection::Explicit,
                        )?
                        .with_promotion_policy(
                            DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
                        );
                        let imported = adapter.import(&source);
                        assert!(
                            imported.application().ok_or("missing application")?.services()[0]
                                .value()
                                .mounts()
                                .is_empty()
                        );
                        assert!(
                            !imported
                                .outcomes()
                                .iter()
                                .any(|outcome| outcome.subject() == "services[0].mounts"
                                    && outcome.kind() == ConversionKind::Exact)
                        );
                        assert!(
                            !imported
                                .diagnostics()
                                .iter()
                                .any(|diagnostic| diagnostic.code().as_str() == "BFD0003"
                                    && diagnostic.fields().iter().any(|field| field.name() == "subject"
                                        && field.value().expose().starts_with("services[0].mounts")))
                        );
                        assert!(!format!("{:?}", imported.diagnostics()).contains("child-private-canary"));
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn rejected_effective_binding_does_not_relabel_a_retained_configured_sibling() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Configured));
        observed.configured_ports = Observed::present(
            vec![PortObservation {
                key: PortKey {
                    container_port: 8080,
                    protocol: TransportProtocol::Tcp,
                },
                bindings: Observed::present(
                    vec![
                        HostBinding {
                            host_ip: observed_image(b"child-private-canary", Origin::Effective),
                            host_port: Observed::present(0, Availability::Present, Origin::Effective),
                        },
                        HostBinding {
                            host_ip: observed_image(b"127.0.0.1", Origin::Configured),
                            host_port: Observed::present(18080, Availability::Present, Origin::Configured),
                        },
                    ],
                    Availability::Present,
                    Origin::Configured,
                ),
            }],
            Availability::Present,
            Origin::Configured,
        );
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()));
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert_eq!(service.ports().len(), 1);
        assert_eq!(service.ports()[0].value().published(), Some(18080));
        assert_value_reconstruction(&service.ports()[0], false);
        assert_exact_reconstruction(&imported, "services[0].configured_ports[0].bindings[1]", false);
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "services[0].configured_ports"
                    && outcome.kind() == ConversionKind::Approximate)
        );
        Ok(())
    }

    #[test]
    fn valued_redacted_port_children_refuse_configured_and_consented_effective_values() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for origin in [Origin::Configured, Origin::Effective] {
            for child in ["outer", "bindings", "host_port", "host_ip"] {
                let mut observed = container(observed_image(b"app:1", Origin::Configured));
                let port = PortObservation {
                    key: PortKey {
                        container_port: 8080,
                        protocol: TransportProtocol::Tcp,
                    },
                    bindings: valued_redacted_child(
                        vec![HostBinding {
                            host_port: valued_redacted_child(0, child == "host_port", origin),
                            host_ip: valued_redacted_child(
                                ProtectedValue::new(b"192.0.2.137".to_vec()),
                                child == "host_ip",
                                origin,
                            ),
                        }],
                        child == "bindings",
                        origin,
                    ),
                };
                observed.configured_ports = valued_redacted_child(vec![port], child == "outer", origin);
                let imported = adapter.import(
                    &source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()),
                );
                assert!(
                    imported.application().ok_or("missing application")?.services()[0]
                        .value()
                        .ports()
                        .is_empty()
                );
                let leaf = match child {
                    "host_port" => "services[0].configured_ports[0].bindings[0].host_port",
                    "host_ip" => "services[0].configured_ports[0].bindings[0].host_ip",
                    "bindings" => "services[0].configured_ports[0]",
                    _ => "services[0].configured_ports",
                };
                assert_refused_valued_redaction(&imported, "services[0].configured_ports", leaf);
            }
        }
        Ok(())
    }

    #[test]
    fn valued_redacted_health_children_refuse_configured_and_consented_effective_values() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for origin in [Origin::Configured, Origin::Effective] {
            for child in [
                "outer",
                "test",
                "interval",
                "timeout",
                "start_period",
                "start_interval",
                "retries",
            ] {
                let mut observed = container(observed_image(b"app:1", Origin::Configured));
                observed.healthcheck = valued_redacted_child(
                    NativeHealthcheck {
                        test: valued_redacted_child(
                            HealthcheckTest::Cmd(vec![ProtectedValue::new(b"valued-private-canary".to_vec())]),
                            child == "test",
                            origin,
                        ),
                        interval_ns: valued_redacted_child(1_000_000_000, child == "interval", origin),
                        timeout_ns: valued_redacted_child(2_000_000_000, child == "timeout", origin),
                        start_period_ns: valued_redacted_child(3_000_000_000, child == "start_period", origin),
                        start_interval_ns: valued_redacted_child(4_000_000_000, child == "start_interval", origin),
                        retries: valued_redacted_child(3, child == "retries", origin),
                    },
                    child == "outer",
                    origin,
                );
                let imported = adapter.import(
                    &source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application()),
                );
                assert!(
                    imported.application().ok_or("missing application")?.services()[0]
                        .value()
                        .healthcheck()
                        .is_none()
                );
                let leaf = if child == "outer" {
                    "services[0].healthcheck".to_owned()
                } else {
                    format!("services[0].healthcheck.{child}")
                };
                assert_refused_valued_redaction(&imported, "services[0].healthcheck", &leaf);
            }
        }
        Ok(())
    }

    #[test]
    fn valued_redacted_mount_children_refuse_configured_and_consented_effective_values() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for bind in [false, true] {
            for origin in [Origin::Configured, Origin::Effective] {
                for child in ["outer", "destination", "read_write", "logical_source"] {
                    let mut observed = container(observed_image(b"app:1", Origin::Configured));
                    let mount = MountObservation {
                        kind: if bind { MountKind::Bind } else { MountKind::Volume },
                        source: if bind {
                            valued_redacted_child(
                                ProtectedValue::new(b"/same-host/valued-private-canary".to_vec()),
                                child == "logical_source",
                                origin,
                            )
                        } else {
                            absent()
                        },
                        destination: valued_redacted_child(
                            ProtectedValue::new(b"/data/valued-private-canary".to_vec()),
                            child == "destination",
                            origin,
                        ),
                        name: if bind {
                            absent()
                        } else {
                            valued_redacted_child(
                                ProtectedValue::new(b"private-volume".to_vec()),
                                child == "logical_source",
                                origin,
                            )
                        },
                        read_write: valued_redacted_child(false, child == "read_write", origin),
                        mode: explicit_empty_mode(Origin::Configured),
                        mode_interpretation: explicit_empty_mode_interpretation(Origin::Configured),
                        propagation: absent(),
                    };
                    observed.mounts = valued_redacted_child(vec![mount], child == "outer", origin);
                    let source = source_with_resources(
                        vec![observed],
                        Vec::new(),
                        if bind { Vec::new() } else { vec![volume()] },
                        DockerSelection::Explicit,
                    )?
                    .with_promotion_policy(
                        DockerPromotionPolicy::portable_application().with_same_host_bind_mounts(true),
                    );
                    let imported = adapter.import(&source);
                    assert!(
                        imported.application().ok_or("missing application")?.services()[0]
                            .value()
                            .mounts()
                            .is_empty()
                    );
                    let leaf = match child {
                        "outer" => "services[0].mounts".to_owned(),
                        "logical_source" => format!("services[0].mounts[0].{}", if bind { "source" } else { "name" }),
                        _ => format!("services[0].mounts[0].{child}"),
                    };
                    assert_refused_valued_redaction(&imported, "services[0].mounts", &leaf);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn redacted_health_child_prevents_complete_reconstruction() -> Result<(), String> {
        let mut observed = container(observed_image(b"app:1", Origin::Effective));
        observed.healthcheck = Observed::present(
            NativeHealthcheck {
                test: Observed::present(
                    HealthcheckTest::Cmd(vec![ProtectedValue::new(b"/bin/true".to_vec())]),
                    Availability::Present,
                    Origin::Effective,
                ),
                interval_ns: Observed::unavailable(Availability::Redacted, Origin::Effective),
                timeout_ns: absent(),
                start_period_ns: absent(),
                start_interval_ns: absent(),
                retries: absent(),
            },
            Availability::Present,
            Origin::Effective,
        );
        let source = source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application());
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        assert!(service.healthcheck().is_none());
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].healthcheck.interval" && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "services[0].healthcheck" && outcome.kind() == ConversionKind::Exact
        }));
        Ok(())
    }

    #[test]
    fn decoded_optional_zero_health_timings_are_retained_with_default_uncertainty() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (period, startup, approximate) in [(0, 1_000_000_000, false), (1_000_000_000, 0, true), (0, 0, true)] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1","Healthcheck":{{"Test":["CMD","/bin/true"],"Interval":2000000000,"Timeout":5000000000,"Retries":60,"StartPeriod":{period},"StartInterval":{startup}}}}}}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let native = source.inventory().containers[0]
                .healthcheck
                .value()
                .ok_or("missing native healthcheck")?;
            assert_eq!(native.start_period_ns.availability, Availability::Present);
            assert_eq!(native.start_interval_ns.availability, Availability::Present);
            assert_eq!(native.start_period_ns.value(), Some(&period));
            assert_eq!(native.start_interval_ns.value(), Some(&startup));
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            let health = service.healthcheck().ok_or("dropped complete healthcheck")?.value();
            assert!(health.command().is_some());
            assert_eq!(
                health.start_period().ok_or("missing grace period")?.value().as_str(),
                format!("{period}ns")
            );
            assert_eq!(
                health
                    .start_interval()
                    .ok_or("missing startup interval")?
                    .value()
                    .as_str(),
                format!("{startup}ns")
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject().starts_with("services[0].healthcheck")
                        && matches!(outcome.kind(), ConversionKind::Unsupported | ConversionKind::Invalid))
            );
            assert_eq!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].healthcheck"
                        && outcome.kind() == ConversionKind::Approximate),
                approximate
            );
            if approximate {
                assert!(
                    !imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].healthcheck"
                            && outcome.kind() == ConversionKind::Exact)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn decoded_shell_healthcheck_retains_zero_startup_timings_without_claiming_exactness() -> Result<(), String> {
        let inspect = r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{"Image":"app:1","Healthcheck":{"Test":["CMD-SHELL","pg_isready -U forgejo -d forgejo"],"Interval":2000000000,"Timeout":5000000000,"Retries":60,"StartPeriod":0,"StartInterval":0}}}"#;
        let source = decoded_source_fixture(inspect, None, None)?
            .with_promotion_policy(DockerPromotionPolicy::portable_application());
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        let service = imported.application().ok_or("missing application")?.services()[0].value();
        let health = service.healthcheck().ok_or("dropped fixture healthcheck")?.value();
        assert!(matches!(
            health.command().map(Sourced::value),
            Some(HealthcheckCommand::Shell(_))
        ));
        assert_eq!(
            health.start_period().ok_or("missing grace period")?.value().as_str(),
            "0ns"
        );
        assert_eq!(
            health
                .start_interval()
                .ok_or("missing startup interval")?
                .value()
                .as_str(),
            "0ns"
        );
        assert_eq!(
            imported
                .outcomes()
                .iter()
                .filter(|outcome| outcome.subject() == "services[0].healthcheck"
                    && outcome.kind() == ConversionKind::Approximate)
                .count(),
            1
        );
        assert!(
            !imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject().starts_with("services[0].healthcheck")
                    && matches!(
                        outcome.kind(),
                        ConversionKind::Unsupported | ConversionKind::Invalid | ConversionKind::Exact
                    ))
        );
        Ok(())
    }

    #[test]
    fn optional_zero_health_defaults_remain_visible_when_required_timings_are_missing_or_null() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for interval in ["", "\"Interval\":null,"] {
            for test in [
                r#"["CMD","/bin/true"]"#,
                r#"["CMD-SHELL","pg_isready -U forgejo -d forgejo"]"#,
            ] {
                let inspect = format!(
                    r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1","Healthcheck":{{"Test":{test},{interval}"Timeout":5000000000,"Retries":60,"StartPeriod":0,"StartInterval":0}}}}}}"#
                );
                let source = decoded_source_fixture(&inspect, None, None)?
                    .with_promotion_policy(DockerPromotionPolicy::portable_application());
                let imported = adapter.import(&source);
                let health = imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .healthcheck()
                    .ok_or("missing partial healthcheck")?
                    .value();
                assert!(health.interval().is_none());
                assert_eq!(
                    health.start_period().ok_or("missing grace period")?.value().as_str(),
                    "0ns"
                );
                assert_eq!(
                    health
                        .start_interval()
                        .ok_or("missing startup interval")?
                        .value()
                        .as_str(),
                    "0ns"
                );
                for kind in [ConversionKind::Unsupported, ConversionKind::Approximate] {
                    assert_eq!(
                        imported
                            .outcomes()
                            .iter()
                            .filter(|outcome| outcome.subject() == "services[0].healthcheck" && outcome.kind() == kind)
                            .count(),
                        1
                    );
                }
                assert!(
                    !imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "services[0].healthcheck"
                            && matches!(outcome.kind(), ConversionKind::Exact | ConversionKind::Invalid))
                );
            }
        }
        Ok(())
    }

    #[test]
    fn optional_zero_health_support_does_not_promote_required_zero_defaults() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (interval, timeout, retries, subject) in [
            (0, 5_000_000_000_u64, 60, "interval"),
            (2_000_000_000_u64, 0, 60, "timeout"),
            (2_000_000_000, 5_000_000_000, 0, "retries"),
        ] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1","Healthcheck":{{"Test":["CMD","/bin/true"],"Interval":{interval},"Timeout":{timeout},"Retries":{retries},"StartPeriod":0,"StartInterval":0}}}}}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .healthcheck()
                    .is_none()
            );
            assert!(imported.outcomes().iter().any(|outcome| outcome.subject()
                == format!("services[0].healthcheck.{subject}")
                && outcome.kind() == ConversionKind::Unsupported));
        }
        Ok(())
    }

    #[test]
    fn optional_zero_health_values_do_not_bypass_redaction_or_conservative_policy() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for child in ["start_period", "start_interval"] {
            let mut observed = container(observed_image(b"app:1", Origin::Effective));
            let mut health = NativeHealthcheck {
                test: Observed::present(
                    HealthcheckTest::Cmd(vec![ProtectedValue::new(b"child-private-canary".to_vec())]),
                    Availability::Present,
                    Origin::Configured,
                ),
                interval_ns: Observed::present(1_000_000_000, Availability::Present, Origin::Configured),
                timeout_ns: Observed::present(2_000_000_000, Availability::Present, Origin::Configured),
                start_period_ns: Observed::present(3_000_000_000, Availability::Present, Origin::Configured),
                start_interval_ns: Observed::present(4_000_000_000, Availability::Present, Origin::Configured),
                retries: Observed::present(3, Availability::Present, Origin::Configured),
            };
            if child == "start_period" {
                health.start_period_ns = valued_redacted_child(0, true, Origin::Effective);
            } else {
                health.start_interval_ns = valued_redacted_child(0, true, Origin::Effective);
            }
            observed.healthcheck = Observed::present(health, Availability::Present, Origin::Configured);
            let source = source(vec![observed])?.with_promotion_policy(DockerPromotionPolicy::portable_application());
            let imported = adapter.import(&source);
            assert!(
                imported.application().ok_or("missing application")?.services()[0]
                    .value()
                    .healthcheck()
                    .is_none()
            );
            assert_refused_valued_redaction(
                &imported,
                "services[0].healthcheck",
                &format!("services[0].healthcheck.{child}"),
            );
        }
        let inspect = r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{"Image":"app:1","Healthcheck":{"Test":["CMD","/bin/true"],"Interval":2000000000,"Timeout":5000000000,"Retries":60,"StartPeriod":0,"StartInterval":0}}}"#;
        let source = decoded_source_fixture(inspect, None, None)?;
        let imported = adapter.import(&source);
        assert!(
            imported.application().ok_or("missing application")?.services()[0]
                .value()
                .healthcheck()
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn decoded_missing_or_null_health_defaults_do_not_claim_exact_mapping() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for (healthcheck, availability) in [
            (r#"{"Test":["CMD","/bin/true"]}"#, Availability::Missing),
            (
                r#"{"Test":["CMD","/bin/true"],"Interval":null,"Timeout":null,"Retries":null,"StartPeriod":1000000000}"#,
                Availability::Null,
            ),
        ] {
            let inspect = format!(
                r#"{{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{{"Image":"app:1","Healthcheck":{healthcheck}}}}}"#
            );
            let source = decoded_source_fixture(&inspect, None, None)?
                .with_promotion_policy(DockerPromotionPolicy::portable_application());
            let observed = source.inventory().containers[0]
                .healthcheck
                .value()
                .ok_or("missing decoded healthcheck")?;
            assert_eq!(observed.interval_ns.availability, availability);
            assert_eq!(observed.timeout_ns.availability, availability);
            assert_eq!(observed.retries.availability, availability);
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("missing application")?.services()[0].value();
            let healthcheck = service.healthcheck().ok_or("missing retained healthcheck")?.value();
            assert!(healthcheck.command().is_some());
            assert_eq!(healthcheck.start_period().is_some(), availability == Availability::Null);
            assert!(imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].healthcheck" && outcome.kind() == ConversionKind::Unsupported
            }));
            assert!(!imported.outcomes().iter().any(|outcome| {
                outcome.subject() == "services[0].healthcheck" && outcome.kind() == ConversionKind::Exact
            }));
        }
        Ok(())
    }

    const CONTEXT_VERSION: &str = r#"{"Version":"29.8.1","ApiVersion":"1.56","MinAPIVersion":"1.41"}"#;
    const CONTEXT_CONTAINER: &str = r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Name":"/context-worker","Config":{"Image":"example.invalid/context-worker:7","Cmd":["context-runner","--single-step"]}}"#;

    // Independently authored public decoder inputs, not a real daemon/root-mode attestation.
    fn decoded_context_inventory(info: Option<&str>, version: &str, inspect: &str) -> Result<DecodedInventory, String> {
        let mut budget = Budget::new(Limits {
            max_requests: 3,
            max_selected_resources: 1,
            max_expansions: 1,
            max_response_bytes: 4096,
            max_total_bytes: 8192,
            max_elapsed: Duration::from_secs(2),
        })
        .map_err(|error| format!("{error:?}"))?;
        let status = HttpStatus::new(200).map_err(|error| format!("{error:?}"))?;
        let api = ApiVersion::new(NonZeroU16::new(1).ok_or("nonzero API major")?, 49);
        budget
            .record_request(ReadRequest::DaemonVersion, None, None)
            .map_err(|error| format!("{error:?}"))?;
        budget
            .read_response(status, version.as_bytes())
            .map_err(|error| format!("{error:?}"))?;
        if let Some(info) = info {
            budget
                .record_request(ReadRequest::DaemonInfo, None, Some(api))
                .map_err(|error| format!("{error:?}"))?;
            budget
                .read_response(status, info.as_bytes())
                .map_err(|error| format!("{error:?}"))?;
        }
        budget.record_selection(1).map_err(|error| format!("{error:?}"))?;
        budget
            .record_request(
                ReadRequest::InspectContainer(NativeId::new("a".repeat(64)).ok_or("literal container identity")?),
                Some(ResourceRef::new(1)),
                Some(api),
            )
            .map_err(|error| format!("{error:?}"))?;
        budget
            .read_response(status, inspect.as_bytes())
            .map_err(|error| format!("{error:?}"))?;
        let capture = budget.into_capture().map_err(|error| format!("{error:?}"))?;
        assert_eq!(capture.route(), docker_lens::evidence::CaptureRoute::CallerAssembled);
        assert_eq!(capture.exchanges().len(), if info.is_some() { 3 } else { 2 });
        decode_capture(&capture).map_err(|error| format!("{error:?}"))
    }

    fn context_source(inventory: DecodedInventory, policy: DockerPromotionPolicy) -> Result<DockerSource, String> {
        Ok(DockerSource::new(
            Identifier::new("context-row").map_err(|error| error.to_string())?,
            inventory,
        )
        .with_selection(DockerSelection::Explicit)
        .with_promotion_policy(policy))
    }

    fn assert_context_neutral_intent(imported: &ImportResult) -> Result<(), String> {
        let application = imported.application().ok_or("portable application")?;
        assert_eq!(application.name().as_str(), "context-row");
        assert_eq!(application.services().len(), 1);
        let service = application.services()[0].value();
        assert_eq!(service.name().as_str(), "docker-service-1");
        assert_eq!(
            service.runtime_name().ok_or("runtime name")?.value().expose(),
            "context-worker"
        );
        assert_eq!(
            service.image().ok_or("image")?.value().as_str(),
            "example.invalid/context-worker:7"
        );
        let Command::Exec(argv) = service.command().ok_or("command")?.value() else {
            return Err("literal exec command expected".into());
        };
        assert_eq!(
            argv.iter()
                .map(boxferry_model::ProtectedString::expose)
                .collect::<Vec<_>>(),
            ["context-runner", "--single-step"]
        );
        Ok(())
    }

    #[test]
    fn decoded_unknown_source_mode_retains_warning_without_changing_portable_intent() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let mut baseline = None;
        // Rootless:false is explicitly synthetic decoder context, not upstream rootful evidence.
        for (info, mode) in [
            (None, DaemonMode::Unknown),
            (
                Some(r#"{"Name":"private-daemon-context-canary","SecurityOptions":["name=seccomp,profile=builtin"]}"#),
                DaemonMode::Unknown,
            ),
            (Some(r#"{"Rootless":false}"#), DaemonMode::Rootful),
            (Some(r#"{"SecurityOptions":["name=rootless"]}"#), DaemonMode::Rootless),
        ] {
            let inventory = decoded_context_inventory(info, CONTEXT_VERSION, CONTEXT_CONTAINER)?;
            assert_eq!(inventory.version.daemon.mode, mode);
            let expected_findings = if mode == DaemonMode::Unknown {
                vec![Finding {
                    severity: NativeSeverity::Warning,
                    code: FindingCode::CapabilityUnknown,
                    resource: None,
                    field: Some(FieldPath::DaemonMode),
                }]
            } else {
                Vec::new()
            };
            assert_eq!(inventory.findings, expected_findings);
            let source = context_source(inventory, DockerPromotionPolicy::portable_application())?;
            let imported = adapter.import(&source);
            assert_context_neutral_intent(&imported)?;
            let outcomes: Vec<_> = imported
                .outcomes()
                .iter()
                .map(|outcome| {
                    (
                        outcome.subject().to_owned(),
                        outcome.kind(),
                        outcome.diagnostic().map(|code| code.as_str().to_owned()),
                    )
                })
                .collect();
            let losses: Vec<_> = imported
                .outcomes()
                .iter()
                .filter(|outcome| outcome.kind() != ConversionKind::Exact)
                .map(|outcome| {
                    (
                        outcome.subject(),
                        outcome.kind(),
                        outcome.diagnostic().map(boxferry_engine::DiagnosticCode::as_str),
                    )
                })
                .collect();
            assert_eq!(
                losses,
                [("services[0].runtime_name", ConversionKind::Approximate, Some("BFD0004"))]
            );
            assert!(
                !outcomes
                    .iter()
                    .any(|(subject, _, _)| subject == "docker.native.DaemonMode")
            );
            if let Some(expected) = &baseline {
                assert_eq!(&outcomes, expected);
            } else {
                baseline = Some(outcomes);
            }
            let context: Vec<_> = imported
                .diagnostics()
                .iter()
                .filter(|diagnostic| {
                    diagnostic
                        .native_finding()
                        .is_some_and(|finding| finding.code() == "CapabilityUnknown")
                })
                .collect();
            assert_eq!(context.len(), usize::from(mode == DaemonMode::Unknown));
            for diagnostic in context {
                assert_eq!(diagnostic.code().as_str(), "BFD0002");
                assert_eq!(diagnostic.severity(), Severity::Warning);
                assert!(diagnostic.summary().contains("execution mode is unknown"));
                let native = diagnostic.native_finding().ok_or("native envelope")?;
                assert_eq!(native.severity(), Severity::Warning);
                assert_eq!(native.stage(), "observation");
                assert_eq!(native.summary(), diagnostic.summary());
                assert_eq!(diagnostic.fields()[0].value().expose(), "docker.native.DaemonMode");
                assert!(!format!("{diagnostic:?}").contains("private-daemon-context-canary"));
                assert!(!format!("{native:?}").contains("private-daemon-context-canary"));
            }
            assert_eq!(source.inventory().version.daemon.mode, mode);
            assert_eq!(source.inventory().findings, expected_findings);
            assert_eq!(
                imported
                    .diagnostics()
                    .iter()
                    .filter(|diagnostic| diagnostic.code().as_str() == "BFD0003")
                    .count(),
                2
            );
        }
        Ok(())
    }

    #[test]
    fn unknown_mode_exception_rejects_native_finding_near_matches() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let base = Finding {
            severity: NativeSeverity::Warning,
            code: FindingCode::CapabilityUnknown,
            resource: None,
            field: Some(FieldPath::DaemonMode),
        };
        for (finding, info, subject, kind) in [
            (
                Finding {
                    code: FindingCode::MissingField,
                    ..base
                },
                None,
                "docker.native.DaemonMode",
                ConversionKind::Unsupported,
            ),
            (
                Finding {
                    field: Some(FieldPath::ApiVersion),
                    ..base
                },
                None,
                "docker.native.ApiVersion",
                ConversionKind::Unsupported,
            ),
            (
                Finding { field: None, ..base },
                None,
                "docker.native.Other",
                ConversionKind::Unsupported,
            ),
            (
                Finding {
                    resource: Some(ResourceRef::new(1)),
                    ..base
                },
                None,
                "docker.native.resources[1].DaemonMode",
                ConversionKind::Unsupported,
            ),
            (
                Finding {
                    severity: NativeSeverity::Error,
                    ..base
                },
                None,
                "docker.native.DaemonMode",
                ConversionKind::Invalid,
            ),
            (
                base,
                Some(r#"{"Rootless":false}"#),
                "docker.native.DaemonMode",
                ConversionKind::Unsupported,
            ),
            (
                base,
                Some(r#"{"SecurityOptions":["name=rootless"]}"#),
                "docker.native.DaemonMode",
                ConversionKind::Unsupported,
            ),
        ] {
            let mut inventory = decoded_context_inventory(info, CONTEXT_VERSION, CONTEXT_CONTAINER)?;
            // Public typed inventories are fallible; deliberately inject inconsistent/nearby findings.
            inventory.findings = vec![finding];
            let source = context_source(inventory, DockerPromotionPolicy::portable_application())?;
            let imported = adapter.import(&source);
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == subject && outcome.kind() == kind)
            );
            let diagnostic = imported
                .diagnostics()
                .iter()
                .find(|diagnostic| diagnostic.native_finding().is_some())
                .ok_or("retained native finding")?;
            assert_eq!(diagnostic.summary(), "DockerLens reported a typed native finding");
            assert_eq!(
                diagnostic.code().as_str(),
                if kind == ConversionKind::Invalid {
                    "BFD0001"
                } else {
                    "BFD0002"
                }
            );
        }
        Ok(())
    }

    #[test]
    fn unknown_mode_does_not_waive_engine_api_or_unmapped_configuration_losses() -> Result<(), String> {
        const UNMAPPED: &str = r#"{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Config":{"Image":"example.invalid/context-worker:7","Cmd":["context-runner","--single-step"],"Env":["TOKEN=private-config-canary"]},"HostConfig":{"Memory":4096,"SecurityOpt":["private-security-canary"]},"Mounts":[{"Type":"bind","Source":"/fake/private-mount-canary","Destination":"/data","RW":true}]}"#;
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        assert_eq!(
            decoded_context_inventory(
                None,
                r#"{"Version":"29.8.1","ApiVersion":"1.48","MinAPIVersion":"1.41"}"#,
                CONTEXT_CONTAINER,
            )
            .err()
            .as_deref(),
            Some("ApiVersionOutOfRange")
        );
        for version in [
            r#"{"ApiVersion":"1.56","MinAPIVersion":"1.41"}"#,
            r#"{"Version":"29.8.1"}"#,
        ] {
            let source = context_source(
                decoded_context_inventory(None, version, CONTEXT_CONTAINER)?,
                DockerPromotionPolicy::portable_application(),
            )?;
            let imported = adapter.import(&source);
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "docker.native.EngineRelease"
                        && outcome.kind() == ConversionKind::Unsupported)
            );
            assert!(
                !imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "docker.native.DaemonMode")
            );
        }
        let source = context_source(
            decoded_context_inventory(
                Some(r#"{"Name":"private-daemon-context-canary"}"#),
                CONTEXT_VERSION,
                UNMAPPED,
            )?,
            DockerPromotionPolicy::portable_application(),
        )?;
        let imported = adapter.import(&source);
        for subject in [
            "services[0].environment[0]",
            "services[0].memory_bytes",
            "services[0].security_options",
            "services[0].mounts[0]",
        ] {
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported),
                "{subject}"
            );
        }
        for canary in [
            "private-config-canary",
            "private-security-canary",
            "private-mount-canary",
            "private-daemon-context-canary",
        ] {
            assert!(!format!("{:?}", imported.diagnostics()).contains(canary));
            for diagnostic in imported.diagnostics() {
                assert!(!diagnostic.summary().contains(canary));
                assert!(
                    diagnostic
                        .fields()
                        .iter()
                        .all(|field| !field.value().expose().contains(canary))
                );
                if let Some(native) = diagnostic.native_finding() {
                    assert!(!format!("{native:?}").contains(canary));
                    assert!(!native.summary().contains(canary));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn unknown_mode_does_not_authorize_conservative_reconstruction() -> Result<(), String> {
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        for info in [
            None,
            Some(r#"{"Rootless":false}"#),
            Some(r#"{"SecurityOptions":["name=rootless"]}"#),
        ] {
            let source = context_source(
                decoded_context_inventory(info, CONTEXT_VERSION, CONTEXT_CONTAINER)?,
                DockerPromotionPolicy::conservative(),
            )?;
            let imported = adapter.import(&source);
            let service = imported.application().ok_or("conservative application")?.services()[0].value();
            assert!(service.image().is_none());
            assert!(service.command().is_none());
            for subject in ["services[0].image", "services[0].command"] {
                assert!(
                    imported
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn source_mode_does_not_choose_target_or_remove_runtime_name_policy_boundary() -> Result<(), String> {
        const DIFFERENT_SOURCE_VERSION: &str = r#"{"Version":"29.7.1","ApiVersion":"1.49","MinAPIVersion":"1.41"}"#;
        let profile = TargetCapabilityCatalog::reviewed()
            .profiles()
            .find(|profile| {
                profile.release().as_str() == "29.8.1"
                    && matches!(profile.identity().build(), EngineBuild::Upstream)
                    && profile.mode() == DaemonMode::Rootful
            })
            .ok_or("reviewed rootful target")?
            .clone();
        let exporter = DockerExporter::new(profile).map_err(|error| error.to_string())?;
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let version = PlatformVersion::new(29, 8, 1);
        let target = TargetProfile::new("docker", version, Some(version)).map_err(|error| error.to_string())?;
        for info in [
            None,
            Some(r#"{"Rootless":false}"#),
            Some(r#"{"SecurityOptions":["name=rootless"]}"#),
        ] {
            let source = context_source(
                decoded_context_inventory(info, DIFFERENT_SOURCE_VERSION, CONTEXT_CONTAINER)?,
                DockerPromotionPolicy::portable_application(),
            )?;
            let exact = convert_imported(adapter.import(&source), &exporter, &target, LossPolicy::ExactOnly)
                .map_err(|error| error.to_string())?;
            assert!(exact.is_blocked());
            assert!(exact.output().is_none());
            assert!(
                exact
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services[0].runtime_name"
                        && outcome.kind() == ConversionKind::Approximate)
            );
            let approximate = convert_imported(
                adapter.import(&source),
                &exporter,
                &target,
                LossPolicy::AllowApproximate,
            )
            .map_err(|error| error.to_string())?;
            assert!(!approximate.is_blocked());
            let bytes = approximate
                .output()
                .ok_or("authorized native review artifact")?
                .complete_bytes()
                .map_err(|error| format!("{error:?}"))?;
            let artifact: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            assert_eq!(artifact["context"]["kind"], "target");
            assert_eq!(artifact["context"]["engine_release"], "29.8.1");
            assert_eq!(artifact["context"]["daemon_mode"], "rootful");
            assert_eq!(artifact["context"]["advertised_api_version"], "1.56");
            assert_eq!(artifact["context"]["acquisition_api_version"], "1.49");
            assert_eq!(artifact["context"]["rendering_api_version"], "1.56");
            assert_eq!(
                artifact["context"]["evidence_sha256"],
                "b0825b797f052257f706b0102c5c720e7fcbbaef0e585869bb6d23d2e756fb94"
            );
            assert_eq!(exporter.native_profile().mode(), DaemonMode::Rootful);
            let wrong_version = PlatformVersion::new(29, 8, 2);
            let wrong_target =
                TargetProfile::new("docker", wrong_version, Some(wrong_version)).map_err(|error| error.to_string())?;
            let refused = convert_imported(
                adapter.import(&source),
                &exporter,
                &wrong_target,
                LossPolicy::AllowPartial,
            )
            .map_err(|error| error.to_string())?;
            assert!(refused.is_blocked());
            assert!(refused.output().is_none());
            assert!(
                refused
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "target.docker" && outcome.kind() == ConversionKind::Invalid)
            );
        }
        Ok(())
    }

    fn authored_resource_and_global_findings_source() -> Result<DockerSource, String> {
        let mut first = container(observed_image(b"private-first-image-canary", Origin::Configured));
        first.reference = ResourceRef::new(7);
        first.name = observed_image(b"/private-first-name-canary", Origin::Effective);
        let mut second = container(observed_image(b"private-second-image-canary", Origin::Configured));
        second.reference = ResourceRef::new(11);
        second.id = observed_image(
            b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Origin::RuntimeAssigned,
        );
        second.name = observed_image(b"/private-second-name-canary", Origin::Effective);
        source_with_findings(
            vec![first, second],
            Vec::new(),
            Vec::new(),
            DockerSelection::Explicit,
            vec![
                Finding {
                    severity: NativeSeverity::Warning,
                    code: FindingCode::UnsupportedValue,
                    resource: Some(ResourceRef::new(7)),
                    field: Some(FieldPath::Image),
                },
                Finding {
                    severity: NativeSeverity::Error,
                    code: FindingCode::NativeConflict,
                    resource: Some(ResourceRef::new(11)),
                    field: Some(FieldPath::Image),
                },
                Finding {
                    severity: NativeSeverity::Warning,
                    code: FindingCode::MissingField,
                    resource: None,
                    field: Some(FieldPath::Image),
                },
            ],
        )
    }

    #[test]
    fn native_findings_preserve_local_resource_correlation_without_claiming_a_phase() -> Result<(), String> {
        let source = authored_resource_and_global_findings_source()?;
        let imported = DockerImporter::new()
            .map_err(|error| error.to_string())?
            .import(&source);
        let mut native_rows = Vec::new();
        for diagnostic in imported.diagnostics() {
            if let Some(native) = diagnostic.native_finding() {
                let subject = diagnostic
                    .fields()
                    .iter()
                    .find(|field| field.name() == "subject")
                    .ok_or("native finding must retain its subject")?
                    .value()
                    .expose();
                native_rows.push((
                    subject,
                    diagnostic.code().as_str(),
                    diagnostic.severity(),
                    native.code(),
                    native.severity(),
                    native.stage(),
                ));
            }
        }
        assert_eq!(
            native_rows,
            [
                (
                    "docker.native.resources[7].Image",
                    "BFD0002",
                    Severity::Warning,
                    "UnsupportedValue",
                    Severity::Warning,
                    "observation"
                ),
                (
                    "docker.native.resources[11].Image",
                    "BFD0001",
                    Severity::Error,
                    "NativeConflict",
                    Severity::Error,
                    "observation"
                ),
                (
                    "docker.native.Image",
                    "BFD0002",
                    Severity::Warning,
                    "MissingField",
                    Severity::Warning,
                    "observation"
                ),
            ]
        );
        let native_losses: Vec<_> = imported
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject().starts_with("docker.native."))
            .map(|outcome| {
                (
                    outcome.subject(),
                    outcome.kind(),
                    outcome.diagnostic().map(boxferry_engine::DiagnosticCode::as_str),
                )
            })
            .collect();
        assert_eq!(
            native_losses,
            [
                (
                    "docker.native.resources[7].Image",
                    ConversionKind::Unsupported,
                    Some("BFD0002")
                ),
                (
                    "docker.native.resources[11].Image",
                    ConversionKind::Invalid,
                    Some("BFD0001")
                ),
                ("docker.native.Image", ConversionKind::Unsupported, Some("BFD0002")),
            ]
        );
        let diagnostics = format!("{:?}", imported.diagnostics());
        for protected in [
            "private-first-image-canary",
            "private-second-image-canary",
            "/private-first-name-canary",
            "/private-second-name-canary",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ] {
            assert!(!diagnostics.contains(protected));
        }
        Ok(())
    }

    #[test]
    fn native_finding_keeps_typed_code_and_value_free_loss() -> Result<(), String> {
        let source = source_with_findings(
            vec![container(observed_image(b"private.registry/app:1", Origin::Effective))],
            Vec::new(),
            Vec::new(),
            DockerSelection::Explicit,
            vec![Finding {
                severity: NativeSeverity::Warning,
                code: FindingCode::UnsupportedValue,
                resource: Some(ResourceRef::new(1)),
                field: Some(FieldPath::Environment { index: 0 }),
            }],
        )?;
        let adapter = DockerImporter::new().map_err(|error| error.to_string())?;
        let imported = adapter.import(&source);
        assert!(imported.diagnostics().iter().any(|diagnostic| {
            diagnostic
                .native_finding()
                .is_some_and(|native| native.code() == "UnsupportedValue")
                && diagnostic.code().as_str() == "BFD0002"
        }));
        assert!(imported.outcomes().iter().any(|outcome| {
            outcome.subject() == "docker.native.resources[1].Environment { index: 0 }"
                && outcome.kind() == ConversionKind::Unsupported
        }));
        assert!(!format!("{imported:?}").contains("private.registry"));
        Ok(())
    }
}
