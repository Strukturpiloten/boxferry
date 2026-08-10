//! Neutral-application-to-Compose planning through `ComposeLens`'s validated generator.

use std::convert::TryFrom;

use boxferry_engine::{
    ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue,
    ExportAdapter, InvalidDiagnosticCode, PlanError, PlatformVersion, Severity, TargetProfile,
};
use boxferry_model::{
    Application, Command, Device, EnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue, HostAddressKind,
    MountSource, ProtectedString, Protocol, Provenance, ProvenanceKind, ResourceOwnership, RestartPolicy,
    SecurityOption, SelinuxRelabel, Service, Sourced,
};
use compose_lens::{
    render::{
        ComposeDocumentBuilder, GeneratedCommand, GeneratedComposeDocument, GeneratedDns, GeneratedDnsSearch,
        GeneratedEnvironment, GeneratedEnvironmentFile, GeneratedEnvironmentFileFormat, GeneratedExtraHost,
        GeneratedLabel, GeneratedMount, GeneratedNetworkAttachment, GeneratedPort, GeneratedProtocol,
        GeneratedResource, GeneratedRestartPolicy, GeneratedSelinux, GeneratedService, GeneratedString,
        GenerationError,
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
                invalid_target: DiagnosticCode::new("BFC0006")?,
                unsupported: DiagnosticCode::new("BFC0007")?,
                generation: DiagnosticCode::new("BFC0008")?,
                compatibility: DiagnosticCode::new("BFC0009")?,
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
            let subject = format!("networks.{}", network.value().name().as_str());
            match self.map_resource(
                network.value().name().as_str(),
                network.value().ownership(),
                network.origins(),
                &subject,
            ) {
                Some(resource) => {
                    if let Err(error) = builder.add_network(resource) {
                        self.generation_error(&subject, &error, network.origins());
                    }
                }
                None => self.generation_failed = true,
            }
        }
        for volume in self.application.volumes() {
            let subject = format!("volumes.{}", volume.value().name().as_str());
            match self.map_resource(
                volume.value().name().as_str(),
                volume.value().ownership(),
                volume.origins(),
                &subject,
            ) {
                Some(resource) => {
                    if let Err(error) = builder.add_volume(resource) {
                        self.generation_error(&subject, &error, volume.origins());
                    }
                }
                None => self.generation_failed = true,
            }
        }

        for acquisition in self.application.image_acquisitions() {
            self.unsupported(
                &format!("image_acquisitions.{}", acquisition.value().name().as_str()),
                "ComposeLens 0.1.16 generation does not expose image-acquisition declarations",
                acquisition.origins(),
            );
        }
        for build in self.application.image_builds() {
            self.unsupported(
                &format!("image_builds.{}", build.value().name().as_str()),
                "ComposeLens 0.1.16 generation does not expose build declarations",
                build.origins(),
            );
        }

        for config in self.application.configs() {
            self.unsupported(
                &format!("configs.{}", config.value().name().as_str()),
                "ComposeLens 0.1.13 generation does not yet expose top-level config definitions",
                config.origins(),
            );
        }
        for secret in self.application.secrets() {
            self.unsupported(
                &format!("secrets.{}", secret.value().name().as_str()),
                "ComposeLens 0.1.13 generation does not yet expose top-level secret definitions",
                secret.origins(),
            );
        }
        for group in self.application.service_groups() {
            self.unsupported(
                &format!("service_groups.{}", group.value().name().as_str()),
                "Compose has no native structural equivalent for a runtime pod or shared namespace group",
                group.origins(),
            );
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
        if !self.map_image(service, sourced.origins(), &mut generated, &service_subject) {
            return None;
        }
        self.map_restart_policy(service, &mut generated, &service_subject);
        self.map_command(service, &mut generated, &service_subject);
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
                    for alias in network.value().aliases() {
                        if let Err(error) = value.add_alias(alias) {
                            self.generation_error(&subject, &error, network.origins());
                            valid = false;
                            break;
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

    fn report_unimplemented_service_fields(&mut self, service: &Service, service_subject: &str) {
        let reason =
            "this neutral field is retained but BoxFerry's Compose exporter does not yet implement its mapping";
        if let Some(value) = service.hostname() {
            self.unsupported(&format!("{service_subject}.hostname"), reason, value.origins());
        }
        if let Some(value) = service.pids_limit() {
            self.unsupported(&format!("{service_subject}.pids_limit"), reason, value.origins());
        }
        if let Some(value) = service.shm_size() {
            self.unsupported(&format!("{service_subject}.shm_size"), reason, value.origins());
        }
        if let Some(value) = service.stop_signal() {
            self.unsupported(&format!("{service_subject}.stop_signal"), reason, value.origins());
        }
        for (field, values, collection_origins) in [
            ("cap_add", service.cap_add(), service.cap_add_origins()),
            ("cap_drop", service.cap_drop(), service.cap_drop_origins()),
            ("tmpfs", service.tmpfs(), service.tmpfs_origins()),
        ] {
            if let Some(values) = values {
                let origins = collection_or_item_origins(values, collection_origins);
                self.unsupported(&format!("{service_subject}.{field}"), reason, &origins);
            }
        }
        if let Some(values) = service.sysctls() {
            let origins = collection_or_item_origins(values, service.sysctls_origins());
            self.unsupported(&format!("{service_subject}.sysctls"), reason, &origins);
        }
        if let Some(values) = service.ulimits() {
            let origins = resource_limit_collection_origins(values, service.ulimits_origins());
            self.unsupported(&format!("{service_subject}.ulimits"), reason, &origins);
        }
        if let Some(values) = service.devices() {
            let origins = device_collection_origins(values, service.devices_origins());
            self.unsupported(&format!("{service_subject}.devices"), reason, &origins);
        }
        if let Some(healthcheck) = service.healthcheck() {
            self.unsupported(
                &format!("{service_subject}.healthcheck"),
                "ComposeLens 0.1.13 generation does not yet expose health-check fields",
                healthcheck.origins(),
            );
        }
        for (index, dependency) in service.dependencies().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.dependencies[{index}]"),
                "ComposeLens 0.1.13 generation does not yet expose service dependencies",
                dependency.origins(),
            );
        }
        for (index, grant) in service.config_grants().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.config_grants[{index}]"),
                "ComposeLens 0.1.13 generation does not yet expose service config grants",
                grant.origins(),
            );
        }
        for (index, grant) in service.secret_grants().iter().enumerate() {
            self.unsupported(
                &format!("{service_subject}.secret_grants[{index}]"),
                "ComposeLens 0.1.13 generation does not yet expose service secret grants",
                grant.origins(),
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
