//! Neutral-application-to-Compose planning through `ComposeLens`'s validated generator.

use std::convert::TryFrom;

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, PlatformVersion, RuleId, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, Config, ConfigMaterial, Device, Entrypoint, EnvironmentFileFormat, EnvironmentFileSyntax,
    EnvironmentValue, HostAddressKind, MountSource, Network, ProtectedString, Protocol, Provenance, ProvenanceKind,
    PullPolicy, ResourceOwnership, RestartPolicy, Secret, SecretMaterial, SecurityOption, SelinuxRelabel, Service,
    Sourced, Volume,
};
use compose_lens::{
    model::{MemLimitUnit, ShmSizeUnit},
    render::{
        ComposeDocumentBuilder, GeneratedAnnotation, GeneratedCommand, GeneratedComposeDocument,
        GeneratedConfigFileDefinition, GeneratedDevice, GeneratedDns, GeneratedDnsSearch, GeneratedEntrypoint,
        GeneratedEnvironment, GeneratedEnvironmentFile, GeneratedEnvironmentFileFormat, GeneratedExtraHost,
        GeneratedHostname, GeneratedLabel, GeneratedLogging, GeneratedLoggingOption, GeneratedLoggingOptionValue,
        GeneratedLongDevice, GeneratedMemLimit, GeneratedMount, GeneratedNetworkAttachment, GeneratedNetworkDefinition,
        GeneratedNetworkDriverOption, GeneratedNetworkDriverOptionValue, GeneratedPidsLimit, GeneratedPort,
        GeneratedProtocol, GeneratedPullPolicy, GeneratedResource, GeneratedRestartPolicy,
        GeneratedSecretFileDefinition, GeneratedSelinux, GeneratedService, GeneratedShmSize, GeneratedString,
        GeneratedSysctl, GeneratedSysctls, GeneratedTmpfs, GeneratedUlimit, GeneratedUlimits,
        GeneratedVolumeDefinition, GeneratedVolumeDriverOption, GeneratedVolumeDriverOptionValue, GenerationError,
    },
    source::SourceId,
    validation::{
        CompatibilityClassification, CompatibilityFeature, CompatibilityProfile, ContainerRuntime,
        ImplementationVersion,
    },
};

/// Target implementation name for Docker Compose output.
pub const DOCKER_COMPOSE_TARGET: &str = "docker-compose";

/// Target implementation name for the independent `containers/podman-compose` provider.
pub const PODMAN_COMPOSE_TARGET: &str = "podman-compose";

/// Provider-neutral target for the rolling Compose Specification.
///
/// The internal profile revision is a `BoxFerry` compatibility token, not a Compose Specification
/// release or a claim that every historical Compose consumer accepts generated output.
pub const COMPOSE_SPECIFICATION_TARGET: &str = "compose-specification";

/// Internal revision used with [`COMPOSE_SPECIFICATION_TARGET`].
pub const COMPOSE_SPECIFICATION_PROFILE_REVISION: PlatformVersion = PlatformVersion::new(1, 0, 0);

/// Exact backend runtime used by the selected Compose provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ComposeRuntime {
    /// Docker Engine at an exact version.
    DockerEngine(PlatformVersion),
    /// Podman at an exact version.
    Podman(PlatformVersion),
}

/// Loss-aware exporter for deterministic, parse-back-validated Compose YAML.
#[derive(Clone, Debug)]
pub struct ComposeExporter {
    codes: Codes,
    runtime: Option<ComposeRuntime>,
}

impl ComposeExporter {
    /// Creates an exporter with no assumed backend runtime.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only when an embedded stable code is malformed.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            codes: Codes {
                invalid_target: RuleId::ComposeTargetInvalid.definition().diagnostic_code()?,
                unsupported: RuleId::ComposeOutputUnsupported.definition().diagnostic_code()?,
                generation: RuleId::ComposeGenerationFailed.definition().diagnostic_code()?,
                compatibility: RuleId::ComposeCompatibilityConstraint.definition().diagnostic_code()?,
            },
            runtime: None,
        })
    }

    /// Attaches the exact backend runtime selected by the caller.
    #[must_use]
    pub const fn with_runtime(mut self, runtime: ComposeRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Returns the explicitly selected backend runtime, when present.
    #[must_use]
    pub const fn runtime(&self) -> Option<ComposeRuntime> {
        self.runtime
    }
}

impl ExportAdapter for ComposeExporter {
    type Output = GeneratedComposeDocument;

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

#[derive(Clone, Debug)]
struct Codes {
    invalid_target: DiagnosticCode,
    unsupported: DiagnosticCode,
    generation: DiagnosticCode,
    compatibility: DiagnosticCode,
}

struct Mapping<'a> {
    exporter: &'a ComposeExporter,
    application: &'a Application,
    target: &'a TargetProfile,
    compatibility: Option<CompatibilityProfile>,
    builder: Option<ComposeDocumentBuilder>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
    generation_failed: bool,
}

impl<'a> Mapping<'a> {
    fn new(exporter: &'a ComposeExporter, application: &'a Application, target: &'a TargetProfile) -> Self {
        Self {
            exporter,
            application,
            target,
            compatibility: None,
            builder: Some(ComposeDocumentBuilder::new()),
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
            generation_failed: false,
        }
    }

    fn validate_target(&mut self) -> bool {
        let versions = self.target.versions();
        if self.target.implementation() == COMPOSE_SPECIFICATION_TARGET {
            if versions.maximum() != Some(versions.minimum())
                || versions.minimum() != COMPOSE_SPECIFICATION_PROFILE_REVISION
            {
                self.invalid(
                    self.exporter.codes.invalid_target.clone(),
                    "target.versions",
                    "Compose Specification output requires the BoxFerry compatibility-profile revision",
                    "use the documented exact BoxFerry Compose Specification profile revision",
                    &[],
                );
                return false;
            }
            if self.exporter.runtime.is_some() {
                self.invalid(
                    self.exporter.codes.invalid_target.clone(),
                    "target.runtime",
                    "Compose Specification output does not select a backend runtime",
                    "select a provider-aware target before attaching a runtime",
                    &[],
                );
                return false;
            }
            self.compatibility = Some(CompatibilityProfile::specification());
            return true;
        }
        let Some(maximum) = versions.maximum() else {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target.versions",
                "Compose output requires one exact provider version",
                "set the minimum and maximum to the same exact Compose provider version",
                &[],
            );
            return false;
        };
        if maximum != versions.minimum() {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target.versions",
                "Compose output requires one exact provider version",
                "ComposeLens compatibility evidence is evaluated for exact provider versions, not a range",
                &[],
            );
            return false;
        }
        let Some(version) = implementation_version(versions.minimum()) else {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                "target.versions",
                "Compose provider version is outside the supported numeric range",
                "each version component must fit into an unsigned 32-bit integer",
                &[],
            );
            return false;
        };
        let mut profile = match self.target.implementation() {
            DOCKER_COMPOSE_TARGET => CompatibilityProfile::docker_compose(version),
            PODMAN_COMPOSE_TARGET => CompatibilityProfile::podman_compose(version),
            _ => {
                self.invalid(
                    self.exporter.codes.invalid_target.clone(),
                    "target.implementation",
                    "Compose output requires a recognized provider",
                    "select `docker-compose` or `podman-compose`; `podman compose` is a wrapper, not a provider",
                    &[],
                );
                return false;
            }
        };
        if let Some(runtime) = self.exporter.runtime {
            let runtime = match runtime {
                ComposeRuntime::DockerEngine(version) => {
                    implementation_version(version).map(ContainerRuntime::DockerEngine)
                }
                ComposeRuntime::Podman(version) => implementation_version(version).map(ContainerRuntime::Podman),
            };
            let Some(runtime) = runtime else {
                self.invalid(
                    self.exporter.codes.invalid_target.clone(),
                    "target.runtime.version",
                    "Compose runtime version is outside the supported numeric range",
                    "each version component must fit into an unsigned 32-bit integer",
                    &[],
                );
                return false;
            };
            profile = profile.with_runtime(runtime);
        }
        self.compatibility = Some(profile);
        true
    }

    fn map_application(&mut self) {
        let mut builder = self.builder.take().unwrap_or_default();
        if let Err(error) = builder.set_name(self.application.name().as_str()) {
            self.generation_error("application.name", &error, &[]);
        } else {
            self.exact("application.name", &[]);
        }

        for network in self.application.networks() {
            self.map_network_definition(network, &mut builder);
        }
        for volume in self.application.volumes() {
            self.map_volume_definition(volume, &mut builder);
        }

        for acquisition in self.application.image_acquisitions() {
            self.unsupported(
                &format!("image_acquisitions.{}", acquisition.value().name().as_str()),
                "the current Compose generation boundary does not expose image-acquisition declarations",
                acquisition.origins(),
            );
        }
        for build in self.application.image_builds() {
            self.unsupported(
                &format!("image_builds.{}", build.value().name().as_str()),
                "the current Compose generation boundary does not expose build declarations",
                build.origins(),
            );
        }

        for config in self.application.configs() {
            self.map_config_definition(config, &mut builder);
        }
        for secret in self.application.secrets() {
            self.map_secret_definition(secret, &mut builder);
        }
        for group in self.application.service_groups() {
            self.report_service_group_loss(group);
        }

        for service in self.application.services() {
            if let Some(generated) = self.map_service(service) {
                let subject = format!("services.{}", service.value().name().as_str());
                if let Err(error) = builder.add_service(generated) {
                    self.generation_error(&subject, &error, service.origins());
                }
            }
        }
        self.builder = Some(builder);
    }

    fn map_config_definition(&mut self, sourced: &Sourced<Config>, builder: &mut ComposeDocumentBuilder) {
        let config = sourced.value();
        let subject = format!("configs.{}", config.name().as_str());
        if config.ownership() != ResourceOwnership::Application {
            self.unsupported(
                &subject,
                "only application-owned file-backed configs can be generated by the current ComposeLens boundary",
                sourced.origins(),
            );
            if let Some(runtime_name) = config.runtime_name() {
                self.unsupported(
                    &format!("{subject}.runtime_name"),
                    "the current ComposeLens file-config generator cannot preserve a config runtime name",
                    runtime_name.origins(),
                );
            }
            if let Some(material) = config.material() {
                self.unsupported(
                    &format!("{subject}.material"),
                    "external or uncertain config ownership does not authorize emitting application-managed material",
                    material.origins(),
                );
            }
            return;
        }
        if let Some(runtime_name) = config.runtime_name() {
            self.unsupported(
                &format!("{subject}.runtime_name"),
                "the current ComposeLens file-config generator cannot preserve a config runtime name",
                runtime_name.origins(),
            );
        }
        let Some(material) = config.material() else {
            self.unsupported(
                &format!("{subject}.material"),
                "application-owned config has no file-backed material to generate",
                sourced.origins(),
            );
            return;
        };
        let ConfigMaterial::File(path) = material.value() else {
            self.unsupported(
                &format!("{subject}.material"),
                "only file-backed application configs can be generated; inline and environment material remain outside this boundary",
                material.origins(),
            );
            return;
        };
        let origins = combined_origins(sourced.origins(), material.origins());
        match generated_string(path).and_then(|path| GeneratedConfigFileDefinition::new(config.name().as_str(), path)) {
            Ok(generated) => match builder.add_config_file(generated) {
                Ok(()) => self.exact(&subject, &origins),
                Err(error) => self.generation_error(&subject, &error, &origins),
            },
            Err(error) => self.generation_error(&format!("{subject}.material"), &error, &origins),
        }
    }

    fn map_secret_definition(&mut self, sourced: &Sourced<Secret>, builder: &mut ComposeDocumentBuilder) {
        let secret = sourced.value();
        let subject = format!("secrets.{}", secret.name().as_str());
        if secret.ownership() != ResourceOwnership::Application {
            self.unsupported(
                &subject,
                "only application-owned file-backed secrets can be generated by the current ComposeLens boundary",
                sourced.origins(),
            );
            if let Some(runtime_name) = secret.runtime_name() {
                self.unsupported(
                    &format!("{subject}.runtime_name"),
                    "the current ComposeLens file-secret generator cannot preserve a secret runtime name",
                    runtime_name.origins(),
                );
            }
            if let Some(material) = secret.material() {
                self.unsupported(
                    &format!("{subject}.material"),
                    "external or uncertain secret ownership does not authorize emitting application-managed material",
                    material.origins(),
                );
            }
            return;
        }
        if let Some(runtime_name) = secret.runtime_name() {
            self.unsupported(
                &format!("{subject}.runtime_name"),
                "the current ComposeLens file-secret generator cannot preserve a secret runtime name",
                runtime_name.origins(),
            );
        }
        let Some(material) = secret.material() else {
            self.unsupported(
                &format!("{subject}.material"),
                "application-owned secret has no file-backed material to generate",
                sourced.origins(),
            );
            return;
        };
        let SecretMaterial::File(path) = material.value() else {
            self.unsupported(
                &format!("{subject}.material"),
                "only file-backed application secrets can be generated; environment material remains outside this boundary",
                material.origins(),
            );
            return;
        };
        let origins = combined_origins(sourced.origins(), material.origins());
        match generated_string(path).and_then(|path| GeneratedSecretFileDefinition::new(secret.name().as_str(), path)) {
            Ok(generated) => match builder.add_secret_file(generated) {
                Ok(()) => self.exact(&subject, &origins),
                Err(error) => self.generation_error(&subject, &error, &origins),
            },
            Err(error) => self.generation_error(&format!("{subject}.material"), &error, &origins),
        }
    }

    fn map_resource(
        &mut self,
        name: &str,
        ownership: ResourceOwnership,
        origins: &[Provenance],
        subject: &str,
    ) -> Option<GeneratedResource> {
        let resource = match ownership {
            ResourceOwnership::Application => GeneratedResource::application(name),
            ResourceOwnership::External => GeneratedResource::external(name),
            ResourceOwnership::Implicit => {
                self.unsupported(
                    subject,
                    "implicit source lifecycle is emitted as an externally managed Compose resource",
                    origins,
                );
                GeneratedResource::external(name)
            }
            ResourceOwnership::Uncertain => {
                self.unsupported(
                    subject,
                    "runtime inspection did not determine resource ownership; output conservatively reuses the existing resource",
                    origins,
                );
                GeneratedResource::external(name)
            }
            _ => {
                self.unsupported(
                    subject,
                    "resource ownership variant is newer than this Compose adapter",
                    origins,
                );
                GeneratedResource::external(name)
            }
        };
        let mut resource = match resource {
            Ok(resource) => resource,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                return None;
            }
        };
        if origins
            .iter()
            .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
        {
            if let Err(error) = resource.set_custom_name(name) {
                self.generation_error(subject, &error, origins);
                return None;
            }
        }
        if matches!(ownership, ResourceOwnership::Application | ResourceOwnership::External) {
            self.exact(subject, origins);
        }
        Some(resource)
    }

    #[allow(clippy::too_many_lines)]
    fn map_network_definition(&mut self, sourced: &Sourced<Network>, builder: &mut ComposeDocumentBuilder) {
        let network = sourced.value();
        let subject = format!("networks.{}", network.name().as_str());
        if network.ownership() != ResourceOwnership::Application {
            let Some(mut generated) = self.map_resource(
                network.name().as_str(),
                network.ownership(),
                sourced.origins(),
                &subject,
            ) else {
                self.generation_failed = true;
                return;
            };
            if let Some(runtime_name) = network.runtime_name() {
                if runtime_name.value().is_sensitive() {
                    self.unsupported(
                        &format!("{subject}.name"),
                        "sensitive runtime network names cannot be passed to ComposeLens' plain-string generator",
                        runtime_name.origins(),
                    );
                } else if let Err(error) = generated.set_custom_name(runtime_name.value().expose()) {
                    self.generation_error(&format!("{subject}.name"), &error, runtime_name.origins());
                } else {
                    self.exact(format!("{subject}.name"), runtime_name.origins());
                }
            }
            self.report_external_network_configuration(network, &subject);
            if let Err(error) = builder.add_network(generated) {
                self.generation_error(&subject, &error, sourced.origins());
            }
            return;
        }

        let mut generated = match GeneratedNetworkDefinition::application(network.name().as_str()) {
            Ok(value) => value,
            Err(error) => {
                self.generation_error(&subject, &error, sourced.origins());
                return;
            }
        };
        let mut valid = true;
        if let Some(runtime_name) = network.runtime_name() {
            if runtime_name.value().is_sensitive() {
                self.unsupported(
                    &format!("{subject}.name"),
                    "sensitive runtime network names cannot be passed to ComposeLens' plain-string generator",
                    runtime_name.origins(),
                );
            } else {
                match generated.set_custom_name(runtime_name.value().expose()) {
                    Ok(()) => self.exact(format!("{subject}.name"), runtime_name.origins()),
                    Err(error) => {
                        self.generation_error(&format!("{subject}.name"), &error, runtime_name.origins());
                        valid = false;
                    }
                }
            }
        }
        if let Some(driver) = network.driver() {
            match generated_string(driver.value()).and_then(|value| generated.set_driver(value)) {
                Ok(()) => self.exact(format!("{subject}.driver"), driver.origins()),
                Err(error) => {
                    self.generation_error(&format!("{subject}.driver"), &error, driver.origins());
                    valid = false;
                }
            }
        }
        if let Some(options) = network.driver_options() {
            let origins = collection_or_item_origins(options, network.driver_options_origins());
            let values: Result<Vec<_>, _> = options
                .iter()
                .map(|option| {
                    GeneratedNetworkDriverOption::new(
                        option.value().name().value().as_str(),
                        GeneratedNetworkDriverOptionValue::String(generated_string(option.value().value().value())?),
                    )
                })
                .collect();
            match values.and_then(|values| generated.set_driver_opts(values)) {
                Ok(()) => self.exact(format!("{subject}.driver_opts"), &origins),
                Err(error) => {
                    self.generation_error(&format!("{subject}.driver_opts"), &error, &origins);
                    valid = false;
                }
            }
        }
        if let Some(ipv6) = network.ipv6() {
            match generated.set_enable_ipv6(*ipv6.value()) {
                Ok(()) => self.exact(format!("{subject}.enable_ipv6"), ipv6.origins()),
                Err(error) => {
                    self.generation_error(&format!("{subject}.enable_ipv6"), &error, ipv6.origins());
                    valid = false;
                }
            }
        }
        if let Some(internal) = network.internal() {
            match generated.set_internal(*internal.value()) {
                Ok(()) => self.exact(format!("{subject}.internal"), internal.origins()),
                Err(error) => {
                    self.generation_error(&format!("{subject}.internal"), &error, internal.origins());
                    valid = false;
                }
            }
        }
        if let Some(labels) = network.labels() {
            let origins = collection_or_item_origins(labels, network.labels_origins());
            let values: Result<Vec<_>, _> = labels
                .iter()
                .map(|label| {
                    GeneratedLabel::new(label.value().name().as_str(), generated_string(label.value().value())?)
                })
                .collect();
            match values.and_then(|values| generated.set_labels(values)) {
                Ok(()) => self.exact(format!("{subject}.labels"), &origins),
                Err(error) => {
                    self.generation_error(&format!("{subject}.labels"), &error, &origins);
                    valid = false;
                }
            }
        }
        if let Some(driver) = network.ipam_driver() {
            self.unsupported(
                &format!("{subject}.ipam.driver"),
                "the current Compose generation boundary does not expose generated IPAM definitions",
                driver.origins(),
            );
        }
        if let Some(configs) = network.ipam_configs() {
            self.unsupported(
                &format!("{subject}.ipam.config"),
                "the current Compose generation boundary does not expose generated IPAM definitions",
                &collection_or_item_origins(configs, network.ipam_configs_origins()),
            );
        }
        if valid {
            if let Err(error) = builder.add_network_definition(generated) {
                self.generation_error(&subject, &error, sourced.origins());
            }
        }
    }

    fn report_external_network_configuration(&mut self, network: &Network, subject: &str) {
        let reason = "Compose external networks may only declare their platform name";
        if let Some(driver) = network.driver() {
            self.unsupported(&format!("{subject}.driver"), reason, driver.origins());
        }
        if let Some(values) = network.driver_options() {
            self.unsupported(
                &format!("{subject}.driver_opts"),
                reason,
                &collection_or_item_origins(values, network.driver_options_origins()),
            );
        }
        if let Some(value) = network.internal() {
            self.unsupported(&format!("{subject}.internal"), reason, value.origins());
        }
        if let Some(value) = network.ipv6() {
            self.unsupported(&format!("{subject}.enable_ipv6"), reason, value.origins());
        }
        if let Some(driver) = network.ipam_driver() {
            self.unsupported(&format!("{subject}.ipam.driver"), reason, driver.origins());
        }
        if let Some(values) = network.ipam_configs() {
            self.unsupported(
                &format!("{subject}.ipam.config"),
                reason,
                &collection_or_item_origins(values, network.ipam_configs_origins()),
            );
        }
        if let Some(values) = network.labels() {
            self.unsupported(
                &format!("{subject}.labels"),
                reason,
                &collection_or_item_origins(values, network.labels_origins()),
            );
        }
    }

    fn map_volume_definition(&mut self, sourced: &Sourced<Volume>, builder: &mut ComposeDocumentBuilder) {
        let volume = sourced.value();
        let subject = format!("volumes.{}", volume.name().as_str());
        self.report_compose_native_only_volume_fields(volume, &subject);
        if volume.ownership() != ResourceOwnership::Application {
            let Some(mut generated) = self.map_external_volume_resource(volume, sourced.origins(), &subject) else {
                self.generation_failed = true;
                return;
            };
            if let Some(runtime_name) = volume.runtime_name() {
                if runtime_name.value().is_sensitive() {
                    self.unsupported(
                        &format!("{subject}.name"),
                        "sensitive runtime volume names cannot be passed to ComposeLens' plain-string generator",
                        runtime_name.origins(),
                    );
                } else {
                    match generated.set_custom_name(runtime_name.value().expose()) {
                        Ok(()) => self.exact(format!("{subject}.name"), runtime_name.origins()),
                        Err(error) => {
                            self.generation_error(&format!("{subject}.name"), &error, runtime_name.origins());
                        }
                    }
                }
            }
            self.report_external_volume_configuration(volume, &subject);
            if let Err(error) = builder.add_volume(generated) {
                self.generation_error(&subject, &error, sourced.origins());
            }
            return;
        }

        let mut generated = match GeneratedVolumeDefinition::application(volume.name().as_str()) {
            Ok(value) => value,
            Err(error) => {
                self.generation_error(&subject, &error, sourced.origins());
                return;
            }
        };
        let mut valid = true;
        if let Some(runtime_name) = volume.runtime_name() {
            if runtime_name.value().is_sensitive() {
                self.unsupported(
                    &format!("{subject}.name"),
                    "sensitive runtime volume names cannot be passed to ComposeLens' plain-string generator",
                    runtime_name.origins(),
                );
            } else {
                match generated.set_custom_name(runtime_name.value().expose()) {
                    Ok(()) => self.exact(format!("{subject}.name"), runtime_name.origins()),
                    Err(error) => {
                        self.generation_error(&format!("{subject}.name"), &error, runtime_name.origins());
                        valid = false;
                    }
                }
            }
        }
        if let Some(driver) = volume.driver() {
            match generated_string(driver.value()).and_then(|value| generated.set_driver(value)) {
                Ok(()) => self.exact(format!("{subject}.driver"), driver.origins()),
                Err(error) => {
                    self.generation_error(&format!("{subject}.driver"), &error, driver.origins());
                    valid = false;
                }
            }
        }
        self.map_local_volume_driver_options(volume, &mut generated, &subject, &mut valid);
        if let Some(labels) = volume.labels() {
            let origins = collection_or_item_origins(labels, volume.labels_origins());
            let values: Result<Vec<_>, _> = labels
                .iter()
                .map(|label| {
                    GeneratedLabel::new(label.value().name().as_str(), generated_string(label.value().value())?)
                })
                .collect();
            match values.and_then(|values| generated.set_labels(values)) {
                Ok(()) => self.exact(format!("{subject}.labels"), &origins),
                Err(error) => {
                    self.generation_error(&format!("{subject}.labels"), &error, &origins);
                    valid = false;
                }
            }
        }
        if valid {
            if let Err(error) = builder.add_volume_definition(generated) {
                self.generation_error(&subject, &error, sourced.origins());
            }
        }
    }

    fn map_external_volume_resource(
        &mut self,
        volume: &Volume,
        origins: &[Provenance],
        subject: &str,
    ) -> Option<GeneratedResource> {
        let generated = match volume.ownership() {
            ResourceOwnership::External => GeneratedResource::external(volume.name().as_str()),
            ResourceOwnership::Implicit => {
                self.unsupported(
                    subject,
                    "implicit source lifecycle is emitted as an externally managed Compose resource",
                    origins,
                );
                GeneratedResource::external(volume.name().as_str())
            }
            ResourceOwnership::Uncertain => {
                self.unsupported(
                    subject,
                    "runtime inspection did not determine volume ownership; output conservatively reuses the existing volume",
                    origins,
                );
                GeneratedResource::external(volume.name().as_str())
            }
            ResourceOwnership::Application => return None,
            _ => {
                self.unsupported(
                    subject,
                    "volume ownership variant is newer than this Compose adapter",
                    origins,
                );
                GeneratedResource::external(volume.name().as_str())
            }
        };
        let mut generated = match generated {
            Ok(generated) => generated,
            Err(error) => {
                self.generation_error(subject, &error, origins);
                return None;
            }
        };
        if origins
            .iter()
            .any(|origin| origin.kind() == ProvenanceKind::RuntimeObservation)
            && volume.runtime_name().is_none()
        {
            if let Err(error) = generated.set_custom_name(volume.name().as_str()) {
                self.generation_error(subject, &error, origins);
                return None;
            }
        }
        if matches!(volume.ownership(), ResourceOwnership::External) {
            self.exact(subject, origins);
        }
        Some(generated)
    }

    fn map_local_volume_driver_options(
        &mut self,
        volume: &Volume,
        generated: &mut GeneratedVolumeDefinition,
        subject: &str,
        valid: &mut bool,
    ) {
        let fields = [
            ("type", volume.volume_type()),
            ("device", volume.device()),
            ("o", volume.options()),
        ];
        if fields.iter().all(|(_, value)| value.is_none()) {
            return;
        }
        let local_driver = volume.driver().is_some_and(|driver| driver.value().expose() == "local");
        if !local_driver {
            for (field, value) in fields {
                if let Some(value) = value {
                    self.unsupported(
                        &format!("{subject}.driver_opts.{field}"),
                        "Compose local-driver options require an explicit `driver: local`",
                        value.origins(),
                    );
                }
            }
            return;
        }
        let origins = fields
            .iter()
            .filter_map(|(_, value)| value.map(Sourced::origins))
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let values: Result<Vec<_>, _> = fields
            .into_iter()
            .filter_map(|(name, value)| {
                value.map(|value| {
                    GeneratedVolumeDriverOption::new(
                        name,
                        GeneratedVolumeDriverOptionValue::String(generated_string(value.value())?),
                    )
                })
            })
            .collect();
        match values.and_then(|values| generated.set_driver_opts(values)) {
            Ok(()) => self.exact(format!("{subject}.driver_opts"), &origins),
            Err(error) => {
                self.generation_error(&format!("{subject}.driver_opts"), &error, &origins);
                *valid = false;
            }
        }
    }

    fn report_external_volume_configuration(&mut self, volume: &Volume, subject: &str) {
        let reason = "Compose external volumes may only declare platform name";
        if let Some(driver) = volume.driver() {
            self.unsupported(&format!("{subject}.driver"), reason, driver.origins());
        }
        for (field, value) in [
            ("type", volume.volume_type()),
            ("device", volume.device()),
            ("o", volume.options()),
        ] {
            if let Some(value) = value {
                self.unsupported(&format!("{subject}.driver_opts.{field}"), reason, value.origins());
            }
        }
        if let Some(values) = volume.labels() {
            self.unsupported(
                &format!("{subject}.labels"),
                reason,
                &collection_or_item_origins(values, volume.labels_origins()),
            );
        }
    }

    fn report_compose_native_only_volume_fields(&mut self, volume: &Volume, subject: &str) {
        for (name, value) in [
            ("service_name", volume.service_name()),
            ("user", volume.user()),
            ("group", volume.group()),
            ("uid", volume.uid()),
            ("gid", volume.gid()),
        ] {
            if let Some(value) = value {
                self.unsupported(
                    &format!("{subject}.{name}"),
                    "Quadlet-only volume setting has no reviewed Compose volume-definition mapping",
                    value.origins(),
                );
            }
        }
        if let Some(value) = volume.copy() {
            self.unsupported(
                &format!("{subject}.copy"),
                "Quadlet-only volume setting has no reviewed Compose volume-definition mapping",
                value.origins(),
            );
        }
        for (name, values, origins) in [
            (
                "containers_conf_modules",
                volume.containers_conf_modules(),
                volume.containers_conf_modules_origins(),
            ),
            ("global_args", volume.global_args(), volume.global_args_origins()),
            ("podman_args", volume.podman_args(), volume.podman_args_origins()),
        ] {
            if let Some(values) = values {
                self.unsupported(
                    &format!("{subject}.{name}"),
                    "Quadlet-only volume setting has no reviewed Compose volume-definition mapping",
                    &collection_or_item_origins(values, origins),
                );
            }
        }
        if let Some(image) = volume.image_source() {
            self.unsupported(
                &format!("{subject}.image"),
                "Quadlet volume image source has no reviewed Compose volume-definition mapping",
                image.origins(),
            );
        }
    }

    fn map_service(&mut self, sourced: &Sourced<Service>) -> Option<GeneratedService> {
        let service = sourced.value();
        let service_name = service.name().as_str();
        let service_subject = format!("services.{service_name}");
        let mut generated = match GeneratedService::new(service_name) {
            Ok(generated) => generated,
            Err(error) => {
                self.generation_error(&service_subject, &error, sourced.origins());
                return None;
            }
        };
        if let Some(runtime_name) = service.runtime_name() {
            match generated_string(runtime_name.value()).and_then(|name| generated.set_container_name(name)) {
                Ok(()) => self.exact(format!("{service_subject}.container_name"), runtime_name.origins()),
                Err(error) => {
                    self.generation_error(
                        &format!("{service_subject}.container_name"),
                        &error,
                        runtime_name.origins(),
                    );
                }
            }
        }
        self.report_compose_native_only_service_fields(service, &service_subject);
        if !self.map_image(service, sourced.origins(), &mut generated, &service_subject) {
            return None;
        }
        self.map_restart_policy(service, &mut generated, &service_subject);
        self.map_command(service, &mut generated, &service_subject);
        self.map_released_container_settings(service, &mut generated, &service_subject);
        self.map_identity(service, &mut generated, &service_subject);
        self.map_execution_context(service, &mut generated, &service_subject);
        self.map_dns(service, &mut generated, &service_subject);
        self.map_security_options(service, &mut generated, &service_subject);
        self.map_environment_files(service, &mut generated, &service_subject);
        self.map_environment(service, &mut generated, &service_subject);
        self.map_labels(service, &mut generated, &service_subject);
        self.map_host_mappings(service, &mut generated, &service_subject);
        self.map_ports(service, &mut generated, &service_subject);
        self.map_mounts(service, &mut generated, &service_subject);
        self.map_network_attachments(service, &mut generated, &service_subject);
        self.report_unimplemented_service_fields(service, &service_subject);
        Some(generated)
    }

    fn map_restart_policy(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        let Some(restart) = service.restart_policy() else {
            return;
        };
        let subject = format!("{service_subject}.restart_policy");
        let policy = match restart.value() {
            RestartPolicy::Never => GeneratedRestartPolicy::No,
            RestartPolicy::Always => GeneratedRestartPolicy::Always,
            RestartPolicy::OnFailure { maximum_retries } => GeneratedRestartPolicy::OnFailure {
                maximum_retries: maximum_retries.map(std::num::NonZeroU64::get),
            },
            RestartPolicy::UnlessStopped => GeneratedRestartPolicy::UnlessStopped,
            _ => {
                self.unsupported(
                    &subject,
                    "restart-policy variant is newer than this Compose adapter",
                    restart.origins(),
                );
                return;
            }
        };
        match generated.set_restart(policy) {
            Ok(()) => self.exact(subject, restart.origins()),
            Err(error) => self.generation_error(&subject, &error, restart.origins()),
        }
    }

    fn map_dns(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        if let Some(values) = service.dns_servers() {
            let subject = format!("{service_subject}.dns");
            let origins = collection_or_item_origins(values, service.dns_servers_origins());
            let generated_values: Result<Vec<_>, _> =
                values.iter().map(|value| generated_string(value.value())).collect();
            match generated_values.and_then(|values| generated.set_dns(GeneratedDns::List(values))) {
                Ok(()) if values.is_empty() => self.exact(subject, &origins),
                Ok(()) if values.iter().any(|value| value.value().expose() == "none") => {
                    self.unsupported(&subject, "DNS `none` has target-specific resolver semantics", &origins);
                }
                Ok(()) => self.exact(subject, &origins),
                Err(error) => self.generation_error(&subject, &error, &origins),
            }
        }
        if let Some(values) = service.dns_options() {
            let subject = format!("{service_subject}.dns_opt");
            let origins = collection_or_item_origins(values, service.dns_options_origins());
            let generated_values: Result<Vec<_>, _> =
                values.iter().map(|value| generated_string(value.value())).collect();
            match generated_values.and_then(|values| generated.set_dns_options(values)) {
                Ok(()) if values.is_empty() => self.exact(subject, &origins),
                Ok(()) => self.exact(subject, &origins),
                Err(error) => self.generation_error(&subject, &error, &origins),
            }
        }
        if let Some(values) = service.dns_search_domains() {
            let subject = format!("{service_subject}.dns_search");
            let origins = collection_or_item_origins(values, service.dns_search_domains_origins());
            let generated_values: Result<Vec<_>, _> =
                values.iter().map(|value| generated_string(value.value())).collect();
            match generated_values.and_then(|values| generated.set_dns_search(GeneratedDnsSearch::List(values))) {
                Ok(()) if values.is_empty() => self.exact(subject, &origins),
                Ok(()) if values.iter().any(|value| value.value().expose() == ".") => self.unsupported(
                    &subject,
                    "DNS search `.` has target-specific resolver semantics",
                    &origins,
                ),
                Ok(()) => self.exact(subject, &origins),
                Err(error) => self.generation_error(&subject, &error, &origins),
            }
        }
    }

    fn map_security_options(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        let Some(options) = service.security_options() else {
            return;
        };
        let subject = format!("{service_subject}.security_opt");
        let origins = collection_or_item_origins(options, service.security_options_origins());
        let mut generated_options = Vec::with_capacity(options.len());
        let mut all_exact = true;
        for (index, option) in options.iter().enumerate() {
            match security_option_string(option.value()) {
                Ok(SecurityOptionGeneration::Generated(value)) => generated_options.push(value),
                Ok(SecurityOptionGeneration::Unsupported(reason)) => {
                    all_exact = false;
                    self.unsupported(&format!("{subject}[{index}]"), reason, option.origins());
                }
                Err(error) => {
                    self.generation_error(&subject, &error, &origins);
                    return;
                }
            }
        }
        match generated.set_security_options(generated_options) {
            Ok(()) if all_exact => self.exact(subject, &origins),
            Ok(()) => {}
            Err(error) => self.generation_error(&subject, &error, &origins),
        }
    }

    fn map_image(
        &mut self,
        service: &Service,
        service_origins: &[Provenance],
        generated: &mut GeneratedService,
        service_subject: &str,
    ) -> bool {
        let Some(image) = service.image() else {
            self.invalid(
                self.exporter.codes.generation.clone(),
                &format!("{service_subject}.image"),
                "generated Compose service has no runnable source",
                "the neutral service has neither an image nor a representable build definition",
                service_origins,
            );
            self.generation_failed = true;
            return false;
        };
        let result = GeneratedString::plain(image.value().as_str()).and_then(|image| generated.set_image(image));
        if let Err(error) = result {
            self.generation_error(&format!("{service_subject}.image"), &error, image.origins());
            return false;
        }
        if image.value().tag().is_some() && image.value().digest().is_some() {
            self.compatibility(
                &format!("{service_subject}.image"),
                CompatibilityFeature::ImageTagAndDigest,
                image.origins(),
            );
        } else {
            self.exact(format!("{service_subject}.image"), image.origins());
        }
        true
    }

    fn map_command(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        let Some(command) = service.command() else {
            return;
        };
        let command_value = match command.value() {
            Command::Exec(arguments) => Some(
                arguments
                    .iter()
                    .map(generated_string)
                    .collect::<Result<Vec<_>, _>>()
                    .map(GeneratedCommand::Exec),
            ),
            Command::Shell(value) => Some(generated_string(value).map(GeneratedCommand::Shell)),
            Command::Empty => Some(Ok(GeneratedCommand::Empty)),
            _ => {
                self.unsupported(
                    &format!("{service_subject}.command"),
                    "command variant is newer than this Compose adapter",
                    command.origins(),
                );
                None
            }
        };
        if let Some(command_value) = command_value {
            match command_value.and_then(|command_value| generated.set_command(command_value)) {
                Ok(()) => self.exact(format!("{service_subject}.command"), command.origins()),
                Err(error) => self.generation_error(&format!("{service_subject}.command"), &error, command.origins()),
            }
        }
    }

    fn map_execution_context(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        if let Some(working_directory) = service.working_directory() {
            let field = format!("{subject}.working_directory");
            match generated_string(working_directory.value()).and_then(|value| generated.set_working_dir(value)) {
                Ok(()) => self.exact(field, working_directory.origins()),
                Err(error) => self.generation_error(&field, &error, working_directory.origins()),
            }
        }
        if let Some(read_only) = service.read_only_root_filesystem() {
            let field = format!("{subject}.read_only_root_filesystem");
            match generated.set_read_only(*read_only.value()) {
                Ok(()) => self.exact(field, read_only.origins()),
                Err(error) => self.generation_error(&field, &error, read_only.origins()),
            }
        }
    }

    fn map_environment(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for environment in service.environment() {
            let subject = format!("{service_subject}.environment.{}", environment.value().name().as_str());
            match environment.value().value() {
                EnvironmentValue::Literal(value) => match generated_string(value)
                    .and_then(|value| GeneratedEnvironment::literal(environment.value().name().as_str(), value))
                {
                    Ok(value) => {
                        generated.add_environment(value);
                        self.exact(subject, environment.origins());
                    }
                    Err(error) => self.generation_error(&subject, &error, environment.origins()),
                },
                EnvironmentValue::Host => match GeneratedEnvironment::host(environment.value().name().as_str()) {
                    Ok(value) => {
                        generated.add_environment(value);
                        self.exact(subject, environment.origins());
                    }
                    Err(error) => self.generation_error(&subject, &error, environment.origins()),
                },
                EnvironmentValue::Unset => self.unsupported(
                    &subject,
                    "Compose environment syntax cannot guarantee that a variable is absent from the container",
                    environment.origins(),
                ),
                _ => self.unsupported(
                    &subject,
                    "environment value variant is newer than this Compose adapter",
                    environment.origins(),
                ),
            }
        }
    }

    fn map_environment_files(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for (index, sourced) in service.environment_files().iter().enumerate() {
            let subject = format!("{service_subject}.environment_files[{index}]");
            let environment_file = sourced.value();
            let path = match generated_string(environment_file.path()) {
                Ok(path) => path,
                Err(error) => {
                    self.generation_error(&subject, &error, sourced.origins());
                    continue;
                }
            };
            let value = match environment_file.syntax() {
                EnvironmentFileSyntax::Short => {
                    if environment_file.required().is_some() || environment_file.format().is_some() {
                        self.unsupported(
                            &subject,
                            "short-syntax environment files cannot carry long-syntax options",
                            sourced.origins(),
                        );
                        continue;
                    }
                    GeneratedEnvironmentFile::short(path)
                }
                EnvironmentFileSyntax::Long => {
                    let format = match environment_file.format().map(Sourced::value) {
                        Some(EnvironmentFileFormat::Raw) => Some(GeneratedEnvironmentFileFormat::Raw),
                        Some(_) => {
                            self.unsupported(
                                &subject,
                                "environment-file format variant is newer than this Compose adapter",
                                sourced.origins(),
                            );
                            continue;
                        }
                        None => None,
                    };
                    GeneratedEnvironmentFile::long(
                        path,
                        environment_file.required().map(|required| *required.value()),
                        format,
                    )
                }
                _ => {
                    self.unsupported(
                        &subject,
                        "environment-file syntax variant is newer than this Compose adapter",
                        sourced.origins(),
                    );
                    continue;
                }
            };
            match value {
                Ok(value) => {
                    generated.add_environment_file(value);
                    self.exact(subject, sourced.origins());
                }
                Err(error) => self.generation_error(&subject, &error, sourced.origins()),
            }
        }
    }

    fn map_labels(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
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
            match generated_string(label.value().value())
                .and_then(|value| GeneratedLabel::new(name, value))
                .and_then(|value| generated.add_label(value))
            {
                Ok(()) => self.exact(subject, label.origins()),
                Err(error) => self.generation_error(&subject, &error, label.origins()),
            }
        }
    }

    fn map_host_mappings(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for (index, host) in service.host_mappings().iter().enumerate() {
            let subject = format!("{service_subject}.extra_hosts[{index}]");
            match GeneratedExtraHost::new(host.value().hostname().as_str(), host.value().address().raw()) {
                Ok(value) => {
                    generated.add_extra_host(value);
                    if host.value().address().kind() == HostAddressKind::HostGateway {
                        self.compatibility(&subject, CompatibilityFeature::HostGatewayToken, host.origins());
                    } else {
                        self.exact(subject, host.origins());
                    }
                }
                Err(error) => self.generation_error(&subject, &error, host.origins()),
            }
        }
    }

    fn map_ports(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for (index, port) in service.ports().iter().enumerate() {
            let subject = format!("{service_subject}.ports[{index}]");
            let protocol = match port.value().protocol() {
                Protocol::Tcp => GeneratedProtocol::Tcp,
                Protocol::Udp => GeneratedProtocol::Udp,
                Protocol::Sctp => GeneratedProtocol::Sctp,
                Protocol::Other(protocol) => {
                    self.unsupported(
                        &subject,
                        &format!("Compose generation does not support protocol `{protocol}`"),
                        port.origins(),
                    );
                    continue;
                }
                _ => {
                    self.unsupported(
                        &subject,
                        "port protocol variant is newer than this Compose adapter",
                        port.origins(),
                    );
                    continue;
                }
            };
            match GeneratedPort::new(
                port.value().container(),
                port.value().published(),
                port.value().host_address().map(str::to_owned),
                protocol,
            ) {
                Ok(value) => {
                    generated.add_port(value);
                    if matches!(port.value().protocol(), Protocol::Sctp) {
                        self.unsupported(
                            &subject,
                            "SCTP syntax is preserved, but the selected provider/runtime pair has no reviewed compatibility evidence",
                            port.origins(),
                        );
                    } else {
                        self.exact(subject, port.origins());
                    }
                }
                Err(error) => self.generation_error(&subject, &error, port.origins()),
            }
        }
    }

    fn map_mounts(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for (index, mount) in service.mounts().iter().enumerate() {
            let subject = format!("{service_subject}.mounts[{index}]");
            let selinux = match mount.value().selinux_relabel() {
                Some(SelinuxRelabel::Shared) => Some(GeneratedSelinux::Shared),
                Some(SelinuxRelabel::Private) => Some(GeneratedSelinux::Private),
                Some(_) => {
                    self.unsupported(
                        &subject,
                        "SELinux relabel variant is newer than this Compose adapter",
                        mount.origins(),
                    );
                    continue;
                }
                None => None,
            };
            let value = match mount.value().source() {
                MountSource::Volume(source) => {
                    GeneratedMount::volume(source.as_str(), mount.value().target(), mount.value().read_only())
                }
                MountSource::HostPath(source) => {
                    GeneratedMount::bind(source, mount.value().target(), mount.value().read_only(), selinux)
                }
                MountSource::Anonymous => GeneratedMount::anonymous(mount.value().target(), mount.value().read_only()),
                _ => {
                    self.unsupported(
                        &subject,
                        "mount source variant is newer than this Compose adapter",
                        mount.origins(),
                    );
                    continue;
                }
            };
            match value {
                Ok(value) => {
                    generated.add_mount(value);
                    if mount.value().selinux_relabel().is_some() {
                        self.compatibility(&subject, CompatibilityFeature::ShortBindSelinuxRelabel, mount.origins());
                    } else {
                        self.exact(subject, mount.origins());
                    }
                }
                Err(error) => self.generation_error(&subject, &error, mount.origins()),
            }
        }
    }

    fn map_network_attachments(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        for network in service.networks() {
            let subject = format!("{service_subject}.networks.{}", network.value().network().as_str());
            match GeneratedNetworkAttachment::new(network.value().network().as_str()) {
                Ok(mut value) => {
                    let mut valid = true;
                    for (index, alias) in network.value().aliases().iter().enumerate() {
                        if network
                            .value()
                            .alias_sensitivities()
                            .get(index)
                            .copied()
                            .unwrap_or(false)
                        {
                            self.unsupported(
                                &subject,
                                "sensitive network aliases cannot be passed to ComposeLens' plain-string generator",
                                network.origins(),
                            );
                            valid = false;
                            break;
                        }
                        if let Err(error) = value.add_alias(alias) {
                            self.generation_error(&subject, &error, network.origins());
                            valid = false;
                            break;
                        }
                    }
                    if let Some(address) = network.value().ipv4_address() {
                        if let Err(error) =
                            generated_string(address.value()).and_then(|address| value.set_ipv4_address(address))
                        {
                            self.generation_error(&subject, &error, address.origins());
                            valid = false;
                        }
                    }
                    if let Some(address) = network.value().ipv6_address() {
                        if let Err(error) =
                            generated_string(address.value()).and_then(|address| value.set_ipv6_address(address))
                        {
                            self.generation_error(&subject, &error, address.origins());
                            valid = false;
                        }
                    }
                    if valid {
                        match generated.add_network(value) {
                            Ok(()) => self.exact(subject, network.origins()),
                            Err(error) => self.generation_error(&subject, &error, network.origins()),
                        }
                    }
                }
                Err(error) => self.generation_error(&subject, &error, network.origins()),
            }
        }
    }

    fn map_released_container_settings(
        &mut self,
        service: &Service,
        generated: &mut GeneratedService,
        service_subject: &str,
    ) {
        self.map_entrypoint_lifecycle_and_pull(service, generated, service_subject);
        self.map_hostname_pids_shm_memory(service, generated, service_subject);
        self.map_collections(service, generated, service_subject);
        self.map_annotations_logging_and_reload(service, generated, service_subject);
    }

    fn map_entrypoint_lifecycle_and_pull(
        &mut self,
        service: &Service,
        generated: &mut GeneratedService,
        subject: &str,
    ) {
        if let Some(entrypoint) = service.entrypoint() {
            let field = format!("{subject}.entrypoint");
            let value = match entrypoint.value() {
                Entrypoint::Exec(values) => values
                    .iter()
                    .map(generated_string)
                    .collect::<Result<Vec<_>, _>>()
                    .map(GeneratedEntrypoint::List),
                Entrypoint::Shell(value) => generated_string(value).map(GeneratedEntrypoint::String),
                Entrypoint::Empty => Ok(GeneratedEntrypoint::Empty),
                _ => {
                    self.unsupported(
                        &field,
                        "entrypoint variant is newer than this Compose adapter",
                        entrypoint.origins(),
                    );
                    return;
                }
            };
            match value.and_then(|value| generated.set_entrypoint(value)) {
                Ok(()) => self.exact(field, entrypoint.origins()),
                Err(error) => self.generation_error(&field, &error, entrypoint.origins()),
            }
        }
        if let Some(init) = service.run_init() {
            let field = format!("{subject}.init");
            match generated.set_init(*init.value()) {
                Ok(()) => self.exact(field, init.origins()),
                Err(error) => self.generation_error(&field, &error, init.origins()),
            }
        }
        if let Some(timeout) = service.stop_timeout() {
            let field = format!("{subject}.stop_grace_period");
            match generated_string(&ProtectedString::plain(timeout.value().as_str()))
                .and_then(|value| generated.set_stop_grace_period(value))
            {
                Ok(()) => self.exact(field, timeout.origins()),
                Err(error) => self.generation_error(&field, &error, timeout.origins()),
            }
        }
        if let Some(policy) = service.pull_policy() {
            let field = format!("{subject}.pull_policy");
            let value = match policy.value() {
                PullPolicy::Always => Ok(GeneratedPullPolicy::Always),
                PullPolicy::Missing => Ok(GeneratedPullPolicy::Missing),
                PullPolicy::Never => Ok(GeneratedPullPolicy::Never),
                PullPolicy::IfNotPresent => Ok(GeneratedPullPolicy::IfNotPresentAlias),
                PullPolicy::Build => Ok(GeneratedPullPolicy::Build),
                PullPolicy::Daily => Ok(GeneratedPullPolicy::Daily),
                PullPolicy::Weekly => Ok(GeneratedPullPolicy::Weekly),
                PullPolicy::Every(value) => generated_string(value).map(GeneratedPullPolicy::Every),
                PullPolicy::Raw(_) => {
                    self.unsupported(
                        &field,
                        "provider-specific pull policy cannot be silently normalized for Compose output",
                        policy.origins(),
                    );
                    return;
                }
                _ => {
                    self.unsupported(
                        &field,
                        "pull-policy variant is newer than this Compose adapter",
                        policy.origins(),
                    );
                    return;
                }
            };
            match value.and_then(|value| generated.set_pull_policy(value)) {
                Ok(()) => self.exact(field, policy.origins()),
                Err(error) => self.generation_error(&field, &error, policy.origins()),
            }
        }
    }

    fn map_hostname_pids_shm_memory(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        if let Some(value) = service.hostname() {
            let field = format!("{subject}.hostname");
            match generated_string(value.value())
                .map(GeneratedHostname::Resolved)
                .and_then(|value| generated.set_hostname(value))
            {
                Ok(()) => self.exact(field, value.origins()),
                Err(error) => self.generation_error(&field, &error, value.origins()),
            }
        }
        if let Some(value) = service.pids_limit() {
            let field = format!("{subject}.pids_limit");
            let limit = if value.value().expose() == "-1" {
                GeneratedPidsLimit::Unlimited
            } else {
                GeneratedPidsLimit::Finite(value.value().expose().to_owned())
            };
            match generated.set_pids_limit(limit) {
                Ok(()) => self.exact(field, value.origins()),
                Err(error) => self.generation_error(&field, &error, value.origins()),
            }
        }
        if let Some(value) = service.shm_size() {
            self.map_sized_value(value.value(), value.origins(), &format!("{subject}.shm_size"), |size| {
                generated.set_shm_size(size)
            });
        }
        if let Some(value) = service.memory_limit() {
            self.map_memory_value(
                value.value(),
                value.origins(),
                &format!("{subject}.mem_limit"),
                |limit| generated.set_mem_limit(limit),
            );
        }
    }

    fn map_sized_value<F>(&mut self, value: &ProtectedString, origins: &[Provenance], subject: &str, set: F)
    where
        F: FnOnce(GeneratedShmSize) -> Result<(), GenerationError>,
    {
        let Some((amount, unit)) = split_size(value.expose()) else {
            self.unsupported(
                subject,
                "shared-memory size requires a documented lowercase unit",
                origins,
            );
            return;
        };
        let amount = if value.is_sensitive() {
            GeneratedString::sensitive(amount)
        } else {
            GeneratedString::plain(amount)
        };
        let Ok(amount) = amount else {
            self.unsupported(subject, "invalid shared-memory size", origins);
            return;
        };
        match set(GeneratedShmSize::Explicit { amount, unit }) {
            Ok(()) => self.exact(subject, origins),
            Err(error) => self.generation_error(subject, &error, origins),
        }
    }

    fn map_memory_value<F>(&mut self, value: &ProtectedString, origins: &[Provenance], subject: &str, set: F)
    where
        F: FnOnce(GeneratedMemLimit) -> Result<(), GenerationError>,
    {
        let Some((amount, unit)) = split_memory(value.expose()) else {
            self.unsupported(subject, "memory limit requires a documented lowercase unit", origins);
            return;
        };
        let amount = if value.is_sensitive() {
            GeneratedString::sensitive(amount)
        } else {
            GeneratedString::plain(amount)
        };
        match amount.and_then(|amount| set(GeneratedMemLimit::Explicit { amount, unit })) {
            Ok(()) => self.exact(subject, origins),
            Err(error) => self.generation_error(subject, &error, origins),
        }
    }

    fn map_collections(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        self.map_capabilities(service, generated, subject);
        self.map_resource_collections(service, generated, subject);
        self.map_devices_signals_and_expose(service, generated, subject);
    }

    fn map_capabilities(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        for (name, values, origins, set) in [
            (
                "cap_add",
                service.cap_add(),
                service.cap_add_origins(),
                GeneratedService::set_cap_add as fn(&mut GeneratedService, Vec<GeneratedString>) -> _,
            ),
            (
                "cap_drop",
                service.cap_drop(),
                service.cap_drop_origins(),
                GeneratedService::set_cap_drop as fn(&mut GeneratedService, Vec<GeneratedString>) -> _,
            ),
        ] {
            if let Some(values) = values {
                let field = format!("{subject}.{name}");
                let all_origins = collection_or_item_origins(values, origins);
                match values
                    .iter()
                    .map(|value| generated_string(value.value()))
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|values| set(generated, values))
                {
                    Ok(()) => self.exact(field, &all_origins),
                    Err(error) => self.generation_error(&field, &error, &all_origins),
                }
            }
        }
    }

    fn map_resource_collections(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        if let Some(values) = service.tmpfs() {
            let field = format!("{subject}.tmpfs");
            let origins = collection_or_item_origins(values, service.tmpfs_origins());
            match values
                .iter()
                .map(|value| generated_string(value.value()))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|values| generated.set_tmpfs(GeneratedTmpfs::List(values)))
            {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
        if let Some(values) = service.sysctls() {
            let field = format!("{subject}.sysctls");
            let origins = collection_or_item_origins(values, service.sysctls_origins());
            let values = values
                .iter()
                .map(|value| {
                    GeneratedSysctl::new(value.value().name().expose(), generated_string(value.value().value())?)
                })
                .collect::<Result<Vec<_>, _>>();
            match values.and_then(|values| generated.set_sysctls(GeneratedSysctls::Map(values))) {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
        if let Some(values) = service.ulimits() {
            let field = format!("{subject}.ulimits");
            let origins = resource_limit_collection_origins(values, service.ulimits_origins());
            let values = values
                .iter()
                .map(|value| {
                    let limit = value.value();
                    match (limit.soft(), limit.hard()) {
                        (Some(soft), Some(hard)) if soft.value().expose() == hard.value().expose() => {
                            GeneratedUlimit::single(limit.name().expose(), generated_string(soft.value())?)
                        }
                        (Some(soft), Some(hard)) => GeneratedUlimit::range(
                            limit.name().expose(),
                            generated_string(soft.value())?,
                            generated_string(hard.value())?,
                        ),
                        _ => Err(GenerationError::MissingUlimitRangeMember("soft/hard")),
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match values
                .and_then(GeneratedUlimits::new)
                .and_then(|values| generated.set_ulimits(values))
            {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
    }

    fn map_devices_signals_and_expose(&mut self, service: &Service, generated: &mut GeneratedService, subject: &str) {
        if let Some(values) = service.devices() {
            let field = format!("{subject}.devices");
            let origins = device_collection_origins(values, service.devices_origins());
            let values = values
                .iter()
                .map(|value| match value.value() {
                    Device::Short(value) => generated_string(value).map(GeneratedDevice::Short),
                    Device::Long {
                        source,
                        target,
                        permissions,
                    } => {
                        let source = source.as_ref().ok_or(GenerationError::InvalidDeviceValue("source"))?;
                        GeneratedLongDevice::new(
                            generated_string(source.value())?,
                            target
                                .as_ref()
                                .map(|value| generated_string(value.value()))
                                .transpose()?,
                            permissions
                                .as_ref()
                                .map(|value| generated_string(value.value()))
                                .transpose()?,
                        )
                        .map(GeneratedDevice::Long)
                    }
                    _ => Err(GenerationError::InvalidDeviceValue("variant")),
                })
                .collect::<Result<Vec<_>, _>>();
            match values.and_then(|values| generated.set_devices(values)) {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
        if let Some(value) = service.stop_signal() {
            let field = format!("{subject}.stop_signal");
            match generated_string(value.value()).and_then(|value| generated.set_stop_signal(value)) {
                Ok(()) => self.exact(field, value.origins()),
                Err(error) => self.generation_error(&field, &error, value.origins()),
            }
        }
        if let Some(values) = service.exposed_ports() {
            let field = format!("{subject}.expose");
            let origins = collection_or_item_origins(values, service.exposed_ports_origins());
            let values = values
                .iter()
                .map(|value| {
                    GeneratedString::plain(format!(
                        "{}/{}",
                        value.value().container(),
                        match value.value().protocol() {
                            Protocol::Tcp => "tcp",
                            Protocol::Udp => "udp",
                            _ => "unsupported",
                        }
                    ))
                })
                .collect::<Result<Vec<_>, _>>();
            match values.and_then(|values| generated.set_expose(values)) {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
    }

    fn map_annotations_logging_and_reload(
        &mut self,
        service: &Service,
        generated: &mut GeneratedService,
        subject: &str,
    ) {
        if let Some(values) = service.annotations() {
            let field = format!("{subject}.annotations");
            let origins = collection_or_item_origins(values, service.annotations_origins());
            let values = values
                .iter()
                .map(|value| {
                    GeneratedAnnotation::new(
                        value.value().name().value().as_str(),
                        generated_string(value.value().value().value())?,
                    )
                })
                .collect::<Result<Vec<_>, _>>();
            match values.and_then(|values| generated.set_annotations(values)) {
                Ok(()) => self.exact(field, &origins),
                Err(error) => self.generation_error(&field, &error, &origins),
            }
        }
        if let Some(logging) = service.logging() {
            let field = format!("{subject}.logging");
            let value = logging.value();
            let Some(driver) = value.driver() else {
                self.unsupported(
                    &field,
                    "Compose logging requires a driver; neutral logging retains its absence",
                    logging.origins(),
                );
                return;
            };
            let options = value
                .options()
                .unwrap_or_default()
                .iter()
                .map(|option| {
                    GeneratedLoggingOption::new(
                        option.value().name().value().as_str(),
                        GeneratedLoggingOptionValue::String(generated_string(option.value().value().value())?),
                    )
                })
                .collect::<Result<Vec<_>, _>>();
            match generated_string(driver.value())
                .and_then(|driver| options.and_then(|options| GeneratedLogging::new(driver, options)))
                .and_then(|value| generated.set_logging(value))
            {
                Ok(()) if value.options().is_some() => self.exact(field, logging.origins()),
                Ok(()) => self.loss(
                    self.exporter.codes.unsupported.clone(),
                    &field,
                    ConversionKind::Approximate,
                    "generated Compose logging adds an explicit empty options mapping",
                    "neutral logging omitted options, but ComposeLens GeneratedLogging requires an options collection",
                    logging.origins(),
                ),
                Err(error) => self.generation_error(&field, &error, logging.origins()),
            }
        }
        if let Some(reload) = service.reload_action() {
            self.unsupported(
                &format!("{subject}.reload_action"),
                "Compose has no reload command or signal field; reload intent is never emitted as a lifecycle hook",
                reload.origins(),
            );
        }
    }

    fn report_unimplemented_service_fields(&mut self, service: &Service, service_subject: &str) {
        if let Some(healthcheck) = service.healthcheck() {
            self.unsupported(
                &format!("{service_subject}.healthcheck"),
                "the current Compose generation boundary does not yet expose health-check fields",
                healthcheck.origins(),
            );
        }
        for (index, dependency) in service.dependencies().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.dependencies[{index}]"),
                "the current Compose generation boundary does not yet expose service dependencies",
                dependency.origins(),
            );
        }
        for (index, grant) in service.config_grants().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.config_grants[{index}]"),
                "the current Compose generation boundary does not yet expose service config grants",
                grant.origins(),
            );
        }
        for (index, grant) in service.secret_grants().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.secret_grants[{index}]"),
                "the current Compose generation boundary does not yet expose service secret grants",
                grant.origins(),
            );
        }
    }

    fn report_compose_native_only_service_fields(&mut self, service: &Service, service_subject: &str) {
        if let Some(notification) = service.startup_notification() {
            self.unsupported(
                &format!("{service_subject}.startup_notification"),
                "Compose has no service startup-notification field",
                notification.origins(),
            );
        }
        if let Some(rootfs) = service.rootfs() {
            self.unsupported(
                &format!("{service_subject}.rootfs"),
                "Compose has no root-filesystem source field; rootfs is never rewritten as an image",
                rootfs.origins(),
            );
        }
        if let Some(arguments) = service.podman_args() {
            for (index, argument) in arguments.iter().enumerate() {
                self.unsupported(
                    &format!("{service_subject}.podman_args[{index}]"),
                    "Compose has no native Podman-argument field; authored arguments are never synthesized",
                    argument.origins(),
                );
            }
            if arguments.is_empty() {
                self.unsupported(
                    &format!("{service_subject}.podman_args"),
                    "Compose has no native Podman-argument field; authored arguments are never synthesized",
                    service.podman_args_origins(),
                );
            }
        }
    }

    fn report_service_group_loss(&mut self, group: &Sourced<boxferry_model::ServiceGroup>) {
        let group_subject = format!("service_groups.{}", group.value().name().as_str());
        self.unsupported(
            &group_subject,
            "Compose has no native structural equivalent for a runtime pod or shared namespace group",
            group.origins(),
        );
        let Some(runtime) = group.value().runtime() else {
            return;
        };
        let runtime_subject = format!("{group_subject}.runtime");
        self.unsupported(
            &runtime_subject,
            "Compose has no native service-group runtime; it is never assigned to a member service",
            runtime.origins(),
        );
        let runtime = runtime.value();
        if let Some(value) = runtime.runtime_name() {
            self.unsupported(
                &format!("{runtime_subject}.runtime_name"),
                "Compose has no native service-group runtime name",
                value.origins(),
            );
        }
        if let Some(value) = runtime.service_name() {
            self.unsupported(
                &format!("{runtime_subject}.service_name"),
                "Compose has no native service-group systemd service name",
                value.origins(),
            );
        }
        self.report_group_runtime_collection(
            &format!("{runtime_subject}.host_mappings"),
            runtime.host_mappings(),
            runtime.host_mappings_origins(),
        );
        self.report_group_runtime_collection(
            &format!("{runtime_subject}.ports"),
            runtime.ports(),
            runtime.ports_origins(),
        );
        self.report_group_runtime_collection(
            &format!("{runtime_subject}.networks"),
            runtime.networks(),
            runtime.networks_origins(),
        );
        if let Some(value) = runtime.user_namespace() {
            self.unsupported(
                &format!("{runtime_subject}.user_namespace"),
                "Compose has no native service-group user-namespace field",
                value.origins(),
            );
        }
        self.report_group_runtime_collection(
            &format!("{runtime_subject}.mounts"),
            runtime.mounts(),
            runtime.mounts_origins(),
        );
        if let Some(value) = runtime.shm_size() {
            self.unsupported(
                &format!("{runtime_subject}.shm_size"),
                "Compose has no native service-group shared-memory-size field",
                value.origins(),
            );
        }
        if let Some(value) = runtime.exit_policy() {
            self.unsupported(
                &format!("{runtime_subject}.exit_policy"),
                "Compose has no native service-group exit-policy field",
                value.origins(),
            );
        }
        if let Some(value) = runtime.stop_timeout() {
            self.unsupported(
                &format!("{runtime_subject}.stop_timeout"),
                "Compose has no native service-group stop-timeout field",
                value.origins(),
            );
        }
    }

    fn report_group_runtime_collection<T>(
        &mut self,
        subject: &str,
        values: Option<&[Sourced<T>]>,
        collection_origins: &[Provenance],
    ) {
        let Some(values) = values else {
            return;
        };
        if values.is_empty() {
            self.unsupported(
                subject,
                "Compose has no native service-group runtime collection; it is never assigned to a member service",
                collection_origins,
            );
            return;
        }
        for (index, value) in values.iter().enumerate() {
            self.unsupported(
                &format!("{subject}[{index}]"),
                "Compose has no native service-group runtime collection; it is never assigned to a member service",
                value.origins(),
            );
        }
    }

    fn map_identity(&mut self, service: &Service, generated: &mut GeneratedService, service_subject: &str) {
        match (service.user(), service.group()) {
            (Some(user), group) => {
                let mut value = user.value().expose().to_owned();
                let mut sensitive = user.value().is_sensitive();
                let mut origins = user.origins().to_vec();
                if let Some(group) = group {
                    value.push(':');
                    value.push_str(group.value().expose());
                    sensitive |= group.value().is_sensitive();
                    origins.extend_from_slice(group.origins());
                }
                let value = if sensitive {
                    GeneratedString::sensitive(value)
                } else {
                    GeneratedString::plain(value)
                };
                match value.and_then(|value| generated.set_user(value)) {
                    Ok(()) => self.exact(format!("{service_subject}.user"), &origins),
                    Err(error) => self.generation_error(&format!("{service_subject}.user"), &error, &origins),
                }
            }
            (None, Some(group)) => self.unsupported(
                &format!("{service_subject}.group"),
                "Compose combines user and primary group in one field and cannot encode a group without a user",
                group.origins(),
            ),
            (None, None) => {}
        }

        if let Some(user_namespace) = service.user_namespace() {
            let subject = format!("{service_subject}.user_namespace");
            match generated_string(user_namespace.value()).and_then(|value| generated.set_userns_mode(value)) {
                Ok(()) if is_podman_user_namespace(user_namespace.value().expose()) => self.compatibility(
                    &subject,
                    CompatibilityFeature::PodmanUserNamespaceMode,
                    user_namespace.origins(),
                ),
                Ok(()) => self.exact(subject, user_namespace.origins()),
                Err(error) => self.generation_error(&subject, &error, user_namespace.origins()),
            }
        }
        for (index, group) in service.supplementary_groups().iter().enumerate() {
            let subject = format!("{service_subject}.supplementary_groups[{index}]");
            match generated_string(group.value()).and_then(|value| generated.add_supplementary_group(value)) {
                Ok(()) => self.exact(subject, group.origins()),
                Err(error) => self.generation_error(&subject, &error, group.origins()),
            }
        }
    }

    fn compatibility(&mut self, subject: &str, feature: CompatibilityFeature, origins: &[Provenance]) {
        let Some(profile) = self.compatibility else {
            self.invalid(
                self.exporter.codes.invalid_target.clone(),
                subject,
                "Compose compatibility profile is unavailable",
                "validate the target before mapping application fields",
                origins,
            );
            return;
        };
        let rule = profile.classify(feature);
        let evidence: Vec<_> = rule.evidence().iter().map(|evidence| evidence.source()).collect();
        match rule.classification() {
            CompatibilityClassification::Supported | CompatibilityClassification::Extension => {
                self.exact(subject, origins);
            }
            CompatibilityClassification::ImplementationSpecific | CompatibilityClassification::Deprecated => {
                self.compatibility_loss(
                    subject,
                    ConversionKind::Approximate,
                    rule.classification(),
                    rule.explanation(),
                    &evidence,
                    origins,
                );
            }
            CompatibilityClassification::Unsupported | CompatibilityClassification::Unknown => {
                self.compatibility_loss(
                    subject,
                    ConversionKind::Unsupported,
                    rule.classification(),
                    rule.explanation(),
                    &evidence,
                    origins,
                );
            }
            _ => self.compatibility_loss(
                subject,
                ConversionKind::Unsupported,
                rule.classification(),
                rule.explanation(),
                &evidence,
                origins,
            ),
        }
    }

    fn compatibility_loss(
        &mut self,
        subject: &str,
        kind: ConversionKind,
        classification: CompatibilityClassification,
        reason: &str,
        evidence: &[&str],
        origins: &[Provenance],
    ) {
        let classification = format!("{classification:?}").to_ascii_lowercase();
        let mut diagnostic = Diagnostic::new(
            self.exporter.codes.compatibility.clone(),
            Severity::Warning,
            "generated Compose construct has a target-specific compatibility constraint",
        )
        .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
        .with_field(DiagnosticField::new(
            "classification",
            DiagnosticValue::plain(classification),
        ))
        .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)));
        if !evidence.is_empty() {
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "evidence",
                DiagnosticValue::plain(evidence.join(",")),
            ));
        }
        self.diagnostics.push(diagnostic);
        self.push_loss(subject, kind, self.exporter.codes.compatibility.clone(), origins);
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
            "neutral application intent is not represented in generated Compose output",
            reason,
            origins,
        );
    }

    fn generation_error(&mut self, subject: &str, error: &GenerationError, origins: &[Provenance]) {
        self.generation_failed = true;
        self.invalid(
            self.exporter.codes.generation.clone(),
            subject,
            "Compose document generation failed",
            &error.to_string(),
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
        self.push_loss(subject, kind, code, origins);
    }

    fn push_loss(&mut self, subject: &str, kind: ConversionKind, code: DiagnosticCode, origins: &[Provenance]) {
        if let Ok(mut outcome) = ConversionOutcome::loss(subject, kind, code) {
            for origin in origins {
                outcome = outcome.with_origin(origin.clone());
            }
            self.outcomes.push(outcome);
        }
    }

    fn finish(
        mut self,
    ) -> (
        Option<GeneratedComposeDocument>,
        Vec<ConversionOutcome>,
        Vec<Diagnostic>,
    ) {
        let candidate = if self.compatibility.is_some() && !self.generation_failed {
            match self.builder.take().unwrap_or_default().build(SourceId::new(1)) {
                Ok(document) => Some(document),
                Err(error) => {
                    self.generation_error("application", &error, &[]);
                    None
                }
            }
        } else {
            None
        };
        (candidate, self.outcomes, self.diagnostics)
    }
}

fn generated_string(value: &ProtectedString) -> Result<GeneratedString, GenerationError> {
    if value.is_sensitive() {
        GeneratedString::sensitive(value.expose())
    } else {
        GeneratedString::plain(value.expose())
    }
}

fn split_size(value: &str) -> Option<(&str, ShmSizeUnit)> {
    for (suffix, unit) in [
        ("b", ShmSizeUnit::B),
        ("k", ShmSizeUnit::K),
        ("m", ShmSizeUnit::M),
        ("g", ShmSizeUnit::G),
    ] {
        if let Some(amount) = value.strip_suffix(suffix) {
            return Some((amount, unit));
        }
    }
    None
}

fn split_memory(value: &str) -> Option<(&str, MemLimitUnit)> {
    for (suffix, unit) in [
        ("kb", MemLimitUnit::Kb),
        ("mb", MemLimitUnit::Mb),
        ("gb", MemLimitUnit::Gb),
        ("b", MemLimitUnit::B),
        ("k", MemLimitUnit::K),
        ("m", MemLimitUnit::M),
        ("g", MemLimitUnit::G),
    ] {
        if let Some(amount) = value.strip_suffix(suffix) {
            return Some((amount, unit));
        }
    }
    None
}

enum SecurityOptionGeneration {
    Generated(GeneratedString),
    Unsupported(&'static str),
}

fn security_option_string(option: &SecurityOption) -> Result<SecurityOptionGeneration, GenerationError> {
    Ok(match option {
        SecurityOption::AppArmor(profile) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("apparmor=", profile)?)
        }
        SecurityOption::NoNewPrivileges(enabled) => {
            SecurityOptionGeneration::Generated(GeneratedString::plain(format!("no-new-privileges:{enabled}"))?)
        }
        SecurityOption::SeccompProfile(profile) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("seccomp=", profile)?)
        }
        SecurityOption::SecurityLabelDisable(enabled) => {
            if *enabled {
                SecurityOptionGeneration::Generated(GeneratedString::plain("label:disable")?)
            } else {
                SecurityOptionGeneration::Unsupported(
                    "Compose has no exact `security_opt` spelling that re-enables SELinux label separation",
                )
            }
        }
        SecurityOption::SecurityLabelFileType(file_type) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("label:filetype:", file_type)?)
        }
        SecurityOption::SecurityLabelLevel(level) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("label:level:", level)?)
        }
        SecurityOption::SecurityLabelNested(enabled) => {
            if *enabled {
                SecurityOptionGeneration::Generated(GeneratedString::plain("label:nested")?)
            } else {
                SecurityOptionGeneration::Unsupported(
                    "Compose has no exact `security_opt` spelling that disables nested SELinux labeling",
                )
            }
        }
        SecurityOption::SecurityLabelType(label_type) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("label:type:", label_type)?)
        }
        SecurityOption::Mask(paths) => SecurityOptionGeneration::Generated(prefixed_generated_string("mask=", paths)?),
        SecurityOption::Unmask(paths) => {
            SecurityOptionGeneration::Generated(prefixed_generated_string("unmask=", paths)?)
        }
        _ => SecurityOptionGeneration::Unsupported("security-option variant is newer than this Compose adapter"),
    })
}

fn prefixed_generated_string(prefix: &str, value: &ProtectedString) -> Result<GeneratedString, GenerationError> {
    let raw = format!("{prefix}{}", value.expose());
    if value.is_sensitive() {
        GeneratedString::sensitive(raw)
    } else {
        GeneratedString::plain(raw)
    }
}

fn collection_or_item_origins<T>(values: &[Sourced<T>], collection_origins: &[Provenance]) -> Vec<Provenance> {
    let mut origins = collection_origins.to_vec();
    for value in values {
        extend_origins(&mut origins, value.origins());
    }
    origins
}

fn combined_origins(first: &[Provenance], second: &[Provenance]) -> Vec<Provenance> {
    let mut origins = first.to_vec();
    extend_origins(&mut origins, second);
    origins
}

fn resource_limit_collection_origins(
    values: &[Sourced<boxferry_model::ResourceLimit>],
    collection_origins: &[Provenance],
) -> Vec<Provenance> {
    let mut origins = collection_or_item_origins(values, collection_origins);
    for value in values {
        for nested in [value.value().soft(), value.value().hard()].into_iter().flatten() {
            extend_origins(&mut origins, nested.origins());
        }
    }
    origins
}

fn device_collection_origins(values: &[Sourced<Device>], collection_origins: &[Provenance]) -> Vec<Provenance> {
    let mut origins = collection_or_item_origins(values, collection_origins);
    for value in values {
        if let Device::Long {
            source,
            target,
            permissions,
        } = value.value()
        {
            for nested in [source.as_ref(), target.as_ref(), permissions.as_ref()]
                .into_iter()
                .flatten()
            {
                extend_origins(&mut origins, nested.origins());
            }
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

fn is_compose_managed_label(name: &str) -> bool {
    name.starts_with("com.docker.compose.")
}

fn is_podman_user_namespace(value: &str) -> bool {
    ["keep-id", "auto", "nomap"]
        .iter()
        .any(|mode| value == *mode || value.strip_prefix(mode).is_some_and(|suffix| suffix.starts_with(':')))
}

fn implementation_version(version: PlatformVersion) -> Option<ImplementationVersion> {
    Some(ImplementationVersion::new(
        u32::try_from(version.major()).ok()?,
        u32::try_from(version.minor()).ok()?,
        u32::try_from(version.patch()).ok()?,
    ))
}
