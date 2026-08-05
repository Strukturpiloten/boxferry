//! Pure mapping from effective runtime observations to application intent.

use std::collections::BTreeSet;

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    Application, Command, EnvironmentValue, EnvironmentVariable, Healthcheck, MetadataLabel, ModelError, MountSource,
    Network, ProtectedString, Provenance, ResourceOwnership, Service, ServiceGroup, SourceId, Sourced, Volume,
};

use crate::{
    ContainerObservation, EffectiveCommand, ImageObservation, PodObservation, RuntimeEnvironmentVariable,
    RuntimeHealthcheck, RuntimeMetadataLabel, RuntimeResolutions, RuntimeSnapshot,
};

/// Caller-selected treatment of effective values that may originate from image defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OverrideReconstruction {
    /// Preserve every supported effective value as an explicit neutral-model value.
    PreserveObservedState,
    /// Compare effective container values with linked image defaults and retain inferred overrides.
    InferImageOverrides,
}

/// Pure runtime-snapshot importer with an explicit override-reconstruction policy.
///
/// This importer performs no inspection itself. Docker and Podman adapters must construct a
/// [`RuntimeSnapshot`] from effective native responses, use stable redacted source identities,
/// and supply complete observed collections explicitly. Runtime reconstruction always reports
/// application-level uncertainty because effective state cannot prove the original definition.
#[derive(Clone, Debug)]
pub struct RuntimeImporter {
    override_reconstruction: OverrideReconstruction,
    resolutions: RuntimeResolutions,
    codes: Codes,
}

impl RuntimeImporter {
    /// Creates an importer with an explicit override-reconstruction policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only when a code embedded in this adapter is invalid.
    pub fn new(override_reconstruction: OverrideReconstruction) -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            override_reconstruction,
            resolutions: RuntimeResolutions::new(),
            codes: Codes {
                reconstruction_uncertain: DiagnosticCode::new("BFR0001")?,
                inferred_override: DiagnosticCode::new("BFR0002")?,
                comparison_incomplete: DiagnosticCode::new("BFR0003")?,
                ownership_uncertain: DiagnosticCode::new("BFR0004")?,
                pod_relationship: DiagnosticCode::new("BFR0005")?,
                image_missing: DiagnosticCode::new("BFR0006")?,
                invalid_model: DiagnosticCode::new("BFR0007")?,
                group_relationship_conflict: DiagnosticCode::new("BFR0008")?,
                lifecycle_resolution: DiagnosticCode::new("BFR0009")?,
                runtime_managed_metadata: DiagnosticCode::new("BFR0010")?,
            },
        })
    }

    /// Returns the caller-selected override-reconstruction policy.
    #[must_use]
    pub const fn override_reconstruction(&self) -> OverrideReconstruction {
        self.override_reconstruction
    }

    /// Applies finite caller-owned lifecycle resolutions during reconstruction.
    #[must_use]
    pub fn with_resolutions(mut self, resolutions: RuntimeResolutions) -> Self {
        self.resolutions = resolutions;
        self
    }

    /// Returns the exact caller-owned lifecycle resolutions used by this importer.
    #[must_use]
    pub const fn resolutions(&self) -> &RuntimeResolutions {
        &self.resolutions
    }
}

impl ImportAdapter for RuntimeImporter {
    type Source = RuntimeSnapshot;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let mut mapping = Mapping::new(&self.codes, self.override_reconstruction, &self.resolutions, source);
        let mut application = Application::new(source.application_name().clone());

        mapping.report_reconstruction_uncertainty();
        mapping.map_resources(&mut application);
        for container in source.containers() {
            mapping.map_container(&mut application, container);
        }
        mapping.map_service_groups(&mut application);

        ImportResult::new(Some(application), mapping.outcomes, mapping.diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    reconstruction_uncertain: DiagnosticCode,
    inferred_override: DiagnosticCode,
    comparison_incomplete: DiagnosticCode,
    ownership_uncertain: DiagnosticCode,
    pod_relationship: DiagnosticCode,
    image_missing: DiagnosticCode,
    invalid_model: DiagnosticCode,
    group_relationship_conflict: DiagnosticCode,
    lifecycle_resolution: DiagnosticCode,
    runtime_managed_metadata: DiagnosticCode,
}

#[derive(Clone, Copy)]
struct LossExplanation<'a> {
    summary: &'static str,
    reason: &'a str,
    action: &'static str,
}

impl<'a> LossExplanation<'a> {
    const fn new(summary: &'static str, reason: &'a str, action: &'static str) -> Self {
        Self {
            summary,
            reason,
            action,
        }
    }
}

struct Mapping<'a> {
    codes: &'a Codes,
    override_reconstruction: OverrideReconstruction,
    resolutions: &'a RuntimeResolutions,
    source: &'a RuntimeSnapshot,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Mapping<'a> {
    const fn new(
        codes: &'a Codes,
        override_reconstruction: OverrideReconstruction,
        resolutions: &'a RuntimeResolutions,
        source: &'a RuntimeSnapshot,
    ) -> Self {
        Self {
            codes,
            override_reconstruction,
            resolutions,
            source,
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn report_reconstruction_uncertainty(&mut self) {
        let mut origins = Vec::new();
        for container in self.source.containers() {
            origins.push(runtime_origin(container.source_id()));
            if let Some(evidence) = container.creation_evidence() {
                origins.push(runtime_origin(evidence.source_id()));
            }
        }
        origins.extend(
            self.source
                .networks()
                .iter()
                .map(|network| runtime_origin(network.source_id())),
        );
        origins.extend(
            self.source
                .volumes()
                .iter()
                .map(|volume| runtime_origin(volume.source_id())),
        );
        for pod in self.source.pods() {
            origins.push(runtime_origin(pod.source_id()));
            if let Some(evidence) = pod.creation_evidence() {
                origins.push(runtime_origin(evidence.source_id()));
            }
        }
        self.loss(
            self.codes.reconstruction_uncertain.clone(),
            "application.reconstruction",
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime inspection cannot prove the original authored definition",
                "effective state may contain image defaults, runtime defaults, generated values, or later changes",
                "review the reconstructed definition and its field-level decisions before deployment",
            ),
            origins,
        );
    }

    fn map_resources(&mut self, application: &mut Application) {
        let mut networks = BTreeSet::new();
        for network in self.source.networks() {
            networks.insert(network.name().as_str().to_owned());
            let origin = runtime_origin(network.source_id());
            self.add_network(application, network.name().clone(), origin, false);
        }

        let mut volumes = BTreeSet::new();
        for volume in self.source.volumes() {
            volumes.insert(volume.name().as_str().to_owned());
            let origin = runtime_origin(volume.source_id());
            self.add_volume(application, volume.name().clone(), origin, false);
        }

        for container in self.source.containers() {
            for attachment in container.networks() {
                if networks.insert(attachment.network().as_str().to_owned()) {
                    self.add_network(
                        application,
                        attachment.network().clone(),
                        runtime_origin(container.source_id()),
                        true,
                    );
                }
            }
            for mount in container.mounts() {
                let MountSource::Volume(name) = mount.source() else {
                    continue;
                };
                if volumes.insert(name.as_str().to_owned()) {
                    self.add_volume(application, name.clone(), runtime_origin(container.source_id()), true);
                }
            }
        }
    }

    fn add_network(
        &mut self,
        application: &mut Application,
        name: boxferry_model::Identifier,
        origin: Provenance,
        synthesized: bool,
    ) {
        let subject = format!("networks.{}", name.as_str());
        let resolution = self.resolutions.network_ownership(&name);
        let ownership = resolution.map_or(ResourceOwnership::Uncertain, |value| *value.value());
        let mut origins = vec![origin.clone()];
        if let Some(resolution) = resolution {
            origins.extend_from_slice(resolution.origins());
        }
        let network = sourced_with_origins(Network::new(name, ownership), origins.clone());
        if let Err(error) = application.add_network(network) {
            self.invalid_model(&subject, &error, origins);
            return;
        }
        if resolution.is_some() {
            self.lifecycle_resolution(subject, "network", synthesized, origins);
            return;
        }
        let reason = if synthesized {
            "a container relationship referenced this network, but the snapshot did not include its own inspection"
        } else {
            "inspection proves that the network exists but not whether generated definitions should create or reuse it"
        };
        self.loss(
            self.codes.ownership_uncertain.clone(),
            subject,
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime resource lifecycle ownership is uncertain",
                reason,
                "choose application-owned or external lifecycle before generating deployable output",
            ),
            vec![origin],
        );
    }

    fn add_volume(
        &mut self,
        application: &mut Application,
        name: boxferry_model::Identifier,
        origin: Provenance,
        synthesized: bool,
    ) {
        let subject = format!("volumes.{}", name.as_str());
        let resolution = self.resolutions.volume_ownership(&name);
        let ownership = resolution.map_or(ResourceOwnership::Uncertain, |value| *value.value());
        let mut origins = vec![origin.clone()];
        if let Some(resolution) = resolution {
            origins.extend_from_slice(resolution.origins());
        }
        let volume = sourced_with_origins(Volume::new(name, ownership), origins.clone());
        if let Err(error) = application.add_volume(volume) {
            self.invalid_model(&subject, &error, origins);
            return;
        }
        if resolution.is_some() {
            self.lifecycle_resolution(subject, "volume", synthesized, origins);
            return;
        }
        let reason = if synthesized {
            "a container relationship referenced this volume, but the snapshot did not include its own inspection"
        } else {
            "inspection proves that the volume exists but not whether generated definitions should create or reuse it"
        };
        self.loss(
            self.codes.ownership_uncertain.clone(),
            subject,
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime resource lifecycle ownership is uncertain",
                reason,
                "choose application-owned or external lifecycle before generating deployable output",
            ),
            vec![origin],
        );
    }

    fn map_container(&mut self, application: &mut Application, container: &ContainerObservation) {
        let service_name = container.name().as_str();
        let service_subject = format!("services.{service_name}");
        let container_origin = runtime_origin(container.source_id());
        let mut service = Service::new(container.name().clone());

        if let Some(image) = container.image() {
            service.set_image(Sourced::from_source(image.clone(), container_origin.clone()));
            self.exact(format!("{service_subject}.image"), vec![container_origin.clone()]);
        } else {
            self.loss(
                self.codes.image_missing.clone(),
                format!("{service_subject}.image"),
                ConversionKind::Unsupported,
                LossExplanation::new(
                    "runtime container has no reconstructable image reference",
                    "the supplied observation did not contain an image reference suitable for a reusable definition",
                    "supply a reviewed image reference or rebuild the container from an authored source definition",
                ),
                vec![container_origin.clone()],
            );
        }

        match self.override_reconstruction {
            OverrideReconstruction::PreserveObservedState => {
                self.preserve_effective_values(&mut service, container, &service_subject);
            }
            OverrideReconstruction::InferImageOverrides => {
                self.infer_overrides(&mut service, container, &service_subject);
            }
        }

        if let Some(restart_policy) = container.restart_policy() {
            service.set_restart_policy(Sourced::from_source(restart_policy, container_origin.clone()));
            self.exact(
                format!("{service_subject}.restart_policy"),
                vec![container_origin.clone()],
            );
        }

        if let Some(read_only) = container.read_only_root_filesystem() {
            service.set_read_only_root_filesystem(Sourced::from_source(read_only, container_origin.clone()));
            self.exact(
                format!("{service_subject}.read_only_root_filesystem"),
                vec![container_origin.clone()],
            );
        }

        for (index, port) in container.ports().iter().enumerate() {
            service.add_port(Sourced::from_source(port.clone(), container_origin.clone()));
            self.exact(
                format!("{service_subject}.ports[{index}]"),
                vec![container_origin.clone()],
            );
        }
        for (index, mount) in container.mounts().iter().enumerate() {
            service.add_mount(Sourced::from_source(mount.clone(), container_origin.clone()));
            self.exact(
                format!("{service_subject}.mounts[{index}]"),
                vec![container_origin.clone()],
            );
        }
        for attachment in container.networks() {
            let subject = format!("{service_subject}.networks.{}", attachment.network().as_str());
            service.add_network(Sourced::from_source(attachment.clone(), container_origin.clone()));
            self.exact(subject, vec![container_origin.clone()]);
        }

        let sourced_service = Sourced::from_source(service, container_origin.clone());
        if let Err(error) = application.add_service(sourced_service) {
            self.invalid_model(&service_subject, &error, vec![container_origin]);
        }
    }

    fn preserve_effective_values(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        service_subject: &str,
    ) {
        let origin = runtime_origin(container.source_id());
        if let Some(command) = container.command() {
            service.set_command(Sourced::from_source(neutral_command(command), origin.clone()));
            self.exact(format!("{service_subject}.command"), vec![origin.clone()]);
        } else {
            self.comparison_incomplete(
                format!("{service_subject}.command"),
                "the effective container command was not supplied",
                vec![origin.clone()],
            );
        }

        if let Some(environment) = container.environment() {
            for variable in environment {
                service.add_environment(Sourced::from_source(neutral_environment(variable), origin.clone()));
                self.exact(
                    format!("{service_subject}.environment.{}", variable.name().as_str()),
                    vec![origin.clone()],
                );
            }
        } else {
            self.comparison_incomplete(
                format!("{service_subject}.environment"),
                "the effective container environment was not supplied",
                vec![origin.clone()],
            );
        }

        if let Some(labels) = container.labels() {
            for label in labels {
                service.add_label(Sourced::from_source(neutral_label(label), origin.clone()));
                let subject = format!("{service_subject}.labels.{}", label.name().as_str());
                if is_compose_managed_label(label) {
                    self.runtime_managed_metadata(subject, vec![origin.clone()]);
                } else {
                    self.exact(subject, vec![origin.clone()]);
                }
            }
        } else {
            self.comparison_incomplete(
                format!("{service_subject}.labels"),
                "the effective container metadata labels were not supplied",
                vec![origin.clone()],
            );
        }

        if let Some(user) = container.user() {
            set_neutral_identity(service, &Sourced::from_source(user.clone(), origin.clone()));
            self.exact(format!("{service_subject}.user"), vec![origin.clone()]);
            if identity_group(user).is_some() {
                self.exact(format!("{service_subject}.group"), vec![origin.clone()]);
            }
        }
        if let Some(working_directory) = container.working_directory() {
            service.set_working_directory(Sourced::from_source(working_directory.clone(), origin.clone()));
            self.exact(format!("{service_subject}.working_directory"), vec![origin.clone()]);
        }
        if let Some(healthcheck) = container.healthcheck().filter(|healthcheck| !healthcheck.is_empty()) {
            service.set_healthcheck(Sourced::from_source(
                neutral_healthcheck(healthcheck, std::slice::from_ref(&origin)),
                origin.clone(),
            ));
            self.report_exact_healthcheck(service_subject, healthcheck, &origin);
        }
    }

    fn infer_overrides(&mut self, service: &mut Service, container: &ContainerObservation, service_subject: &str) {
        let image = container
            .image_source_id()
            .and_then(|source_id| self.source.images().iter().find(|image| image.source_id() == source_id));
        let Some(image) = image else {
            self.preserve_without_image_defaults(service, container, service_subject);
            return;
        };

        self.infer_command(service, container, image, service_subject);
        self.infer_environment(service, container, image, service_subject);
        self.infer_labels(service, container, image, service_subject);
        self.infer_identity(service, container, image, service_subject);
        self.infer_healthcheck(service, container, image, service_subject);
        self.infer_protected_override(
            format!("{service_subject}.working_directory"),
            container.working_directory(),
            image.working_directory(),
            container,
            image,
            |value| service.set_working_directory(value),
        );
    }

    fn infer_healthcheck(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        image: &ImageObservation,
        service_subject: &str,
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let origins = vec![container_origin, image_origin, decision_origin];

        let (container_healthcheck, image_healthcheck) = match (container.healthcheck(), image.healthcheck()) {
            (Some(container_healthcheck), Some(image_healthcheck)) => (container_healthcheck, image_healthcheck),
            (Some(container_healthcheck), None) if !container_healthcheck.is_empty() => {
                service.set_healthcheck(sourced_with_origins(
                    neutral_healthcheck(container_healthcheck, &origins),
                    origins.clone(),
                ));
                self.comparison_incomplete(
                    format!("{service_subject}.healthcheck"),
                    "image health-check data was not supplied for comparison",
                    origins,
                );
                return;
            }
            (None, Some(image_healthcheck)) if !image_healthcheck.is_empty() => {
                self.comparison_incomplete(
                    format!("{service_subject}.healthcheck"),
                    "effective container health-check data was not supplied for comparison",
                    origins,
                );
                return;
            }
            _ => return,
        };

        let mut retained = RuntimeHealthcheck::new();
        if container_healthcheck.disabled() == Some(true) {
            self.infer_health_bool_field(
                format!("{service_subject}.healthcheck.disable"),
                container_healthcheck.disabled(),
                image_healthcheck.disabled(),
                &origins,
                |value| retained.set_disabled(value),
            );
            if !retained.is_empty() {
                service.set_healthcheck(sourced_with_origins(neutral_healthcheck(&retained, &origins), origins));
            }
            return;
        }
        self.infer_health_field(
            format!("{service_subject}.healthcheck.test"),
            container_healthcheck.command(),
            image_healthcheck.command(),
            &origins,
            |value| retained.set_command(value),
        );
        self.infer_health_bool_field(
            format!("{service_subject}.healthcheck.disable"),
            container_healthcheck.disabled(),
            image_healthcheck.disabled(),
            &origins,
            |value| retained.set_disabled(value),
        );
        self.infer_health_field(
            format!("{service_subject}.healthcheck.interval"),
            container_healthcheck.interval(),
            image_healthcheck.interval(),
            &origins,
            |value| retained.set_interval(value),
        );
        self.infer_health_field(
            format!("{service_subject}.healthcheck.timeout"),
            container_healthcheck.timeout(),
            image_healthcheck.timeout(),
            &origins,
            |value| retained.set_timeout(value),
        );
        self.infer_health_field(
            format!("{service_subject}.healthcheck.retries"),
            container_healthcheck.retries(),
            image_healthcheck.retries(),
            &origins,
            |value| retained.set_retries(value),
        );
        self.infer_health_field(
            format!("{service_subject}.healthcheck.start_period"),
            container_healthcheck.start_period(),
            image_healthcheck.start_period(),
            &origins,
            |value| retained.set_start_period(value),
        );
        self.infer_health_field(
            format!("{service_subject}.healthcheck.start_interval"),
            container_healthcheck.start_interval(),
            image_healthcheck.start_interval(),
            &origins,
            |value| retained.set_start_interval(value),
        );

        if !retained.is_empty() {
            service.set_healthcheck(sourced_with_origins(neutral_healthcheck(&retained, &origins), origins));
        }
    }

    fn infer_health_field<T: Clone + Eq>(
        &mut self,
        subject: String,
        container_value: Option<&T>,
        image_value: Option<&T>,
        origins: &[Provenance],
        retain: impl FnOnce(T),
    ) {
        match (container_value, image_value) {
            (Some(value), image_value) => {
                let matches_default = image_value == Some(value);
                if !matches_default {
                    retain(value.clone());
                }
                self.inferred_override(subject, matches_default, origins.to_vec());
            }
            (None, Some(_)) => self.comparison_incomplete(
                subject,
                "an image health-check default is absent from the effective container observation",
                origins.to_vec(),
            ),
            (None, None) => {}
        }
    }

    fn infer_health_bool_field(
        &mut self,
        subject: String,
        container_value: Option<bool>,
        image_value: Option<bool>,
        origins: &[Provenance],
        retain: impl FnOnce(bool),
    ) {
        match (container_value, image_value) {
            (Some(value), image_value) => {
                let matches_default = image_value == Some(value);
                if !matches_default {
                    retain(value);
                }
                self.inferred_override(subject, matches_default, origins.to_vec());
            }
            (None, Some(_)) => self.comparison_incomplete(
                subject,
                "an image health-check default is absent from the effective container observation",
                origins.to_vec(),
            ),
            (None, None) => {}
        }
    }

    fn report_exact_healthcheck(
        &mut self,
        service_subject: &str,
        healthcheck: &RuntimeHealthcheck,
        origin: &Provenance,
    ) {
        for field in [
            healthcheck.command().map(|_| "test"),
            healthcheck.disabled().map(|_| "disable"),
            healthcheck.interval().map(|_| "interval"),
            healthcheck.timeout().map(|_| "timeout"),
            healthcheck.retries().map(|_| "retries"),
            healthcheck.start_period().map(|_| "start_period"),
            healthcheck.start_interval().map(|_| "start_interval"),
        ]
        .into_iter()
        .flatten()
        {
            self.exact(format!("{service_subject}.healthcheck.{field}"), vec![origin.clone()]);
        }
    }

    fn infer_command(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        image: &ImageObservation,
        service_subject: &str,
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let origins = vec![container_origin.clone(), image_origin.clone(), decision_origin.clone()];

        let (Some(command), Some(image_command)) = (container.command(), image.command()) else {
            if let Some(command) = container.command() {
                service.set_command(sourced_with_origins(neutral_command(command), origins.clone()));
            }
            self.comparison_incomplete(
                format!("{service_subject}.command"),
                "container or image command data was not supplied for comparison",
                origins,
            );
            return;
        };

        if command != image_command {
            service.set_command(sourced_with_origins(neutral_command(command), origins.clone()));
        }
        self.inferred_override(format!("{service_subject}.command"), command == image_command, origins);
    }

    fn infer_environment(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        image: &ImageObservation,
        service_subject: &str,
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let (Some(environment), Some(image_environment)) = (container.environment(), image.environment()) else {
            if let Some(environment) = container.environment() {
                for variable in environment {
                    service.add_environment(sourced_with_origins(
                        neutral_environment(variable),
                        vec![container_origin.clone(), decision_origin.clone()],
                    ));
                }
            }
            self.comparison_incomplete(
                format!("{service_subject}.environment"),
                "container or image environment data was not supplied for comparison",
                vec![container_origin, image_origin, decision_origin],
            );
            return;
        };

        for variable in environment {
            let image_variable = image_environment
                .iter()
                .rev()
                .find(|candidate| candidate.name() == variable.name());
            let matches_default = image_variable == Some(variable);
            let origins = vec![container_origin.clone(), image_origin.clone(), decision_origin.clone()];
            if !matches_default {
                service.add_environment(sourced_with_origins(neutral_environment(variable), origins.clone()));
            }
            self.inferred_override(
                format!("{service_subject}.environment.{}", variable.name().as_str()),
                matches_default,
                origins,
            );
        }

        for image_variable in image_environment {
            if environment
                .iter()
                .all(|variable| variable.name() != image_variable.name())
            {
                self.loss(
                    self.codes.comparison_incomplete.clone(),
                    format!("{service_subject}.environment.{}", image_variable.name().as_str()),
                    ConversionKind::Unsupported,
                    LossExplanation::new(
                        "an image environment default is absent from the effective container environment",
                        "inspection cannot establish how the image default was removed",
                        "review whether the generated definition needs an explicit target-specific unset operation",
                    ),
                    vec![container_origin.clone(), image_origin.clone(), decision_origin.clone()],
                );
            }
        }
    }

    fn infer_labels(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        image: &ImageObservation,
        service_subject: &str,
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let (Some(labels), Some(image_labels)) = (container.labels(), image.labels()) else {
            if let Some(labels) = container.labels() {
                for label in labels {
                    service.add_label(sourced_with_origins(
                        neutral_label(label),
                        vec![container_origin.clone(), decision_origin.clone()],
                    ));
                    if is_compose_managed_label(label) {
                        self.runtime_managed_metadata(
                            format!("{service_subject}.labels.{}", label.name().as_str()),
                            vec![container_origin.clone(), decision_origin.clone()],
                        );
                    }
                }
            }
            self.comparison_incomplete(
                format!("{service_subject}.labels"),
                "container or image metadata-label data was not supplied for comparison",
                vec![container_origin, image_origin, decision_origin],
            );
            return;
        };

        for label in labels {
            let image_label = image_labels.iter().find(|candidate| candidate.name() == label.name());
            let matches_default = image_label == Some(label);
            let origins = vec![container_origin.clone(), image_origin.clone(), decision_origin.clone()];
            if !matches_default {
                service.add_label(sourced_with_origins(neutral_label(label), origins.clone()));
            }
            let subject = format!("{service_subject}.labels.{}", label.name().as_str());
            if !matches_default && is_compose_managed_label(label) {
                self.runtime_managed_metadata(subject, origins);
            } else {
                self.inferred_override(subject, matches_default, origins);
            }
        }

        for image_label in image_labels {
            if labels.iter().all(|label| label.name() != image_label.name()) {
                self.loss(
                    self.codes.comparison_incomplete.clone(),
                    format!("{service_subject}.labels.{}", image_label.name().as_str()),
                    ConversionKind::Unsupported,
                    LossExplanation::new(
                        "an image metadata-label default is absent from the effective container labels",
                        "inspection cannot establish how the inherited image label was removed",
                        "review whether generated output needs an explicit target-specific empty or replacement label",
                    ),
                    vec![container_origin.clone(), image_origin.clone(), decision_origin.clone()],
                );
            }
        }
    }

    fn infer_protected_override(
        &mut self,
        subject: String,
        container_value: Option<&ProtectedString>,
        image_value: Option<&ProtectedString>,
        container: &ContainerObservation,
        image: &ImageObservation,
        set_value: impl FnOnce(Sourced<ProtectedString>),
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let origins = vec![container_origin, image_origin, decision_origin];

        match (container_value, image_value) {
            (Some(value), image_value) => {
                let matches_default = image_value == Some(value);
                if !matches_default {
                    set_value(sourced_with_origins(value.clone(), origins.clone()));
                }
                self.inferred_override(subject, matches_default, origins);
            }
            (None, Some(_)) => self.loss(
                self.codes.comparison_incomplete.clone(),
                subject,
                ConversionKind::Unsupported,
                LossExplanation::new(
                    "an image default is absent from the effective container observation",
                    "inspection cannot establish whether the value was cleared or omitted by the native response",
                    "review whether generated output needs an explicit target-specific reset",
                ),
                origins,
            ),
            (None, None) => {}
        }
    }

    fn infer_identity(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        image: &ImageObservation,
        service_subject: &str,
    ) {
        let container_origin = runtime_origin(container.source_id());
        let image_origin = runtime_origin(image.source_id());
        let decision_origin = decision_origin(container.source_id());
        let origins = vec![container_origin, image_origin, decision_origin];

        match (container.user(), image.user()) {
            (Some(value), image_value) => {
                let matches_default = image_value == Some(value);
                if !matches_default {
                    set_neutral_identity(service, &sourced_with_origins(value.clone(), origins.clone()));
                }
                self.inferred_override(format!("{service_subject}.user"), matches_default, origins.clone());
                if identity_group(value).is_some() || image_value.and_then(identity_group).is_some() {
                    self.inferred_override(format!("{service_subject}.group"), matches_default, origins);
                }
            }
            (None, Some(image_value)) => {
                for field in if identity_group(image_value).is_some() {
                    [Some("user"), Some("group")]
                } else {
                    [Some("user"), None]
                }
                .into_iter()
                .flatten()
                {
                    self.loss(
                        self.codes.comparison_incomplete.clone(),
                        format!("{service_subject}.{field}"),
                        ConversionKind::Unsupported,
                        LossExplanation::new(
                            "an image identity default is absent from the effective container observation",
                            "inspection cannot establish whether the identity was cleared or omitted by the native response",
                            "review whether generated output needs an explicit target-specific identity reset",
                        ),
                        origins.clone(),
                    );
                }
            }
            (None, None) => {}
        }
    }

    fn preserve_without_image_defaults(
        &mut self,
        service: &mut Service,
        container: &ContainerObservation,
        service_subject: &str,
    ) {
        let runtime_origin = runtime_origin(container.source_id());
        let decision_origin = decision_origin(container.source_id());
        if let Some(command) = container.command() {
            service.set_command(sourced_with_origins(
                neutral_command(command),
                vec![runtime_origin.clone(), decision_origin.clone()],
            ));
        }
        if let Some(environment) = container.environment() {
            for variable in environment {
                service.add_environment(sourced_with_origins(
                    neutral_environment(variable),
                    vec![runtime_origin.clone(), decision_origin.clone()],
                ));
            }
        }
        if let Some(labels) = container.labels() {
            for label in labels {
                service.add_label(sourced_with_origins(
                    neutral_label(label),
                    vec![runtime_origin.clone(), decision_origin.clone()],
                ));
                if is_compose_managed_label(label) {
                    self.runtime_managed_metadata(
                        format!("{service_subject}.labels.{}", label.name().as_str()),
                        vec![runtime_origin.clone(), decision_origin.clone()],
                    );
                }
            }
        }
        if let Some(user) = container.user() {
            set_neutral_identity(
                service,
                &sourced_with_origins(user.clone(), vec![runtime_origin.clone(), decision_origin.clone()]),
            );
        }
        if let Some(working_directory) = container.working_directory() {
            service.set_working_directory(sourced_with_origins(
                working_directory.clone(),
                vec![runtime_origin.clone(), decision_origin.clone()],
            ));
        }
        if let Some(healthcheck) = container.healthcheck().filter(|healthcheck| !healthcheck.is_empty()) {
            let origins = vec![runtime_origin.clone(), decision_origin.clone()];
            service.set_healthcheck(sourced_with_origins(
                neutral_healthcheck(healthcheck, &origins),
                origins,
            ));
        }
        self.loss(
            self.codes.comparison_incomplete.clone(),
            format!("{service_subject}.overrides"),
            ConversionKind::Approximate,
            LossExplanation::new(
                "image defaults were unavailable for override reconstruction",
                "no matching complete image observation was linked to the container",
                "inspect the exact image and review which preserved effective values are true overrides",
            ),
            vec![runtime_origin, decision_origin],
        );
    }

    fn inferred_override(&mut self, subject: String, matches_default: bool, origins: Vec<Provenance>) {
        let reason = if matches_default {
            "the effective container value matches the inspected image default and was omitted"
        } else {
            "the effective container value differs from the inspected image default and was retained as an override"
        };
        self.loss(
            self.codes.inferred_override.clone(),
            subject,
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime value was classified by comparing container and image observations",
                reason,
                "review the inferred override because inspection cannot establish original author intent",
            ),
            origins,
        );
    }

    fn runtime_managed_metadata(&mut self, subject: String, origins: Vec<Provenance>) {
        self.loss(
            self.codes.runtime_managed_metadata.clone(),
            subject,
            ConversionKind::Unsupported,
            LossExplanation::new(
                "runtime-managed orchestration metadata cannot be re-authored safely",
                "the observed label uses Compose's reserved com.docker.compose namespace",
                "omit it from authored output or replace it with reviewed application-owned metadata",
            ),
            origins,
        );
    }

    fn comparison_incomplete(&mut self, subject: String, reason: &'static str, origins: Vec<Provenance>) {
        self.loss(
            self.codes.comparison_incomplete.clone(),
            subject,
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime override reconstruction is incomplete",
                reason,
                "supply complete container and image inspection fields or review the preserved effective values",
            ),
            origins,
        );
    }

    fn map_service_groups(&mut self, application: &mut Application) {
        for pod in self.source.pods() {
            self.map_service_group(application, pod);
        }

        for container in self.source.containers() {
            let Some(pod_source_id) = container.pod_source_id() else {
                continue;
            };
            let Some(pod) = self.source.pods().iter().find(|pod| pod.source_id() == pod_source_id) else {
                self.loss(
                    self.codes.pod_relationship.clone(),
                    format!("services.{}.service_group", container.name().as_str()),
                    ConversionKind::Unsupported,
                    LossExplanation::new(
                        "container references a service group missing from the runtime snapshot",
                        "the relationship cannot be reconstructed without the pod observation",
                        "include the referenced pod inspection and review target grouping",
                    ),
                    vec![runtime_origin(container.source_id())],
                );
                continue;
            };
            if pod.members().contains(container.source_id()) {
                continue;
            }
            self.group_relationship_conflict(
                format!("services.{}.service_group", container.name().as_str()),
                "the container references this pod, but the pod does not list the container as a member",
                vec![runtime_origin(container.source_id())],
            );
        }
    }

    fn map_service_group(&mut self, application: &mut Application, pod: &PodObservation) {
        let group_subject = format!("service_groups.{}", pod.name().as_str());
        let group_origin = runtime_origin(pod.source_id());
        let resolution = self.resolutions.service_group_ownership(pod.name());
        let ownership = resolution.map_or(ResourceOwnership::Uncertain, |value| *value.value());
        let mut group = ServiceGroup::new(pod.name().clone(), ownership);

        for (index, member_source_id) in pod.members().iter().enumerate() {
            let member_subject = format!("{group_subject}.members[{index}]");
            let Some(container) = self
                .source
                .containers()
                .iter()
                .find(|container| container.source_id() == member_source_id)
            else {
                self.loss(
                    self.codes.pod_relationship.clone(),
                    member_subject,
                    ConversionKind::Unsupported,
                    LossExplanation::new(
                        "runtime service-group member is missing from the snapshot",
                        "the pod observation references a container that was not supplied",
                        "include the referenced container inspection or remove the stale pod relationship",
                    ),
                    vec![group_origin.clone()],
                );
                continue;
            };

            let member_origins = vec![group_origin.clone(), runtime_origin(container.source_id())];
            let member = sourced_with_origins(container.name().clone(), member_origins.clone());
            if let Err(error) = group.add_member(member) {
                self.invalid_model(&member_subject, &error, member_origins);
                continue;
            }
            match container.pod_source_id() {
                Some(container_pod) if container_pod == pod.source_id() => {
                    self.exact(member_subject, member_origins);
                }
                Some(_) => self.group_relationship_conflict(
                    member_subject,
                    "the pod lists this container, but the container references a different pod",
                    member_origins,
                ),
                None => self.group_relationship_conflict(
                    member_subject,
                    "the pod lists this container, but the container has no matching pod relationship",
                    member_origins,
                ),
            }
        }

        let mut group_origins = vec![group_origin.clone()];
        if let Some(resolution) = resolution {
            group_origins.extend_from_slice(resolution.origins());
        }
        let sourced_group = sourced_with_origins(group, group_origins.clone());
        if let Err(error) = application.add_service_group(sourced_group) {
            self.invalid_model(&group_subject, &error, group_origins);
        } else if resolution.is_some() {
            self.lifecycle_resolution(
                format!("{group_subject}.lifecycle"),
                "service group",
                false,
                group_origins,
            );
        } else {
            self.loss(
                self.codes.pod_relationship.clone(),
                format!("{group_subject}.lifecycle"),
                ConversionKind::Approximate,
                LossExplanation::new(
                    "runtime service-group lifecycle and target semantics are uncertain",
                    "inspection proves structural membership but not which namespaces or future definition should own the group",
                    "review the group and select explicit lifecycle and target grouping policies",
                ),
                vec![group_origin],
            );
        }
    }

    fn group_relationship_conflict(&mut self, subject: String, reason: &'static str, origins: Vec<Provenance>) {
        self.loss(
            self.codes.group_relationship_conflict.clone(),
            subject,
            ConversionKind::Invalid,
            LossExplanation::new(
                "runtime service-group observations contradict each other",
                reason,
                "recapture a consistent pod and container inspection set before generating output",
            ),
            origins,
        );
    }

    fn lifecycle_resolution(
        &mut self,
        subject: String,
        resource_kind: &'static str,
        synthesized: bool,
        origins: Vec<Provenance>,
    ) {
        let reason = if synthesized {
            "the resource was synthesized from a container relationship and its lifecycle was selected by an explicit caller override"
        } else {
            "runtime inspection established the resource and an explicit caller override selected its lifecycle ownership"
        };
        self.loss(
            self.codes.lifecycle_resolution.clone(),
            subject,
            ConversionKind::Approximate,
            LossExplanation::new(
                "runtime resource lifecycle was resolved by the caller",
                reason,
                match resource_kind {
                    "service group" => "review the selected service-group ownership and target grouping semantics",
                    _ => "review the selected resource ownership before deploying generated output",
                },
            ),
            origins,
        );
    }

    fn invalid_model(&mut self, subject: &str, error: &ModelError, origins: Vec<Provenance>) {
        self.loss(
            self.codes.invalid_model.clone(),
            subject.to_owned(),
            ConversionKind::Invalid,
            LossExplanation::new(
                "runtime observation could not enter the neutral application model",
                &error.to_string(),
                "correct the runtime adapter's resource naming or observation mapping",
            ),
            origins,
        );
    }

    fn exact(&mut self, subject: String, origins: Vec<Provenance>) {
        let mut outcome = ConversionOutcome::exact(subject);
        for origin in origins {
            outcome = outcome.with_origin(origin);
        }
        self.outcomes.push(outcome);
    }

    fn loss(
        &mut self,
        code: DiagnosticCode,
        subject: impl Into<String>,
        kind: ConversionKind,
        explanation: LossExplanation<'_>,
        origins: Vec<Provenance>,
    ) {
        let subject = subject.into();
        let diagnostic = Diagnostic::new(code.clone(), severity(kind), explanation.summary)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject.clone())))
            .with_field(DiagnosticField::new(
                "runtime",
                DiagnosticValue::plain(self.source.implementation().as_str()),
            ))
            .with_field(DiagnosticField::new(
                "reason",
                DiagnosticValue::plain(explanation.reason),
            ))
            .with_field(DiagnosticField::new(
                "action",
                DiagnosticValue::plain(explanation.action),
            ));
        if let Ok(mut outcome) = ConversionOutcome::loss(subject, kind, code) {
            for origin in origins {
                outcome = outcome.with_origin(origin);
            }
            self.outcomes.push(outcome);
            self.diagnostics.push(diagnostic);
        }
    }
}

fn runtime_origin(source_id: &SourceId) -> Provenance {
    Provenance::runtime_observation(source_id.clone())
}

fn decision_origin(source_id: &SourceId) -> Provenance {
    Provenance::conversion_decision(source_id.clone())
}

fn neutral_command(command: &EffectiveCommand) -> Command {
    match command {
        EffectiveCommand::Exec(arguments) => Command::Exec(arguments.clone()),
        EffectiveCommand::Empty => Command::Empty,
    }
}

fn neutral_environment(variable: &RuntimeEnvironmentVariable) -> EnvironmentVariable {
    EnvironmentVariable::new(
        variable.name().clone(),
        EnvironmentValue::Literal(variable.value().clone()),
    )
}

fn neutral_label(label: &RuntimeMetadataLabel) -> MetadataLabel {
    MetadataLabel::new(label.name().clone(), label.value().clone())
}

fn is_compose_managed_label(label: &RuntimeMetadataLabel) -> bool {
    label.name().as_str().starts_with("com.docker.compose.")
}

fn neutral_healthcheck(healthcheck: &RuntimeHealthcheck, origins: &[Provenance]) -> Healthcheck {
    let mut neutral = Healthcheck::new();
    if let Some(command) = healthcheck.command() {
        neutral.set_command(sourced_with_origins(command.clone(), origins.to_vec()));
    }
    if let Some(disabled) = healthcheck.disabled() {
        neutral.set_disabled(sourced_with_origins(disabled, origins.to_vec()));
    }
    if let Some(interval) = healthcheck.interval() {
        neutral.set_interval(sourced_with_origins(interval.clone(), origins.to_vec()));
    }
    if let Some(timeout) = healthcheck.timeout() {
        neutral.set_timeout(sourced_with_origins(timeout.clone(), origins.to_vec()));
    }
    if let Some(retries) = healthcheck.retries() {
        neutral.set_retries(sourced_with_origins(retries.clone(), origins.to_vec()));
    }
    if let Some(start_period) = healthcheck.start_period() {
        neutral.set_start_period(sourced_with_origins(start_period.clone(), origins.to_vec()));
    }
    if let Some(start_interval) = healthcheck.start_interval() {
        neutral.set_start_interval(sourced_with_origins(start_interval.clone(), origins.to_vec()));
    }
    neutral
}

fn set_neutral_identity(service: &mut Service, identity: &Sourced<ProtectedString>) {
    let origins = identity.origins().to_vec();
    let (user, group) = identity
        .value()
        .expose()
        .split_once(':')
        .map_or((identity.value().expose(), None), |(user, group)| {
            (user, (!group.is_empty()).then_some(group))
        });
    service.set_user(sourced_with_origins(ProtectedString::sensitive(user), origins.clone()));
    if let Some(group) = group {
        service.set_group(sourced_with_origins(ProtectedString::sensitive(group), origins));
    }
}

fn identity_group(identity: &ProtectedString) -> Option<&str> {
    identity
        .expose()
        .split_once(':')
        .and_then(|(_, group)| (!group.is_empty()).then_some(group))
}

fn sourced_with_origins<T>(value: T, origins: Vec<Provenance>) -> Sourced<T> {
    let mut origins = origins.into_iter();
    let Some(first) = origins.next() else {
        return Sourced::generated(value);
    };
    let mut sourced = Sourced::from_source(value, first);
    for origin in origins {
        sourced.add_origin(origin);
    }
    sourced
}

const fn severity(kind: ConversionKind) -> Severity {
    match kind {
        ConversionKind::Invalid => Severity::Error,
        ConversionKind::Exact => Severity::Note,
        _ => Severity::Warning,
    }
}
