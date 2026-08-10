//! Compose-to-application mapping with explicit fidelity decisions.

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    Application, BuildAttestation, BuildContext, BuildSettingValues, BuildSourceDeclaration, BuildSyntax, Command,
    Config, ConfigMaterial, Device, EnvironmentFile as NeutralEnvironmentFile,
    EnvironmentFileFormat as NeutralEnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue,
    EnvironmentVariable, Healthcheck, HealthcheckCommand, HealthcheckDuration as NeutralHealthcheckDuration,
    HealthcheckRetries as NeutralHealthcheckRetries, HostAddress, HostMapping, Identifier, ImageArtifactAssignment,
    ImageBuild, ImageBuildSetting, ImageReference, KernelParameter, MetadataLabel, ModelError, Mount, MountSource,
    Network, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, ResourceGrant, ResourceGrantSyntax,
    ResourceLimit, ResourceOwnership, RestartPolicy as NeutralRestartPolicy, Secret, SecretMaterial, SecurityOption,
    SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition, SourceBuildSecret, SourceBuildSetting,
    SourceSpan, Sourced, Volume,
};
use compose_lens::merge::MergeProvenance;
use compose_lens::model::{
    BooleanValue, ComposeScalar, ConfigDefinition, DependencyCondition as ComposeDependencyCondition,
    EnvironmentFileFormatKind, HealthcheckDuration as ComposeHealthcheckDuration,
    HealthcheckRetries as ComposeHealthcheckRetries, HealthcheckTest, HealthcheckTestKind, HostnameKind, LimitValue,
    LongPort, LongVolumeMount, MountType, NetworkDefinition, PidsLimitKind, Port as ComposePort,
    RestartPolicyKind as ComposeRestartPolicyKind, SecretDefinition, SecurityOptionKind,
    SelinuxRelabel as ComposeSelinuxRelabel, ServiceNetwork, ServiceNetworks, ShmSizeKind, ShmSizeUnit,
    ShortDeviceKind, ShortPort, ShortVolumeMount, TmpfsItemKind, VolumeDefinition, VolumeMount,
};
use compose_lens::project::{
    ProjectBuild, ProjectBuildAdditionalContexts, ProjectBuildArgs, ProjectBuildDefinition, ProjectBuildExtraHosts,
    ProjectBuildLabels, ProjectBuildNoCacheFilter, ProjectBuildSsh, ProjectDependsOn, ProjectDevice, ProjectDns,
    ProjectDnsSearch, ProjectEnvironment, ProjectEnvironmentFile, ProjectFieldReference, ProjectGrant,
    ProjectHealthcheck, ProjectLabels, ProjectResource, ProjectService, ProjectSysctls, ProjectTmpfs,
    ProjectUlimitValue, ProjectValue, ProjectView, build_project_view,
};
use compose_lens::source::SourceSpan as ComposeSpan;

use crate::ComposeSource;

/// Maps an explicitly processed Compose source into `BoxFerry`'s neutral model.
#[derive(Clone, Debug)]
pub struct ComposeImporter {
    codes: Codes,
}

impl ComposeImporter {
    /// Creates an importer and validates its stable machine-readable diagnostic codes.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only when a code embedded in this adapter is invalid.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            codes: Codes {
                invalid_model: DiagnosticCode::new("BFC0001")?,
                profile_required: DiagnosticCode::new("BFC0002")?,
                profile_mismatch: DiagnosticCode::new("BFC0003")?,
                unsupported: DiagnosticCode::new("BFC0004")?,
                invalid_value: DiagnosticCode::new("BFC0005")?,
            },
        })
    }
}

impl ImportAdapter for ComposeImporter {
    type Source = ComposeSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let mut mapping = Mapping::new(&self.codes, source);
        let project_result = build_project_view(source.project(), source.profile_selection());
        let Some(view) = project_result.view() else {
            mapping.invalid_optional_span(
                self.codes.profile_mismatch.clone(),
                "services.profiles",
                "profile selection does not belong to the merged Compose project",
                "profile selection",
                source.project().root().provenance().effective_source(),
            );
            return ImportResult::new(None, mapping.outcomes, mapping.diagnostics);
        };
        mapping.report_project_diagnostics(project_result.diagnostics());

        let application_name = view.name().map_or_else(
            || source.fallback_application_name().clone(),
            |name| {
                Identifier::new(name.value().clone()).unwrap_or_else(|error| {
                    mapping.invalid_model_optional("application.name", &error, name.effective_source());
                    source.fallback_application_name().clone()
                })
            },
        );
        let mut application = Application::new(application_name);
        mapping.exact_provenance("application", view.provenance());

        let profiles_valid = mapping.validate_profiles(view, source);

        for definition in view.volumes() {
            if let Some(volume) = mapping.map_volume_definition(definition) {
                if let Err(error) = application.add_volume(volume) {
                    mapping.invalid_model_optional("volumes", &error, definition.definition().effective_source());
                }
            }
        }
        for definition in view.networks() {
            if let Some(network) = mapping.map_network_definition(definition) {
                if let Err(error) = application.add_network(network) {
                    mapping.invalid_model_optional("networks", &error, definition.definition().effective_source());
                }
            }
        }
        for definition in view.configs() {
            if let Some(config) = mapping.map_config_definition(definition) {
                if let Err(error) = application.add_config(config) {
                    mapping.invalid_model_optional("configs", &error, definition.definition().effective_source());
                }
            }
        }
        for definition in view.secrets() {
            if let Some(secret) = mapping.map_secret_definition(definition) {
                if let Err(error) = application.add_secret(secret) {
                    mapping.invalid_model_optional("secrets", &error, definition.definition().effective_source());
                }
            }
        }
        for native_service in view.services() {
            let active = if native_service
                .profiles()
                .is_none_or(|profiles| profiles.value().is_empty())
            {
                true
            } else {
                profiles_valid
                    && source
                        .profile_selection()
                        .is_some_and(|selection| selection.is_active(native_service.name().value()))
            };
            if !active {
                continue;
            }
            if let Some(service) = mapping.map_service(native_service) {
                let (mut service, origins) = service.into_parts();
                if let Some(build) = mapping.map_service_build(service.name(), service.image(), native_service) {
                    let build_name = build.value().name().clone();
                    let build_origins = build.origins().to_vec();
                    if let Err(error) = application.add_image_build(build) {
                        mapping.invalid_model_optional("image_builds", &error, view.provenance().effective_source());
                    } else {
                        let mut reference = Sourced::generated(build_name);
                        for origin in build_origins {
                            reference.add_origin(origin);
                        }
                        service.set_image_build(reference);
                    }
                }
                let mut service = Sourced::generated(service);
                for origin in origins {
                    service.add_origin(origin);
                }
                if let Err(error) = application.add_service(service) {
                    mapping.invalid_model_optional("services", &error, view.provenance().effective_source());
                }
            }
        }

        mapping.report_document_unsupported(view);
        ImportResult::new(Some(application), mapping.outcomes, mapping.diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_model: DiagnosticCode,
    profile_required: DiagnosticCode,
    profile_mismatch: DiagnosticCode,
    unsupported: DiagnosticCode,
    invalid_value: DiagnosticCode,
}

enum SecurityOptionMapping {
    Exact(SecurityOption),
    Invalid(&'static str),
    Unsupported(&'static str),
}

struct Mapping<'a> {
    codes: &'a Codes,
    source: &'a ComposeSource,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Mapping<'a> {
    const fn new(codes: &'a Codes, source: &'a ComposeSource) -> Self {
        Self {
            codes,
            source,
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn validate_profiles(&mut self, view: &ProjectView, source: &ComposeSource) -> bool {
        if view
            .services()
            .iter()
            .all(|service| service.profiles().is_none_or(|profiles| profiles.value().is_empty()))
        {
            return true;
        }
        let Some(selection) = source.profile_selection() else {
            self.invalid_optional_span(
                self.codes.profile_required.clone(),
                "services.profiles",
                "profiled Compose services require an explicit ComposeLens profile selection",
                "profile selection",
                view.provenance().effective_source(),
            );
            return false;
        };
        if !selection.is_valid() {
            self.invalid_optional_span(
                self.codes.profile_mismatch.clone(),
                "services.profiles",
                "profile selection is invalid or does not belong to the merged Compose project",
                "profile selection",
                view.provenance().effective_source(),
            );
            return false;
        }
        true
    }

    fn map_service(&mut self, native: &ProjectService) -> Option<Sourced<Service>> {
        let subject = format!("services.{}", native.name().value());
        let name = self.identifier_optional(
            &format!("{subject}.name"),
            native.name().value(),
            native.name().effective_source(),
        )?;
        let mut service = Service::new(name);

        self.map_runtime_name(&subject, native, &mut service);
        self.map_restart_policy(&subject, native, &mut service);
        if let Some(image) = native.image() {
            match ImageReference::parse(image.value().raw()) {
                Ok(value) => {
                    service.set_image(self.sourced_provenance(value, image.provenance()));
                    self.exact_provenance(format!("{subject}.image"), image.provenance());
                }
                Err(error) => {
                    self.invalid_model_optional(&format!("{subject}.image"), &error, image.effective_source());
                }
            }
        }
        if let Some(command) = native.command() {
            if let Some(value) = Self::map_command(command.value(), command.is_sensitive()) {
                service.set_command(self.sourced_provenance(value, command.provenance()));
                self.exact_provenance(format!("{subject}.command"), command.provenance());
            }
        }
        if let Some(healthcheck) = native.healthcheck() {
            service.set_healthcheck(self.map_healthcheck(&subject, healthcheck));
        }
        self.map_execution_context(&subject, native, &mut service);
        self.map_released_container_settings(&subject, native, &mut service);
        self.map_dns(&subject, native, &mut service);
        self.map_security_options(&subject, native, &mut service);
        self.map_service_environment(&subject, native, &mut service);
        self.map_service_labels(&subject, native, &mut service);
        if let Some(extra_hosts) = native.extra_hosts() {
            for (index, entry) in extra_hosts.value().entries().iter().enumerate() {
                let host_subject = format!("{subject}.extra_hosts[{index}]");
                let Some(hostname) = self.identifier_optional(
                    &host_subject,
                    entry.hostname().value(),
                    entry.hostname().effective_source(),
                ) else {
                    continue;
                };
                let address = match HostAddress::new(entry.address().value().raw()) {
                    Ok(address) => address,
                    Err(error) => {
                        self.invalid_model_optional(&host_subject, &error, entry.address().effective_source());
                        continue;
                    }
                };
                let mapping = self.sourced_host_mapping(
                    HostMapping::new(hostname, address),
                    entry.hostname().sources(),
                    entry.address().provenance(),
                );
                self.exact_origins(host_subject, mapping.origins());
                service.add_host_mapping(mapping);
            }
        }
        if let Some(ports) = native.ports() {
            for (index, port) in ports.value().iter().enumerate() {
                let port_subject = format!("{subject}.ports[{index}]");
                if let Some(value) = self.map_port(&port_subject, port.value()) {
                    service.add_port(self.sourced_provenance(value, port.provenance()));
                    self.exact_provenance(port_subject, port.provenance());
                }
            }
        }
        if let Some(volumes) = native.volumes() {
            for (index, mount) in volumes.value().iter().enumerate() {
                let mount_subject = format!("{subject}.volumes[{index}]");
                if let Some(value) = self.map_mount(&mount_subject, mount.value()) {
                    service.add_mount(self.sourced_provenance(value, mount.provenance()));
                    self.exact_provenance(mount_subject, mount.provenance());
                }
            }
        }
        if let Some(configs) = native.configs() {
            self.map_service_grants(&subject, "configs", configs, false, &mut service);
        }
        if let Some(secrets) = native.secrets() {
            self.map_service_grants(&subject, "secrets", secrets, true, &mut service);
        }
        if let Some(networks) = native.networks() {
            self.map_service_networks(&subject, networks.value(), networks.provenance(), &mut service);
        }
        if let Some(profiles) = native.profiles() {
            if !profiles.value().is_empty() {
                self.exact_provenance(format!("{subject}.profiles"), profiles.provenance());
            }
        }
        if let Some(dependencies) = native.depends_on() {
            self.map_service_dependencies(&subject, dependencies, &mut service);
        }

        self.report_service_unsupported(&subject, native);
        self.exact_provenance(&subject, native.provenance());
        Some(self.sourced_provenance(service, native.provenance()))
    }

    fn map_runtime_name(&mut self, subject: &str, native: &ProjectService, service: &mut Service) {
        let Some(container_name) = native.container_name() else {
            return;
        };
        service.set_runtime_name(self.sourced_provenance(
            ProtectedString::plain(container_name.value()),
            container_name.provenance(),
        ));
        self.exact_provenance(format!("{subject}.container_name"), container_name.provenance());
    }

    fn map_service_build(
        &mut self,
        service_name: &Identifier,
        service_image: Option<&Sourced<ImageReference>>,
        native: &ProjectService,
    ) -> Option<Sourced<ImageBuild>> {
        let build = native.build()?;
        let subject = format!("services.{}.build", service_name.as_str());
        let name = match Identifier::new(format!("{}-build", service_name.as_str())) {
            Ok(name) => name,
            Err(error) => {
                self.invalid_model_optional(&subject, &error, build.effective_source());
                return None;
            }
        };
        let mut neutral = ImageBuild::new(name);
        match build.value() {
            ProjectBuild::Context(context) => {
                neutral.set_source_declaration(self.sourced_provenance(
                    BuildSourceDeclaration::Scalar(Self::protected(context.value(), context.is_sensitive())),
                    context.provenance(),
                ));
                self.exact_provenance(&subject, context.provenance());
            }
            ProjectBuild::Definition(definition) => self.map_build_definition(&subject, definition, &mut neutral),
            _ => self.unsupported_optional(
                &subject,
                "build declaration variant is newer than this Compose adapter",
                build.effective_source(),
            ),
        }
        Self::add_service_image_build_tag(&mut neutral, service_image);
        Some(self.sourced_provenance(neutral, build.provenance()))
    }

    fn add_service_image_build_tag(build: &mut ImageBuild, service_image: Option<&Sourced<ImageReference>>) {
        let Some(service_image) = service_image else { return };
        let image = service_image.value().as_str();
        let mut settings = build.settings().map_or_else(Vec::new, <[_]>::to_vec);
        let duplicate = settings.iter().any(|setting| {
            matches!(
                setting.value(),
                ImageBuildSetting::ImageTags(values)
                    if values.values().iter().any(|value| value.value().expose() == image)
            )
        });
        if !duplicate {
            let mut tag = Sourced::generated(ProtectedString::plain(image));
            for origin in service_image.origins() {
                tag.add_origin(origin.clone());
            }
            let mut setting = Sourced::generated(ImageBuildSetting::ImageTags(BuildSettingValues::new(
                BuildSyntax::Scalar,
                vec![tag],
            )));
            for origin in service_image.origins() {
                setting.add_origin(origin.clone());
            }
            settings.insert(0, setting);
        }
        build.set_settings(settings);
    }

    fn map_build_definition(&mut self, subject: &str, definition: &ProjectBuildDefinition, build: &mut ImageBuild) {
        let mut settings = Vec::new();
        let mut overlap = Vec::new();

        self.map_build_direct_settings(definition, &mut settings, &mut overlap);
        self.map_build_boolean_settings(subject, definition, &mut settings);
        self.map_build_attestations(definition, &mut settings);
        self.map_build_string_collections(definition, &mut settings, &mut overlap);
        self.map_build_additional_contexts(definition.additional_contexts(), &mut settings);
        self.map_build_args(definition.args(), &mut settings, &mut overlap);
        self.map_build_labels(definition.labels(), &mut settings, &mut overlap);
        self.map_build_extra_hosts(definition.extra_hosts(), &mut settings);
        self.map_build_no_cache_filter(definition.no_cache_filter(), &mut settings);
        self.map_build_ssh(definition.ssh(), &mut settings);
        self.map_build_secrets(definition.secrets(), &mut settings);
        self.map_build_ulimits(definition.ulimits(), &mut settings);
        self.report_project_fields(subject, "build field", definition.unmodeled_fields());

        settings.sort_by_key(|setting| {
            setting
                .origins()
                .last()
                .and_then(Provenance::span)
                .map_or(usize::MAX, SourceSpan::start)
        });
        build.set_source_declaration(Sourced::generated(BuildSourceDeclaration::Structured(settings)));
        if !overlap.is_empty() {
            build.set_settings(overlap);
        }
    }

    fn map_build_direct_settings(
        &self,
        definition: &ProjectBuildDefinition,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
        overlap: &mut Vec<Sourced<ImageBuildSetting>>,
    ) {
        if let Some(context) = definition.context() {
            settings.push(self.sourced_provenance(
                SourceBuildSetting::Context(Self::protected(context.value(), context.is_sensitive())),
                context.provenance(),
            ));
        }
        if let Some(dockerfile) = definition.dockerfile() {
            let value = Self::protected(dockerfile.value(), dockerfile.is_sensitive());
            settings
                .push(self.sourced_provenance(SourceBuildSetting::RecipeFile(value.clone()), dockerfile.provenance()));
            overlap.push(self.sourced_provenance(ImageBuildSetting::RecipeFile(value), dockerfile.provenance()));
        }
        if let Some(inline) = definition.dockerfile_inline() {
            settings.push(self.sourced_provenance(
                SourceBuildSetting::InlineRecipe(Self::protected(inline.value(), inline.is_sensitive())),
                inline.provenance(),
            ));
        }
        if let Some(target) = definition.target() {
            let value = Self::protected(target.value(), target.is_sensitive());
            settings.push(self.sourced_provenance(SourceBuildSetting::Target(value.clone()), target.provenance()));
            overlap.push(self.sourced_provenance(ImageBuildSetting::Target(value), target.provenance()));
        }
        if let Some(network) = definition.network() {
            settings.push(self.sourced_provenance(
                SourceBuildSetting::Network(Self::protected(network.value(), network.is_sensitive())),
                network.provenance(),
            ));
        }
        if let Some(isolation) = definition.isolation() {
            settings.push(self.sourced_provenance(
                SourceBuildSetting::Isolation(Self::protected(isolation.value(), isolation.is_sensitive())),
                isolation.provenance(),
            ));
        }
        if let Some(shm_size) = definition.shm_size() {
            settings.push(self.sourced_provenance(
                SourceBuildSetting::ShmSize(Self::protected(shm_size.value().raw().value(), shm_size.is_sensitive())),
                shm_size.provenance(),
            ));
        }
    }

    fn map_build_boolean_settings(
        &mut self,
        subject: &str,
        definition: &ProjectBuildDefinition,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        if let Some(privileged) = definition.privileged() {
            if let BooleanValue::Literal(value) = privileged.value() {
                settings.push(self.sourced_provenance(SourceBuildSetting::Privileged(*value), privileged.provenance()));
            } else {
                self.invalid_value_optional(
                    &format!("{subject}.privileged"),
                    "build privileged expression was not resolved",
                    privileged.effective_source(),
                );
            }
        }
        if let Some(pull) = definition.pull() {
            if let BooleanValue::Literal(value) = pull.value() {
                settings.push(self.sourced_provenance(SourceBuildSetting::Pull(*value), pull.provenance()));
            } else {
                self.invalid_value_optional(
                    &format!("{subject}.pull"),
                    "build pull expression was not resolved",
                    pull.effective_source(),
                );
            }
        }
        if let Some(no_cache) = definition.no_cache() {
            match no_cache.value() {
                compose_lens::model::BuildNoCache::Boolean(value) => {
                    settings.push(self.sourced_provenance(SourceBuildSetting::NoCache(*value), no_cache.provenance()));
                }
                compose_lens::model::BuildNoCache::String(_) => self.unsupported_optional(
                    &format!("{subject}.no_cache"),
                    "string build no_cache values have no equivalent boolean neutral value",
                    no_cache.effective_source(),
                ),
            }
        }
    }

    fn map_build_attestations(
        &self,
        definition: &ProjectBuildDefinition,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        if let Some(sbom) = definition.sbom() {
            let value = match sbom.value() {
                compose_lens::model::BuildSbom::Boolean(value) => BuildAttestation::Boolean(*value),
                compose_lens::model::BuildSbom::String(value) => {
                    BuildAttestation::Value(Self::protected(value, sbom.is_sensitive()))
                }
            };
            settings.push(self.sourced_provenance(SourceBuildSetting::Sbom(value), sbom.provenance()));
        }
        if let Some(provenance) = definition.provenance() {
            let value = match provenance.value() {
                compose_lens::model::BuildProvenance::Boolean(value) => BuildAttestation::Boolean(*value),
                compose_lens::model::BuildProvenance::String(value) => {
                    BuildAttestation::Value(Self::protected(value, provenance.is_sensitive()))
                }
            };
            settings.push(self.sourced_provenance(SourceBuildSetting::Provenance(value), provenance.provenance()));
        }
    }

    fn map_build_string_collections(
        &self,
        definition: &ProjectBuildDefinition,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
        overlap: &mut Vec<Sourced<ImageBuildSetting>>,
    ) {
        self.map_build_string_list(
            definition.entitlements(),
            BuildSyntax::Sequence,
            SourceBuildSetting::Entitlements,
            settings,
        );
        self.map_build_string_list(
            definition.cache_from(),
            BuildSyntax::Sequence,
            SourceBuildSetting::CacheFrom,
            settings,
        );
        self.map_build_string_list(
            definition.cache_to(),
            BuildSyntax::Sequence,
            SourceBuildSetting::CacheTo,
            settings,
        );
        self.map_build_string_list(
            definition.platforms(),
            BuildSyntax::Sequence,
            SourceBuildSetting::Platforms,
            settings,
        );
        self.map_build_string_list(
            definition.tags(),
            BuildSyntax::Sequence,
            SourceBuildSetting::Tags,
            settings,
        );
        if let Some(tags) = definition.tags() {
            let values = tags
                .value()
                .iter()
                .map(|value| {
                    self.sourced_provenance(Self::protected(value.value(), value.is_sensitive()), value.provenance())
                })
                .collect();
            overlap.push(self.sourced_provenance(
                ImageBuildSetting::ImageTags(BuildSettingValues::new(BuildSyntax::Sequence, values)),
                tags.provenance(),
            ));
        }
    }

    fn map_build_string_list(
        &self,
        native: Option<&ProjectValue<Vec<ProjectValue<String>>>>,
        syntax: BuildSyntax,
        constructor: fn(BuildSettingValues<ProtectedString>) -> SourceBuildSetting,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let values = native
            .value()
            .iter()
            .map(|value| {
                self.sourced_provenance(Self::protected(value.value(), value.is_sensitive()), value.provenance())
            })
            .collect();
        settings.push(self.sourced_provenance(
            constructor(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_additional_contexts(
        &self,
        native: Option<&ProjectValue<ProjectBuildAdditionalContexts>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values) = match native.value() {
            ProjectBuildAdditionalContexts::Map(entries) => (
                BuildSyntax::Mapping,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_project_key_value(
                            BuildContext::new(
                                Self::protected(entry.name().value(), entry.name().is_sensitive()),
                                Self::protected(
                                    scalar_text(entry.value().value()).as_str(),
                                    entry.value().is_sensitive(),
                                ),
                            ),
                            entry.name().sources(),
                            entry.value().provenance(),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            ProjectBuildAdditionalContexts::List(entries) => (
                BuildSyntax::Sequence,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_provenance(
                            BuildContext::new(
                                Self::protected(entry.value(), entry.is_sensitive()),
                                ProtectedString::plain(""),
                            ),
                            entry.provenance(),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => return,
        };
        settings.push(self.sourced_provenance(
            SourceBuildSetting::AdditionalContexts(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_args(
        &self,
        native: Option<&ProjectValue<ProjectBuildArgs>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
        overlap: &mut Vec<Sourced<ImageBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values, overlap_values) = match native.value() {
            ProjectBuildArgs::Map(entries) => (
                BuildSyntax::Mapping,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_project_key_value(
                            ImageArtifactAssignment::new(
                                Self::protected(entry.name().value(), entry.name().is_sensitive()),
                                scalar_option(entry.value().value())
                                    .map(|value| Self::protected(&value, entry.value().is_sensitive())),
                            ),
                            entry.name().sources(),
                            entry.value().provenance(),
                        )
                    })
                    .collect::<Vec<_>>(),
                entries
                    .iter()
                    .filter(|entry| scalar_option(entry.value().value()).is_some())
                    .map(|entry| {
                        self.sourced_project_key_value(
                            ImageArtifactAssignment::new(
                                Self::protected(entry.name().value(), entry.name().is_sensitive()),
                                scalar_option(entry.value().value())
                                    .map(|value| Self::protected(&value, entry.value().is_sensitive())),
                            ),
                            entry.name().sources(),
                            entry.value().provenance(),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            ProjectBuildArgs::List(entries) => (
                BuildSyntax::Sequence,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_provenance(
                            ImageArtifactAssignment::new(Self::protected(entry.value(), entry.is_sensitive()), None),
                            entry.provenance(),
                        )
                    })
                    .collect(),
                entries
                    .iter()
                    .filter_map(|entry| {
                        let (name, value) = literal_assignment(entry.value())?;
                        Some(self.sourced_provenance(
                            ImageArtifactAssignment::new(
                                Self::protected(name, entry.is_sensitive()),
                                Some(Self::protected(value, entry.is_sensitive())),
                            ),
                            entry.provenance(),
                        ))
                    })
                    .collect(),
            ),
            _ => return,
        };
        let source_values = BuildSettingValues::new(syntax, values.clone());
        settings.push(self.sourced_provenance(SourceBuildSetting::Arguments(source_values), native.provenance()));
        if !overlap_values.is_empty() {
            overlap.push(self.sourced_provenance(
                ImageBuildSetting::BuildArguments(BuildSettingValues::new(syntax, overlap_values)),
                native.provenance(),
            ));
        }
    }

    fn map_build_labels(
        &self,
        native: Option<&ProjectValue<ProjectBuildLabels>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
        overlap: &mut Vec<Sourced<ImageBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values) = match native.value() {
            ProjectBuildLabels::Map(entries) => (
                BuildSyntax::Mapping,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_project_key_value(
                            ImageArtifactAssignment::new(
                                Self::protected(entry.name().value(), entry.name().is_sensitive()),
                                scalar_option(entry.value().value())
                                    .map(|value| Self::protected(&value, entry.value().is_sensitive())),
                            ),
                            entry.name().sources(),
                            entry.value().provenance(),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            ProjectBuildLabels::List(entries) => (
                BuildSyntax::Sequence,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_provenance(
                            ImageArtifactAssignment::new(Self::protected(entry.value(), entry.is_sensitive()), None),
                            entry.provenance(),
                        )
                    })
                    .collect(),
            ),
            _ => return,
        };
        settings.push(self.sourced_provenance(
            SourceBuildSetting::Labels(BuildSettingValues::new(syntax, values.clone())),
            native.provenance(),
        ));
        overlap.push(self.sourced_provenance(
            ImageBuildSetting::Labels(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_extra_hosts(
        &self,
        native: Option<&ProjectValue<ProjectBuildExtraHosts>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values) = match native.value() {
            ProjectBuildExtraHosts::List(entries) => (
                BuildSyntax::Sequence,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_provenance(
                            ImageArtifactAssignment::new(Self::protected(entry.value(), entry.is_sensitive()), None),
                            entry.provenance(),
                        )
                    })
                    .collect(),
            ),
            ProjectBuildExtraHosts::Map(entries) => (
                BuildSyntax::Mapping,
                entries
                    .iter()
                    .flat_map(|entry| match entry.addresses() {
                        compose_lens::project::ProjectBuildExtraHostAddresses::Scalar(value) => {
                            vec![self.sourced_project_key_value(
                                ImageArtifactAssignment::new(
                                    Self::protected(entry.hostname().value(), entry.hostname().is_sensitive()),
                                    Some(Self::protected(value.value(), value.is_sensitive())),
                                ),
                                entry.hostname().sources(),
                                value.provenance(),
                            )]
                        }
                        compose_lens::project::ProjectBuildExtraHostAddresses::List(values) => values
                            .iter()
                            .map(|value| {
                                self.sourced_project_key_value(
                                    ImageArtifactAssignment::new(
                                        Self::protected(entry.hostname().value(), entry.hostname().is_sensitive()),
                                        Some(Self::protected(value.value(), value.is_sensitive())),
                                    ),
                                    entry.hostname().sources(),
                                    value.provenance(),
                                )
                            })
                            .collect(),
                        _ => Vec::new(),
                    })
                    .collect(),
            ),
            _ => return,
        };
        settings.push(self.sourced_provenance(
            SourceBuildSetting::ExtraHosts(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_no_cache_filter(
        &self,
        native: Option<&ProjectValue<ProjectBuildNoCacheFilter>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values) = match native.value() {
            ProjectBuildNoCacheFilter::Scalar(value) => (
                BuildSyntax::Scalar,
                vec![self.sourced_provenance(Self::protected(value.value(), value.is_sensitive()), value.provenance())],
            ),
            ProjectBuildNoCacheFilter::List(values) => (
                BuildSyntax::Sequence,
                values
                    .iter()
                    .map(|value| {
                        self.sourced_provenance(
                            Self::protected(value.value(), value.is_sensitive()),
                            value.provenance(),
                        )
                    })
                    .collect(),
            ),
            _ => return,
        };
        settings.push(self.sourced_provenance(
            SourceBuildSetting::NoCacheFilters(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_ssh(
        &self,
        native: Option<&ProjectValue<ProjectBuildSsh>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let (syntax, values) = match native.value() {
            ProjectBuildSsh::List(entries) => (
                BuildSyntax::Sequence,
                entries
                    .iter()
                    .map(|entry| self.sourced_provenance(ProtectedString::sensitive(entry.value()), entry.provenance()))
                    .collect(),
            ),
            ProjectBuildSsh::Map(entries) => (
                BuildSyntax::Mapping,
                entries
                    .iter()
                    .map(|entry| {
                        self.sourced_project_key_value(
                            ProtectedString::sensitive(format!(
                                "{}={}",
                                entry.name().value(),
                                scalar_text(entry.value().value())
                            )),
                            entry.name().sources(),
                            entry.value().provenance(),
                        )
                    })
                    .collect(),
            ),
            _ => return,
        };
        settings.push(self.sourced_provenance(
            SourceBuildSetting::Ssh(BuildSettingValues::new(syntax, values)),
            native.provenance(),
        ));
    }

    fn map_build_secrets(
        &self,
        native: Option<&ProjectValue<Vec<ProjectValue<ProjectGrant>>>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let values = native
            .value()
            .iter()
            .filter_map(|grant| match grant.value() {
                ProjectGrant::Short(source) => Some(self.sourced_provenance(
                    SourceBuildSecret::new(Self::protected(source, grant.is_sensitive())),
                    grant.provenance(),
                )),
                ProjectGrant::Long(long) => {
                    let source = long.source()?;
                    let mut secret = SourceBuildSecret::new(Self::protected(source.value(), source.is_sensitive()));
                    if let Some(target) = long.target() {
                        secret.set_target(Self::protected(target.value(), target.is_sensitive()));
                    }
                    if let Some(uid) = long.uid() {
                        secret.set_uid(Self::protected(uid.value(), uid.is_sensitive()));
                    }
                    if let Some(gid) = long.gid() {
                        secret.set_gid(Self::protected(gid.value(), gid.is_sensitive()));
                    }
                    if let Some(mode) = long.mode() {
                        secret.set_mode(Self::protected(mode.value(), mode.is_sensitive()));
                    }
                    Some(self.sourced_provenance(secret, grant.provenance()))
                }
            })
            .collect();
        settings.push(self.sourced_provenance(
            SourceBuildSetting::Secrets(BuildSettingValues::new(BuildSyntax::Sequence, values)),
            native.provenance(),
        ));
    }

    fn map_build_ulimits(
        &self,
        native: Option<&ProjectValue<compose_lens::project::ProjectUlimits>>,
        settings: &mut Vec<Sourced<SourceBuildSetting>>,
    ) {
        let Some(native) = native else { return };
        let values = native
            .value()
            .entries()
            .iter()
            .map(|entry| {
                let value = entry.value();
                let (soft, hard) = match value.value() {
                    ProjectUlimitValue::Single(value) => {
                        let value = self.sourced_provenance(
                            Self::protected(value.value().value().raw(), value.is_sensitive()),
                            value.provenance(),
                        );
                        (Some(value.clone()), Some(value))
                    }
                    ProjectUlimitValue::Range(range) => (
                        range.soft().map(|value| {
                            self.sourced_provenance(
                                Self::protected(value.value().value().raw(), value.is_sensitive()),
                                value.provenance(),
                            )
                        }),
                        range.hard().map(|value| {
                            self.sourced_provenance(
                                Self::protected(value.value().value().raw(), value.is_sensitive()),
                                value.provenance(),
                            )
                        }),
                    ),
                    _ => (None, None),
                };
                self.sourced_project_key_value(
                    ResourceLimit::new(
                        Self::protected(entry.value().name().value(), entry.value().name().is_sensitive()),
                        soft,
                        hard,
                    ),
                    entry.value().name().sources(),
                    entry.provenance(),
                )
            })
            .collect();
        settings.push(self.sourced_provenance(
            SourceBuildSetting::Ulimits(BuildSettingValues::new(BuildSyntax::Mapping, values)),
            native.provenance(),
        ));
    }

    fn map_service_labels(&mut self, subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(labels) = native.labels() {
            self.map_labels(subject, labels.value(), service);
        }
    }

    fn map_restart_policy(&mut self, subject: &str, native: &ProjectService, service: &mut Service) {
        let Some(restart) = native.restart() else {
            return;
        };
        let restart_subject = format!("{subject}.restart_policy");
        let policy = match restart.value().kind() {
            ComposeRestartPolicyKind::No => NeutralRestartPolicy::Never,
            ComposeRestartPolicyKind::Always => NeutralRestartPolicy::Always,
            ComposeRestartPolicyKind::OnFailure { maximum_retries: None } => NeutralRestartPolicy::on_failure(None),
            ComposeRestartPolicyKind::OnFailure {
                maximum_retries: Some(maximum_retries),
            } => {
                let Ok(maximum_retries) = maximum_retries.parse::<u64>() else {
                    self.invalid_value_optional(
                        &restart_subject,
                        "restart maximum retry count exceeds the neutral model's unsigned 64-bit range",
                        restart.effective_source(),
                    );
                    return;
                };
                let Some(maximum_retries) = std::num::NonZeroU64::new(maximum_retries) else {
                    self.invalid_value_optional(
                        &restart_subject,
                        "an explicitly authored restart maximum retry count must be greater than zero",
                        restart.effective_source(),
                    );
                    return;
                };
                NeutralRestartPolicy::on_failure(Some(maximum_retries))
            }
            ComposeRestartPolicyKind::UnlessStopped => NeutralRestartPolicy::UnlessStopped,
            ComposeRestartPolicyKind::Expression => {
                self.invalid_value_optional(
                    &restart_subject,
                    "restart policy expression was not resolved before conversion",
                    restart.effective_source(),
                );
                return;
            }
            ComposeRestartPolicyKind::Other => {
                self.invalid_value_optional(
                    &restart_subject,
                    "restart policy is not a Compose-defined service-level policy",
                    restart.effective_source(),
                );
                return;
            }
            _ => {
                self.unsupported_optional(
                    &restart_subject,
                    "restart policy variant is newer than this Compose adapter",
                    restart.effective_source(),
                );
                return;
            }
        };
        service.set_restart_policy(self.sourced_provenance(policy, restart.provenance()));
        self.exact_provenance(restart_subject, restart.provenance());
    }

    fn map_execution_context(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(user) = native.user() {
            let sensitive = user.is_sensitive();
            let primary = Self::protected(user.value().user().raw(), sensitive);
            service.set_user(self.sourced_provenance(primary, user.provenance()));
            self.exact_provenance(format!("{service_subject}.user"), user.provenance());

            if let Some(group) = user.value().group() {
                service.set_group(self.sourced_provenance(Self::protected(group.raw(), sensitive), user.provenance()));
                self.exact_provenance(format!("{service_subject}.group"), user.provenance());
            }
        }
        if let Some(user_namespace) = native.userns_mode() {
            service.set_user_namespace(self.sourced_provenance(
                Self::protected(user_namespace.value().raw().value(), user_namespace.is_sensitive()),
                user_namespace.provenance(),
            ));
            self.exact_provenance(format!("{service_subject}.user_namespace"), user_namespace.provenance());
        }
        if let Some(groups) = native.group_add() {
            for (index, group) in groups.value().iter().enumerate() {
                service.add_supplementary_group(
                    self.sourced_provenance(Self::protected(group.value(), group.is_sensitive()), group.provenance()),
                );
                self.exact_provenance(
                    format!("{service_subject}.supplementary_groups[{index}]"),
                    group.provenance(),
                );
            }
        }
        if let Some(working_directory) = native.working_dir() {
            service.set_working_directory(self.sourced_provenance(
                Self::protected(working_directory.value(), working_directory.is_sensitive()),
                working_directory.provenance(),
            ));
            self.exact_provenance(
                format!("{service_subject}.working_directory"),
                working_directory.provenance(),
            );
        }
        if let Some(read_only) = native.read_only() {
            let subject = format!("{service_subject}.read_only_root_filesystem");
            match read_only.value() {
                BooleanValue::Literal(value) => {
                    service.set_read_only_root_filesystem(self.sourced_provenance(*value, read_only.provenance()));
                    self.exact_provenance(subject, read_only.provenance());
                }
                BooleanValue::Expression(_) => self.invalid_value_optional(
                    &subject,
                    "read-only root-filesystem expression was not resolved",
                    read_only.effective_source(),
                ),
            }
        }
    }

    fn map_released_container_settings(
        &mut self,
        service_subject: &str,
        native: &ProjectService,
        service: &mut Service,
    ) {
        self.map_hostname_pids_and_shm(service_subject, native, service);
        self.map_capabilities_and_tmpfs(service_subject, native, service);
        self.map_sysctls(service_subject, native, service);
        self.map_ulimits(service_subject, native, service);
        self.map_devices_and_stop_signal(service_subject, native, service);
    }

    fn map_dns(&mut self, subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(dns) = native.dns() {
            let values = match dns.value() {
                ProjectDns::Scalar(value) => Some(std::slice::from_ref(value)),
                ProjectDns::List(values) => Some(values.as_slice()),
                _ => {
                    self.unsupported_optional(
                        &format!("{subject}.dns"),
                        "DNS form is newer than this adapter",
                        dns.effective_source(),
                    );
                    None
                }
            };
            if let Some(values) = values {
                let mapped = values
                    .iter()
                    .map(|value| {
                        self.sourced_provenance(
                            Self::protected(value.value(), value.is_sensitive()),
                            value.provenance(),
                        )
                    })
                    .collect();
                service.set_dns_servers_with_origins(mapped, self.origins(dns.provenance()));
                self.dns_import_outcome(&format!("{subject}.dns"), values, dns.provenance(), "dns");
            }
        }
        if let Some(options) = native.dns_options() {
            let mapped = options
                .value()
                .iter()
                .map(|value| {
                    self.sourced_provenance(Self::protected(value.value(), value.is_sensitive()), value.provenance())
                })
                .collect();
            service.set_dns_options_with_origins(mapped, self.origins(options.provenance()));
            let duplicate = options.value().iter().enumerate().any(|(index, value)| {
                options.value()[..index]
                    .iter()
                    .any(|prior| prior.value() == value.value())
            });
            if duplicate {
                self.invalid_value_optional(
                    &format!("{subject}.dns_opt"),
                    "dns_opt contains duplicate resolver options",
                    options.effective_source(),
                );
            } else {
                self.dns_import_outcome(
                    &format!("{subject}.dns_opt"),
                    options.value(),
                    options.provenance(),
                    "dns_opt",
                );
            }
        }
        if let Some(search) = native.dns_search() {
            let values = match search.value() {
                ProjectDnsSearch::Scalar(value) => Some(std::slice::from_ref(value)),
                ProjectDnsSearch::List(values) => Some(values.as_slice()),
                _ => {
                    self.unsupported_optional(
                        &format!("{subject}.dns_search"),
                        "DNS search form is newer than this adapter",
                        search.effective_source(),
                    );
                    None
                }
            };
            if let Some(values) = values {
                let mapped = values
                    .iter()
                    .map(|value| {
                        self.sourced_provenance(
                            Self::protected(value.value(), value.is_sensitive()),
                            value.provenance(),
                        )
                    })
                    .collect();
                service.set_dns_search_domains_with_origins(mapped, self.origins(search.provenance()));
                self.dns_import_outcome(
                    &format!("{subject}.dns_search"),
                    values,
                    search.provenance(),
                    "dns_search",
                );
            }
        }
    }

    fn map_security_options(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        let Some(options) = native.security_options() else {
            return;
        };
        let subject = format!("{service_subject}.security_opt");
        let mut mapped = Vec::new();
        let mut singleton_counts = [0_usize; 8];
        let mut has_label_disable = false;
        let mut has_other_label = false;

        for (index, item) in options.value().iter().enumerate() {
            let item_subject = format!("{subject}[{index}]");
            match Self::map_security_option(item.value().kind(), item.is_sensitive()) {
                SecurityOptionMapping::Exact(value) => {
                    Self::record_security_option_conflict(
                        &value,
                        &mut singleton_counts,
                        &mut has_label_disable,
                        &mut has_other_label,
                    );
                    mapped.push(self.sourced_provenance(value, item.provenance()));
                    self.exact_provenance(item_subject, item.provenance());
                }
                SecurityOptionMapping::Invalid(reason) => {
                    self.invalid_value_optional(&item_subject, reason, item.effective_source());
                }
                SecurityOptionMapping::Unsupported(reason) => {
                    self.unsupported_optional(&item_subject, reason, item.effective_source());
                }
            }
        }

        service.set_security_options_with_origins(mapped, self.origins(options.provenance()));
        let singleton_conflict = singleton_counts.iter().any(|count| *count > 1);
        if singleton_conflict {
            self.invalid_value_optional(
                &subject,
                "security_opt contains multiple candidates for a singleton security-option family",
                options.effective_source(),
            );
        }
        if has_label_disable && has_other_label {
            self.unsupported_optional(
                &subject,
                "label:disable conflicts semantically with other SELinux label candidates",
                options.effective_source(),
            );
        }
        if !(singleton_conflict || has_label_disable && has_other_label) {
            self.exact_provenance(subject, options.provenance());
        }
    }

    fn map_security_option(kind: &SecurityOptionKind, sensitive: bool) -> SecurityOptionMapping {
        let exact = match kind {
            SecurityOptionKind::AppArmor { profile } => SecurityOption::AppArmor(Self::protected(profile, sensitive)),
            SecurityOptionKind::NoNewPrivileges { enabled } => SecurityOption::NoNewPrivileges(*enabled),
            SecurityOptionKind::Seccomp { profile } => {
                SecurityOption::SeccompProfile(Self::protected(profile, sensitive))
            }
            SecurityOptionKind::SecurityLabelDisable { enabled } => SecurityOption::SecurityLabelDisable(*enabled),
            SecurityOptionKind::SecurityLabelFileType { file_type } => {
                SecurityOption::SecurityLabelFileType(Self::protected(file_type, sensitive))
            }
            SecurityOptionKind::SecurityLabelLevel { level } => {
                SecurityOption::SecurityLabelLevel(Self::protected(level, sensitive))
            }
            SecurityOptionKind::SecurityLabelNested { enabled } => SecurityOption::SecurityLabelNested(*enabled),
            SecurityOptionKind::SecurityLabelType { label_type } => {
                SecurityOption::SecurityLabelType(Self::protected(label_type, sensitive))
            }
            SecurityOptionKind::Mask { paths } => SecurityOption::Mask(Self::protected(paths, sensitive)),
            SecurityOptionKind::Unmask { paths } => SecurityOption::Unmask(Self::protected(paths, sensitive)),
            SecurityOptionKind::Expression => {
                return SecurityOptionMapping::Invalid("security option expression was not resolved before conversion");
            }
            SecurityOptionKind::Empty => {
                return SecurityOptionMapping::Invalid("security options must not contain empty strings");
            }
            SecurityOptionKind::AppArmorNearMiss
            | SecurityOptionKind::SeccompNearMiss
            | SecurityOptionKind::NoNewPrivilegesNearMiss
            | SecurityOptionKind::SecurityLabelDisableNearMiss
            | SecurityOptionKind::SecurityLabelFileTypeNearMiss
            | SecurityOptionKind::SecurityLabelLevelNearMiss
            | SecurityOptionKind::SecurityLabelNestedNearMiss
            | SecurityOptionKind::SecurityLabelTypeNearMiss
            | SecurityOptionKind::MaskNearMiss
            | SecurityOptionKind::UnmaskNearMiss => {
                return SecurityOptionMapping::Invalid(
                    "security option must use a released exact ComposeLens candidate spelling",
                );
            }
            SecurityOptionKind::Other => {
                return SecurityOptionMapping::Unsupported("raw security option has no neutral semantic mapping");
            }
            _ => {
                return SecurityOptionMapping::Unsupported(
                    "security option variant is newer than this Compose adapter",
                );
            }
        };
        SecurityOptionMapping::Exact(exact)
    }

    fn record_security_option_conflict(
        value: &SecurityOption,
        singleton_counts: &mut [usize; 8],
        has_label_disable: &mut bool,
        has_other_label: &mut bool,
    ) {
        let family = match value {
            SecurityOption::AppArmor(_) => Some(0),
            SecurityOption::NoNewPrivileges(_) => Some(1),
            SecurityOption::SeccompProfile(_) => Some(2),
            SecurityOption::SecurityLabelDisable(enabled) => {
                *has_label_disable |= *enabled;
                Some(3)
            }
            SecurityOption::SecurityLabelFileType(_) => Some(4),
            SecurityOption::SecurityLabelLevel(_) => Some(5),
            SecurityOption::SecurityLabelNested(_) => Some(6),
            SecurityOption::SecurityLabelType(_) => Some(7),
            _ => None,
        };
        *has_other_label |= matches!(
            value,
            SecurityOption::SecurityLabelFileType(_)
                | SecurityOption::SecurityLabelLevel(_)
                | SecurityOption::SecurityLabelNested(_)
                | SecurityOption::SecurityLabelType(_)
        );
        if let Some(family) = family {
            singleton_counts[family] += 1;
        }
    }

    fn dns_import_outcome(
        &mut self,
        subject: &str,
        values: &[ProjectValue<String>],
        provenance: &MergeProvenance,
        field: &str,
    ) {
        if values.is_empty() {
            self.unsupported_optional(
                subject,
                "explicit empty DNS collections have target-specific reset semantics",
                provenance.effective_source(),
            );
        } else if values
            .iter()
            .any(|value| value.value().is_empty() || value.value().contains(['\r', '\n']) || value.is_sensitive())
        {
            self.invalid_value_optional(
                subject,
                "DNS values must be resolved non-empty single physical lines",
                provenance.effective_source(),
            );
        } else if (field == "dns" && values.iter().any(|value| value.value() == "none"))
            || (field == "dns_search" && values.iter().any(|value| value.value() == "."))
        {
            self.unsupported_optional(
                subject,
                "special DNS values have target-specific resolver semantics",
                provenance.effective_source(),
            );
        } else {
            self.exact_provenance(subject, provenance);
        }
    }

    fn map_hostname_pids_and_shm(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(hostname) = native.hostname() {
            let subject = format!("{service_subject}.hostname");
            let value = hostname.value().raw().value();
            service.set_hostname(
                self.sourced_provenance(Self::protected(value, hostname.is_sensitive()), hostname.provenance()),
            );
            if native
                .unmodeled_fields()
                .iter()
                .any(|field| field.path().last().is_some_and(|name| name == "uts"))
            {
                self.unsupported_optional(
                    &subject,
                    "hostname cannot be emitted while host UTS mode remains unmodeled",
                    hostname.effective_source(),
                );
            } else if matches!(hostname.value().kind(), HostnameKind::Resolved) {
                self.exact_provenance(subject, hostname.provenance());
            } else {
                self.invalid_value_optional(
                    &subject,
                    "hostname is unresolved or outside the conservative portable hostname grammar",
                    hostname.effective_source(),
                );
            }
        }
        if let Some(limit) = native.pids_limit() {
            let subject = format!("{service_subject}.pids_limit");
            service.set_pids_limit(self.sourced_provenance(
                Self::protected(limit.value().raw().value(), limit.is_sensitive()),
                limit.provenance(),
            ));
            match limit.value().kind() {
                PidsLimitKind::Unlimited | PidsLimitKind::Finite { .. } => {
                    self.exact_provenance(subject, limit.provenance());
                }
                _ => self.invalid_value_optional(
                    &subject,
                    "PID limit must be -1 or a positive ASCII decimal before exact conversion",
                    limit.effective_source(),
                ),
            }
        }
        if let Some(size) = native.shm_size() {
            let subject = format!("{service_subject}.shm_size");
            service.set_shm_size(self.sourced_provenance(
                Self::protected(size.value().raw().value(), size.is_sensitive()),
                size.provenance(),
            ));
            let exact = matches!(size.value().kind(), ShmSizeKind::Documented { amount_raw, unit: ShmSizeUnit::B | ShmSizeUnit::K | ShmSizeUnit::M | ShmSizeUnit::G }
                if amount_raw.bytes().any(|byte| byte != b'0') && amount_raw.bytes().all(|byte| byte.is_ascii_digit()));
            if exact {
                self.exact_provenance(subject, size.provenance());
            } else {
                self.invalid_value_optional(
                    &subject,
                    "shared-memory size must be a positive ASCII decimal with b, k, m, or g",
                    size.effective_source(),
                );
            }
        }
    }

    fn map_capabilities_and_tmpfs(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(values) = native.cap_add() {
            let subject = format!("{service_subject}.cap_add");
            let mut exact = true;
            let mapped = values
                .value()
                .iter()
                .map(|value| {
                    if !value.value().is_exact_candidate() || value.value().value().contains('$') {
                        exact = false;
                    }
                    self.sourced_provenance(
                        Self::protected(value.value().value(), value.is_sensitive()),
                        value.provenance(),
                    )
                })
                .collect();
            service.set_cap_add_with_origins(mapped, self.origins(values.provenance()));
            if exact {
                self.exact_provenance(subject, values.provenance());
            } else {
                self.invalid_value_optional(
                    &subject,
                    "capability entries must be resolved non-empty strings without whitespace",
                    values.effective_source(),
                );
            }
        }
        if let Some(values) = native.cap_drop() {
            let subject = format!("{service_subject}.cap_drop");
            let mut exact = true;
            let mapped = values
                .value()
                .iter()
                .map(|value| {
                    if !value.value().is_exact_candidate() || value.value().value().contains('$') {
                        exact = false;
                    }
                    self.sourced_provenance(
                        Self::protected(value.value().value(), value.is_sensitive()),
                        value.provenance(),
                    )
                })
                .collect();
            service.set_cap_drop_with_origins(mapped, self.origins(values.provenance()));
            if exact {
                self.exact_provenance(subject, values.provenance());
            } else {
                self.invalid_value_optional(
                    &subject,
                    "capability entries must be resolved non-empty strings without whitespace",
                    values.effective_source(),
                );
            }
        }
        if let Some(tmpfs) = native.tmpfs() {
            let (values, exact) = match tmpfs.value() {
                ProjectTmpfs::Scalar(value) => (vec![value], value.value().kind() == TmpfsItemKind::Documented),
                ProjectTmpfs::List(values) => (
                    values.iter().collect(),
                    values
                        .iter()
                        .all(|value| value.value().kind() == TmpfsItemKind::Documented),
                ),
                _ => (Vec::new(), false),
            };
            let mapped = values
                .into_iter()
                .map(|value| {
                    self.sourced_provenance(
                        Self::protected(value.value().value(), value.is_sensitive()),
                        value.provenance(),
                    )
                })
                .collect();
            service.set_tmpfs_with_origins(mapped, self.origins(tmpfs.provenance()));
            if exact {
                self.exact_provenance(format!("{service_subject}.tmpfs"), tmpfs.provenance());
            } else {
                self.invalid_value_optional(
                    &format!("{service_subject}.tmpfs"),
                    "tmpfs entries must be resolved documented declarations",
                    tmpfs.effective_source(),
                );
            }
        }
    }

    fn map_sysctls(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(sysctls) = native.sysctls() {
            let mut exact = true;
            let values = match sysctls.value() {
                ProjectSysctls::Map(values) => values
                    .iter()
                    .map(|value| {
                        let native = value.value();
                        let scalar = Self::compose_scalar(native.value().value());
                        if native.name().value().is_empty()
                            || native.name().value().contains('$')
                            || scalar.contains('$')
                            || matches!(native.value().value(), ComposeScalar::Null)
                        {
                            exact = false;
                        }
                        self.sourced_project_key_value(
                            KernelParameter::new(
                                Self::protected(native.name().value(), native.name().is_sensitive()),
                                Self::protected(&scalar, native.value().is_sensitive()),
                            ),
                            native.name().sources(),
                            native.value().provenance(),
                        )
                    })
                    .collect(),
                ProjectSysctls::List(values) => values
                    .iter()
                    .filter_map(|value| {
                        let Some((name, scalar)) = value.value().split_once('=') else {
                            exact = false;
                            self.invalid_value_optional(
                                &format!("{service_subject}.sysctls"),
                                "sysctl list entries must use unambiguous name=value spelling",
                                value.effective_source(),
                            );
                            return None;
                        };
                        if name.is_empty() || name.contains('$') || scalar.contains('$') {
                            exact = false;
                        }
                        Some(self.sourced_provenance(
                            KernelParameter::new(
                                Self::protected(name, value.is_sensitive()),
                                Self::protected(scalar, value.is_sensitive()),
                            ),
                            value.provenance(),
                        ))
                    })
                    .collect(),
                _ => {
                    exact = false;
                    Vec::new()
                }
            };
            service.set_sysctls_with_origins(values, self.origins(sysctls.provenance()));
            if exact {
                self.exact_provenance(format!("{service_subject}.sysctls"), sysctls.provenance());
            } else {
                self.invalid_value_optional(
                    &format!("{service_subject}.sysctls"),
                    "sysctl entries must use resolved unambiguous name=value spelling",
                    sysctls.effective_source(),
                );
            }
        }
    }

    fn map_ulimits(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(ulimits) = native.ulimits() {
            let mut mapped = Vec::new();
            let mut exact = true;
            for limit in ulimits.value().entries() {
                let name = limit.value().name();
                let (soft, hard) = match limit.value().value() {
                    ProjectUlimitValue::Single(value) => {
                        if !matches!(value.value().value(), LimitValue::Unlimited | LimitValue::Number(_)) {
                            exact = false;
                        }
                        let value = self.sourced_provenance(
                            Self::protected(value.value().authored(), value.is_sensitive()),
                            value.provenance(),
                        );
                        (Some(value.clone()), Some(value))
                    }
                    ProjectUlimitValue::Range(range) => (
                        range.soft().map(|value| {
                            if !matches!(value.value().value(), LimitValue::Unlimited | LimitValue::Number(_)) {
                                exact = false;
                            }
                            self.sourced_provenance(
                                Self::protected(value.value().authored(), value.is_sensitive()),
                                value.provenance(),
                            )
                        }),
                        range.hard().map(|value| {
                            if !matches!(value.value().value(), LimitValue::Unlimited | LimitValue::Number(_)) {
                                exact = false;
                            }
                            self.sourced_provenance(
                                Self::protected(value.value().authored(), value.is_sensitive()),
                                value.provenance(),
                            )
                        }),
                    ),
                    _ => {
                        exact = false;
                        (None, None)
                    }
                };
                if soft.is_none() || hard.is_none() {
                    exact = false;
                }
                let item = ResourceLimit::new(Self::protected(name.value(), name.is_sensitive()), soft, hard);
                mapped.push(self.sourced_spans(item, name.sources()));
            }
            service.set_ulimits_with_origins(mapped, self.origins(ulimits.provenance()));
            if exact {
                self.exact_provenance(format!("{service_subject}.ulimits"), ulimits.provenance());
            } else {
                self.invalid_value_optional(
                    &format!("{service_subject}.ulimits"),
                    "ulimits require complete resolved -1 or non-negative decimal values",
                    ulimits.effective_source(),
                );
            }
        }
    }

    fn map_devices_and_stop_signal(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(devices) = native.devices() {
            let mut exact = true;
            let mapped = devices
                .value()
                .iter()
                .filter_map(|device| match device.value() {
                    ProjectDevice::Short(short) => {
                        if matches!(short.kind(), ShortDeviceKind::Deferred) {
                            exact = false;
                        }
                        Some(self.sourced_provenance(
                            Device::Short(Self::protected(short.raw().value(), device.is_sensitive())),
                            device.provenance(),
                        ))
                    }
                    ProjectDevice::Long(long) => {
                        if long.source().is_none_or(|value| value.value().contains('$'))
                            || long.target().is_some_and(|value| value.value().contains('$'))
                            || long.permissions().is_some_and(|value| value.value().contains('$'))
                            || !long.extension_fields().is_empty()
                            || !long.unknown_fields().is_empty()
                        {
                            exact = false;
                        }
                        Some(self.sourced_provenance(
                            Device::Long {
                                source: long.source().map(|value| {
                                    self.sourced_provenance(
                                        Self::protected(value.value(), value.is_sensitive()),
                                        value.provenance(),
                                    )
                                }),
                                target: long.target().map(|value| {
                                    self.sourced_provenance(
                                        Self::protected(value.value(), value.is_sensitive()),
                                        value.provenance(),
                                    )
                                }),
                                permissions: long.permissions().map(|value| {
                                    self.sourced_provenance(
                                        Self::protected(value.value(), value.is_sensitive()),
                                        value.provenance(),
                                    )
                                }),
                            },
                            device.provenance(),
                        ))
                    }
                    _ => {
                        exact = false;
                        None
                    }
                })
                .collect();
            service.set_devices_with_origins(mapped, self.origins(devices.provenance()));
            if exact {
                self.exact_provenance(format!("{service_subject}.devices"), devices.provenance());
            } else {
                self.invalid_value_optional(
                    &format!("{service_subject}.devices"),
                    "devices require complete resolved short or long declarations",
                    devices.effective_source(),
                );
            }
        }
        if let Some(signal) = native.stop_signal() {
            service.set_stop_signal(self.sourced_provenance(
                Self::protected(signal.value(), signal.is_sensitive()),
                signal.provenance(),
            ));
            if Self::is_safe_stop_signal(signal.value()) {
                self.exact_provenance(format!("{service_subject}.stop_signal"), signal.provenance());
            } else {
                self.invalid_value_optional(
                    &format!("{service_subject}.stop_signal"),
                    "stop signal must be a non-empty resolved token or number",
                    signal.effective_source(),
                );
            }
        }
    }

    fn compose_scalar(value: &ComposeScalar) -> String {
        match value {
            ComposeScalar::Null => String::new(),
            ComposeScalar::Boolean(value) => value.to_string(),
            ComposeScalar::Number(value) | ComposeScalar::String(value) => value.clone(),
        }
    }

    fn is_safe_stop_signal(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }

    fn protected(value: &str, sensitive: bool) -> ProtectedString {
        if sensitive {
            ProtectedString::sensitive(value)
        } else {
            ProtectedString::plain(value)
        }
    }

    fn map_service_dependencies(
        &mut self,
        service_subject: &str,
        dependencies: &ProjectValue<ProjectDependsOn>,
        service: &mut Service,
    ) {
        for (index, native) in dependencies.value().services().iter().enumerate() {
            let dependency_subject = format!("{service_subject}.depends_on[{index}]");
            let key = native.value().service();
            if key.is_sensitive() {
                self.invalid_value_optional(
                    &dependency_subject,
                    "dependency service name contains sensitive interpolated content",
                    key.effective_source(),
                );
                continue;
            }
            let Some(name) = self.identifier_optional(&dependency_subject, key.value(), key.effective_source()) else {
                continue;
            };
            let mut dependency = ServiceDependency::new(name);

            if let Some(condition) = native.value().condition() {
                if condition.is_sensitive() {
                    self.invalid_value_optional(
                        &format!("{dependency_subject}.condition"),
                        "dependency condition contains sensitive interpolated content",
                        condition.effective_source(),
                    );
                } else {
                    let value = match condition.value() {
                        ComposeDependencyCondition::ServiceStarted => ServiceDependencyCondition::Started,
                        ComposeDependencyCondition::ServiceHealthy => ServiceDependencyCondition::Healthy,
                        ComposeDependencyCondition::ServiceCompletedSuccessfully => {
                            ServiceDependencyCondition::CompletedSuccessfully
                        }
                        ComposeDependencyCondition::Other(value) => {
                            ServiceDependencyCondition::Other(ProtectedString::plain(value.clone()))
                        }
                    };
                    dependency.set_condition(self.sourced_provenance(value, condition.provenance()));
                    self.exact_provenance(format!("{dependency_subject}.condition"), condition.provenance());
                }
            }

            if let Some(restart) = native.value().restart() {
                self.map_dependency_boolean(&format!("{dependency_subject}.restart"), restart, |value| {
                    dependency.set_restart(value);
                });
            }
            if let Some(required) = native.value().required() {
                self.map_dependency_boolean(&format!("{dependency_subject}.required"), required, |value| {
                    dependency.set_required(value);
                });
            }
            self.report_project_fields(
                &dependency_subject,
                "dependency option",
                native.value().unmodeled_fields(),
            );

            let dependency = self.sourced_spans(dependency, key.sources());
            self.exact_origins(dependency_subject, dependency.origins());
            service.add_dependency(dependency);
        }
    }

    fn map_dependency_boolean(
        &mut self,
        subject: &str,
        native: &ProjectValue<BooleanValue>,
        set: impl FnOnce(Sourced<bool>),
    ) {
        match native.value() {
            BooleanValue::Literal(value) => {
                set(self.sourced_provenance(*value, native.provenance()));
                self.exact_provenance(subject, native.provenance());
            }
            BooleanValue::Expression(_) => self.invalid_value_optional(
                subject,
                "dependency boolean expression was not resolved",
                native.effective_source(),
            ),
        }
    }

    fn map_command(command: &compose_lens::model::Command, sensitive: bool) -> Option<Command> {
        let protect = |value: String| {
            if sensitive {
                ProtectedString::sensitive(value)
            } else {
                ProtectedString::plain(value)
            }
        };
        match command {
            compose_lens::model::Command::Null(_) => None,
            compose_lens::model::Command::String(value) if value.value().is_empty() => Some(Command::Empty),
            compose_lens::model::Command::String(value) => Some(Command::Shell(protect(value.value().clone()))),
            compose_lens::model::Command::List { values, .. } if values.is_empty() => Some(Command::Empty),
            compose_lens::model::Command::List { values, .. } => Some(Command::Exec(
                values.iter().map(|value| protect(value.value().clone())).collect(),
            )),
        }
    }

    fn map_healthcheck(
        &mut self,
        service_subject: &str,
        native: &ProjectValue<ProjectHealthcheck>,
    ) -> Sourced<Healthcheck> {
        let mut healthcheck = Healthcheck::new();

        if let Some(disable) = native.value().disable() {
            let subject = format!("{service_subject}.healthcheck.disable");
            match disable.value() {
                BooleanValue::Literal(value) => {
                    healthcheck.set_disabled(self.sourced_provenance(*value, disable.provenance()));
                    self.exact_provenance(subject, disable.provenance());
                }
                BooleanValue::Expression(_) => self.invalid_value_optional(
                    &subject,
                    "health-check disable expression was not resolved",
                    disable.effective_source(),
                ),
            }
        }

        if let Some(test) = native.value().test() {
            self.map_healthcheck_test(service_subject, test, &mut healthcheck);
        }
        if let Some(interval) = native.value().interval() {
            let subject = format!("{service_subject}.healthcheck.interval");
            if let Some(value) = self.map_healthcheck_duration(&subject, interval) {
                healthcheck.set_interval(value);
            }
        }
        if let Some(timeout) = native.value().timeout() {
            let subject = format!("{service_subject}.healthcheck.timeout");
            if let Some(value) = self.map_healthcheck_duration(&subject, timeout) {
                healthcheck.set_timeout(value);
            }
        }
        if let Some(retries) = native.value().retries() {
            let subject = format!("{service_subject}.healthcheck.retries");
            match retries.value() {
                ComposeHealthcheckRetries::Count(value) => match NeutralHealthcheckRetries::new(value.clone()) {
                    Ok(value) => {
                        healthcheck.set_retries(self.sourced_provenance(value, retries.provenance()));
                        self.exact_provenance(subject, retries.provenance());
                    }
                    Err(error) => self.invalid_model_optional(&subject, &error, retries.effective_source()),
                },
                ComposeHealthcheckRetries::Expression(_) => self.invalid_value_optional(
                    &subject,
                    "health-check retry expression was not resolved",
                    retries.effective_source(),
                ),
                ComposeHealthcheckRetries::Other(value) => self.invalid_value_optional(
                    &subject,
                    &format!("invalid health-check retry count `{value}`"),
                    retries.effective_source(),
                ),
            }
        }
        if let Some(start_period) = native.value().start_period() {
            let subject = format!("{service_subject}.healthcheck.start_period");
            if let Some(value) = self.map_healthcheck_duration(&subject, start_period) {
                healthcheck.set_start_period(value);
            }
        }
        if let Some(start_interval) = native.value().start_interval() {
            let subject = format!("{service_subject}.healthcheck.start_interval");
            if let Some(value) = self.map_healthcheck_duration(&subject, start_interval) {
                healthcheck.set_start_interval(value);
            }
        }
        self.report_project_fields(
            &format!("{service_subject}.healthcheck"),
            "health-check field",
            native.value().unmodeled_fields(),
        );

        self.sourced_provenance(healthcheck, native.provenance())
    }

    fn map_healthcheck_test(
        &mut self,
        service_subject: &str,
        test: &ProjectValue<HealthcheckTest>,
        healthcheck: &mut Healthcheck,
    ) {
        let subject = format!("{service_subject}.healthcheck.test");
        let protect = |value: String| {
            if test.is_sensitive() {
                ProtectedString::sensitive(value)
            } else {
                ProtectedString::plain(value)
            }
        };
        match test.value() {
            HealthcheckTest::String(value) => {
                healthcheck.set_command(self.sourced_provenance(
                    HealthcheckCommand::Shell(protect(value.value().clone())),
                    test.provenance(),
                ));
                self.exact_provenance(subject, test.provenance());
            }
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::Cmd),
                values,
                ..
            } if values.len() > 1 => {
                healthcheck.set_command(self.sourced_provenance(
                    HealthcheckCommand::Exec(values[1..].iter().map(|value| protect(value.value().clone())).collect()),
                    test.provenance(),
                ));
                self.exact_provenance(subject, test.provenance());
            }
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::CmdShell),
                values,
                ..
            } if values.len() == 2 => {
                healthcheck.set_command(self.sourced_provenance(
                    HealthcheckCommand::Shell(protect(values[1].value().clone())),
                    test.provenance(),
                ));
                self.exact_provenance(subject, test.provenance());
            }
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::None),
                values,
                ..
            } if values.len() == 1 => {
                healthcheck.set_disabled(self.sourced_provenance(true, test.provenance()));
                self.exact_provenance(subject, test.provenance());
            }
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::Cmd),
                ..
            } => self.invalid_value_optional(
                &subject,
                "CMD health check requires at least one command argument",
                test.effective_source(),
            ),
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::CmdShell),
                ..
            } => self.unsupported_optional(
                &subject,
                "CMD-SHELL health check must contain exactly one shell string",
                test.effective_source(),
            ),
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::None),
                ..
            } => self.invalid_value_optional(
                &subject,
                "NONE health check cannot contain command arguments",
                test.effective_source(),
            ),
            HealthcheckTest::List { kind: None, .. } => {
                self.invalid_value_optional(&subject, "health-check test list is empty", test.effective_source());
            }
            HealthcheckTest::List {
                kind: Some(HealthcheckTestKind::Other),
                ..
            } => self.unsupported_optional(&subject, "unknown health-check command mode", test.effective_source()),
        }
    }

    fn map_healthcheck_duration(
        &mut self,
        subject: &str,
        duration: &ProjectValue<ComposeHealthcheckDuration>,
    ) -> Option<Sourced<NeutralHealthcheckDuration>> {
        match duration.value() {
            ComposeHealthcheckDuration::Value(value) => match NeutralHealthcheckDuration::new(value.clone()) {
                Ok(value) => {
                    let value = self.sourced_provenance(value, duration.provenance());
                    self.exact_provenance(subject, duration.provenance());
                    Some(value)
                }
                Err(error) => {
                    self.invalid_model_optional(subject, &error, duration.effective_source());
                    None
                }
            },
            ComposeHealthcheckDuration::Expression(_) => {
                self.invalid_value_optional(
                    subject,
                    "health-check duration expression was not resolved",
                    duration.effective_source(),
                );
                None
            }
            ComposeHealthcheckDuration::Other(value) => {
                self.invalid_value_optional(
                    subject,
                    &format!("invalid health-check duration `{value}`"),
                    duration.effective_source(),
                );
                None
            }
        }
    }

    fn map_environment(&mut self, service_subject: &str, environment: &ProjectEnvironment, service: &mut Service) {
        for entry in environment.entries() {
            let subject = format!("{service_subject}.environment.{}", entry.name().value());
            let Some(name) = self.identifier_optional(&subject, entry.name().value(), entry.name().effective_source())
            else {
                continue;
            };
            let value = match entry.value().value() {
                ComposeScalar::Null => EnvironmentValue::Host,
                ComposeScalar::Boolean(value) => {
                    EnvironmentValue::Literal(ProtectedString::sensitive(value.to_string()))
                }
                ComposeScalar::Number(value) | ComposeScalar::String(value) => {
                    EnvironmentValue::Literal(ProtectedString::sensitive(value.clone()))
                }
            };
            service.add_environment(
                self.sourced_provenance(EnvironmentVariable::new(name, value), entry.value().provenance()),
            );
            self.exact_provenance(subject, entry.value().provenance());
        }
    }

    fn map_service_environment(&mut self, service_subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(environment) = native.environment() {
            self.map_environment(service_subject, environment.value(), service);
        }
        if let Some(environment_files) = native.environment_files() {
            self.map_environment_files(service_subject, environment_files, service);
        }
    }

    fn map_environment_files(
        &mut self,
        service_subject: &str,
        environment_files: &ProjectValue<Vec<ProjectValue<ProjectEnvironmentFile>>>,
        service: &mut Service,
    ) {
        for (index, native) in environment_files.value().iter().enumerate() {
            let subject = format!("{service_subject}.env_file[{index}]");
            let environment_file = match native.value() {
                ProjectEnvironmentFile::Short(path) => match NeutralEnvironmentFile::new(
                    Self::protected(path, native.is_sensitive()),
                    EnvironmentFileSyntax::Short,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        self.invalid_model_optional(&subject, &error, native.effective_source());
                        continue;
                    }
                },
                ProjectEnvironmentFile::Long(long) => {
                    let Some(path) = long.path() else {
                        self.invalid_value_optional(
                            &subject,
                            "long-syntax environment file has no path",
                            native.effective_source(),
                        );
                        continue;
                    };
                    let mut value = match NeutralEnvironmentFile::new(
                        Self::protected(path.value(), path.is_sensitive()),
                        EnvironmentFileSyntax::Long,
                    ) {
                        Ok(value) => value,
                        Err(error) => {
                            self.invalid_model_optional(&subject, &error, path.effective_source());
                            continue;
                        }
                    };
                    if let Some(required) = long.required() {
                        match required.value() {
                            BooleanValue::Literal(required_value) => {
                                value.set_required(self.sourced_provenance(*required_value, required.provenance()));
                                self.exact_provenance(format!("{subject}.required"), required.provenance());
                            }
                            BooleanValue::Expression(_) => {
                                self.invalid_value_optional(
                                    &format!("{subject}.required"),
                                    "environment-file required expression was not resolved",
                                    required.effective_source(),
                                );
                                continue;
                            }
                        }
                    }
                    if let Some(format) = long.format() {
                        match format.value().kind() {
                            EnvironmentFileFormatKind::Raw => {
                                value.set_format(
                                    self.sourced_provenance(NeutralEnvironmentFileFormat::Raw, format.provenance()),
                                );
                                self.exact_provenance(format!("{subject}.format"), format.provenance());
                            }
                            EnvironmentFileFormatKind::Expression => {
                                self.invalid_value_optional(
                                    &format!("{subject}.format"),
                                    "environment-file format expression was not resolved",
                                    format.effective_source(),
                                );
                                continue;
                            }
                            EnvironmentFileFormatKind::Other => {
                                self.invalid_value_optional(
                                    &format!("{subject}.format"),
                                    "environment-file format is not supported by Compose",
                                    format.effective_source(),
                                );
                                continue;
                            }
                            _ => {
                                self.invalid_value_optional(
                                    &format!("{subject}.format"),
                                    "environment-file format is newer than this BoxFerry adapter",
                                    format.effective_source(),
                                );
                                continue;
                            }
                        }
                    }
                    self.report_project_fields(&subject, "environment-file option", long.unmodeled_fields());
                    value
                }
            };
            let sourced = self.sourced_provenance(environment_file, native.provenance());
            self.exact_origins(subject, sourced.origins());
            service.add_environment_file(sourced);
        }
    }

    fn map_labels(&mut self, service_subject: &str, labels: &ProjectLabels, service: &mut Service) {
        for (index, entry) in labels.entries().iter().enumerate() {
            if entry.name().is_sensitive() {
                self.unsupported_optional(
                    &format!("{service_subject}.labels[{index}]"),
                    "interpolated sensitive label names cannot be represented safely in the neutral model",
                    entry.name().effective_source(),
                );
                continue;
            }
            let subject = format!("{service_subject}.labels.{}", entry.name().value());
            let Some(name) = self.identifier_optional(&subject, entry.name().value(), entry.name().effective_source())
            else {
                continue;
            };
            let value = match entry.value().value() {
                ComposeScalar::Null => String::new(),
                ComposeScalar::Boolean(value) => value.to_string(),
                ComposeScalar::Number(value) | ComposeScalar::String(value) => value.clone(),
            };
            let label = MetadataLabel::new(name, Self::protected(&value, entry.value().is_sensitive()));
            let label = self.sourced_metadata_label(label, entry.name().sources(), entry.value().provenance());
            self.exact_origins(subject, label.origins());
            service.add_label(label);
        }
    }

    fn map_port(&mut self, subject: &str, port: &ComposePort) -> Option<Port> {
        match port {
            ComposePort::Short(value) => self.map_short_port(subject, value),
            ComposePort::Long(value) => self.map_long_port(subject, value),
        }
    }

    fn map_short_port(&mut self, subject: &str, port: &ShortPort) -> Option<Port> {
        let target = self.port_number(subject, "target", port.target(), port.raw().span())?;
        let published = match port.published() {
            Some(value) => Some(self.port_number(subject, "published", value, port.raw().span())?),
            None => None,
        };
        let protocol = map_protocol(port.protocol());
        match Port::new(target, published, port.host_ip().map(str::to_owned), protocol) {
            Ok(value) => Some(value),
            Err(error) => {
                self.invalid_model(subject, &error, port.raw().span());
                None
            }
        }
    }

    fn map_long_port(&mut self, subject: &str, port: &LongPort) -> Option<Port> {
        let Some(target) = port.target() else {
            self.invalid_value(subject, "long-syntax port has no target", port.span());
            return None;
        };
        let target = self.port_number(subject, "target", target.value(), target.span())?;
        let published = match port.published() {
            Some(value) => Some(self.port_number(subject, "published", value.value(), value.span())?),
            None => None,
        };
        if port.app_protocol().is_some() {
            self.unsupported(subject, "ports.app_protocol", port.span());
        }
        if port.mode().is_some() {
            self.unsupported(subject, "ports.mode", port.span());
        }
        if port.name().is_some() {
            self.unsupported(subject, "ports.name", port.span());
        }
        self.report_fields(subject, "port extension", port.extension_fields());
        self.report_fields(subject, "unknown port field", port.unknown_fields());

        match Port::new(
            target,
            published,
            port.host_ip().map(|value| value.value().clone()),
            map_protocol(port.protocol().map(|value| value.value().as_str())),
        ) {
            Ok(value) => Some(value),
            Err(error) => {
                self.invalid_model(subject, &error, port.span());
                None
            }
        }
    }

    fn port_number(&mut self, subject: &str, field: &str, value: &str, span: ComposeSpan) -> Option<u16> {
        match value.parse::<u16>() {
            Ok(0) => {
                self.invalid_value(subject, &format!("{field} port must not be zero"), span);
                None
            }
            Ok(value) => Some(value),
            Err(_) => {
                self.unsupported(subject, &format!("non-single {field} port `{value}`"), span);
                None
            }
        }
    }

    fn map_mount(&mut self, subject: &str, mount: &VolumeMount) -> Option<Mount> {
        match mount {
            VolumeMount::Short(value) => self.map_short_mount(subject, value),
            VolumeMount::Long(value) => self.map_long_mount(subject, value),
        }
    }

    fn map_short_mount(&mut self, subject: &str, mount: &ShortVolumeMount) -> Option<Mount> {
        let Some(target) = mount.target() else {
            self.invalid_value(subject, "short-syntax volume has no target", mount.raw().span());
            return None;
        };
        let source = match mount.source() {
            None => MountSource::Anonymous,
            Some(value) if is_host_path(value) => MountSource::HostPath(value.to_owned()),
            Some(value) if value.contains('$') => {
                self.unsupported(subject, "unresolved ambiguous volume source", mount.raw().span());
                return None;
            }
            Some(value) => MountSource::Volume(self.identifier(subject, value, mount.raw().span())?),
        };
        let mut read_only = false;
        let mut relabel = None;
        for option in mount.options() {
            match option.as_str() {
                "" | "rw" => {}
                "ro" => read_only = true,
                "z" => relabel = Some(SelinuxRelabel::Shared),
                "Z" => relabel = Some(SelinuxRelabel::Private),
                value => self.unsupported(subject, &format!("volume option `{value}`"), mount.raw().span()),
            }
        }
        let mut value = match Mount::new(source, target, read_only) {
            Ok(value) => value,
            Err(error) => {
                self.invalid_model(subject, &error, mount.raw().span());
                return None;
            }
        };
        if let Some(relabel) = relabel {
            value.set_selinux_relabel(relabel);
        }
        Some(value)
    }

    fn map_long_mount(&mut self, subject: &str, mount: &LongVolumeMount) -> Option<Mount> {
        let Some(mount_type) = mount.mount_type() else {
            self.invalid_value(subject, "long-syntax volume has no type", mount.span());
            return None;
        };
        let Some(target) = mount.target() else {
            self.invalid_value(subject, "long-syntax volume has no target", mount.span());
            return None;
        };
        let source = match mount_type.value() {
            MountType::Volume => match mount.source() {
                Some(value) => MountSource::Volume(self.identifier(subject, value.value(), value.span())?),
                None => MountSource::Anonymous,
            },
            MountType::Bind => {
                let Some(value) = mount.source() else {
                    self.invalid_value(subject, "long-syntax bind mount has no source", mount.span());
                    return None;
                };
                MountSource::HostPath(value.value().clone())
            }
            other => {
                self.unsupported(subject, &format!("mount type `{other:?}`"), mount.span());
                return None;
            }
        };
        let read_only = match mount.read_only().map(compose_lens::model::Located::value) {
            None | Some(BooleanValue::Literal(false)) => false,
            Some(BooleanValue::Literal(true)) => true,
            Some(BooleanValue::Expression(_)) => {
                self.invalid_value(subject, "read_only expression was not resolved", mount.span());
                return None;
            }
        };
        let mut value = match Mount::new(source, target.value(), read_only) {
            Ok(value) => value,
            Err(error) => {
                self.invalid_model(subject, &error, mount.span());
                return None;
            }
        };
        if let Some(bind) = mount.bind() {
            if bind.propagation().is_some() {
                self.unsupported(subject, "bind.propagation", bind.span());
            }
            if bind.create_host_path().is_some() {
                self.unsupported(subject, "bind.create_host_path", bind.span());
            }
            if let Some(relabel) = bind.selinux() {
                value.set_selinux_relabel(match relabel.value() {
                    ComposeSelinuxRelabel::Shared => SelinuxRelabel::Shared,
                    ComposeSelinuxRelabel::Private => SelinuxRelabel::Private,
                });
            }
            self.report_fields(subject, "bind extension", bind.extension_fields());
            self.report_fields(subject, "unknown bind field", bind.unknown_fields());
        }
        self.report_fields(subject, "volume extension", mount.extension_fields());
        self.report_fields(subject, "unknown volume field", mount.unknown_fields());
        Some(value)
    }

    fn map_service_networks(
        &mut self,
        service_subject: &str,
        networks: &ServiceNetworks,
        provenance: &MergeProvenance,
        service: &mut Service,
    ) {
        match networks {
            ServiceNetworks::Short { names, .. } => {
                for name in names {
                    let subject = format!("{service_subject}.networks.{}", name.value());
                    let Some(identifier) = self.identifier(&subject, name.value(), name.span()) else {
                        continue;
                    };
                    service.add_network(
                        self.sourced_provenance(NetworkAttachment::new(identifier, Vec::new()), provenance),
                    );
                    self.exact_provenance(subject, provenance);
                }
            }
            ServiceNetworks::Long { networks, .. } => {
                for network in networks {
                    self.map_service_network(service_subject, network, provenance, service);
                }
            }
        }
    }

    fn map_service_network(
        &mut self,
        service_subject: &str,
        network: &ServiceNetwork,
        provenance: &MergeProvenance,
        service: &mut Service,
    ) {
        let subject = format!("{service_subject}.networks.{}", network.name().value());
        let Some(identifier) = self.identifier(&subject, network.name().value(), network.name().span()) else {
            return;
        };
        let aliases = network.aliases().iter().map(|alias| alias.value().clone()).collect();
        service.add_network(self.sourced_provenance(NetworkAttachment::new(identifier, aliases), provenance));

        if network.interface_name().is_some() {
            self.unsupported(&subject, "networks.interface_name", network.span());
        }
        if network.ipv4_address().is_some() {
            self.unsupported(&subject, "networks.ipv4_address", network.span());
        }
        if network.ipv6_address().is_some() {
            self.unsupported(&subject, "networks.ipv6_address", network.span());
        }
        if !network.link_local_ips().is_empty() {
            self.unsupported(&subject, "networks.link_local_ips", network.span());
        }
        if network.mac_address().is_some() {
            self.unsupported(&subject, "networks.mac_address", network.span());
        }
        if !network.driver_opts().is_empty() {
            self.unsupported(&subject, "networks.driver_opts", network.span());
        }
        if network.gw_priority().is_some() {
            self.unsupported(&subject, "networks.gw_priority", network.span());
        }
        if network.priority().is_some() {
            self.unsupported(&subject, "networks.priority", network.span());
        }
        self.report_fields(&subject, "network attachment extension", network.extension_fields());
        self.report_fields(&subject, "unknown network attachment field", network.unknown_fields());
        self.exact_provenance(subject, provenance);
    }

    fn map_volume_definition(&mut self, resource: &ProjectResource<VolumeDefinition>) -> Option<Sourced<Volume>> {
        let native = resource.definition().value();
        let subject = format!("volumes.{}", resource.name().value());
        let name = self.identifier_optional(&subject, resource.name().value(), resource.name().effective_source())?;
        let ownership = self.resource_ownership(&subject, native.external(), native.span());
        if native.driver().is_some() {
            self.unsupported(&subject, "volume.driver", native.span());
        }
        if !native.driver_opts().is_empty() {
            self.unsupported(&subject, "volume.driver_opts", native.span());
        }
        if native.labels().is_some() {
            self.unsupported(&subject, "volume.labels", native.span());
        }
        if native.custom_name().is_some() {
            self.unsupported(&subject, "volume.name", native.span());
        }
        self.report_fields(&subject, "volume definition extension", native.extension_fields());
        self.report_fields(&subject, "unknown volume definition field", native.unknown_fields());
        self.exact_provenance(&subject, resource.definition().provenance());
        Some(self.sourced_provenance(Volume::new(name, ownership), resource.definition().provenance()))
    }

    fn map_network_definition(&mut self, resource: &ProjectResource<NetworkDefinition>) -> Option<Sourced<Network>> {
        let native = resource.definition().value();
        let subject = format!("networks.{}", resource.name().value());
        let name = self.identifier_optional(&subject, resource.name().value(), resource.name().effective_source())?;
        let ownership = self.resource_ownership(&subject, native.external(), native.span());
        if native.driver().is_some() {
            self.unsupported(&subject, "network.driver", native.span());
        }
        if !native.driver_opts().is_empty() {
            self.unsupported(&subject, "network.driver_opts", native.span());
        }
        if native.attachable().is_some() {
            self.unsupported(&subject, "network.attachable", native.span());
        }
        if native.enable_ipv4().is_some() {
            self.unsupported(&subject, "network.enable_ipv4", native.span());
        }
        if native.enable_ipv6().is_some() {
            self.unsupported(&subject, "network.enable_ipv6", native.span());
        }
        if native.internal().is_some() {
            self.unsupported(&subject, "network.internal", native.span());
        }
        if native.ipam().is_some() {
            self.unsupported(&subject, "network.ipam", native.span());
        }
        if native.labels().is_some() {
            self.unsupported(&subject, "network.labels", native.span());
        }
        if native.custom_name().is_some() {
            self.unsupported(&subject, "network.name", native.span());
        }
        self.report_fields(&subject, "network definition extension", native.extension_fields());
        self.report_fields(&subject, "unknown network definition field", native.unknown_fields());
        self.exact_provenance(&subject, resource.definition().provenance());
        Some(self.sourced_provenance(Network::new(name, ownership), resource.definition().provenance()))
    }

    fn map_config_definition(&mut self, resource: &ProjectResource<ConfigDefinition>) -> Option<Sourced<Config>> {
        let native = resource.definition().value();
        let subject = format!("configs.{}", resource.name().value());
        let name = self.identifier_optional(&subject, resource.name().value(), resource.name().effective_source())?;
        let ownership = self.resource_ownership(&subject, native.external(), native.span());
        let mut config = Config::new(name, ownership);

        if let Some(runtime_name) = native.custom_name() {
            config.set_runtime_name(self.sourced_spans(
                Self::protected(runtime_name.value(), resource.definition().is_sensitive()),
                &[runtime_name.span()],
            ));
            self.exact_spans(format!("{subject}.runtime_name"), &[runtime_name.span()]);
        }

        let materials = [
            native.file().map(|value| {
                (
                    ConfigMaterial::File(Self::protected(value.value(), resource.definition().is_sensitive())),
                    value.span(),
                )
            }),
            native.environment().map(|value| {
                (
                    ConfigMaterial::Environment(Self::protected(value.value(), resource.definition().is_sensitive())),
                    value.span(),
                )
            }),
            native.content().map(|value| {
                (
                    ConfigMaterial::Content(Self::protected(value.value(), resource.definition().is_sensitive())),
                    value.span(),
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        self.map_config_material(&subject, ownership, &materials, &mut config, resource.definition());

        self.report_fields(&subject, "config definition extension", native.extension_fields());
        self.report_fields(&subject, "unknown config definition field", native.unknown_fields());
        self.exact_provenance(&subject, resource.definition().provenance());
        Some(self.sourced_provenance(config, resource.definition().provenance()))
    }

    fn map_config_material(
        &mut self,
        subject: &str,
        ownership: ResourceOwnership,
        materials: &[(ConfigMaterial, ComposeSpan)],
        config: &mut Config,
        definition: &ProjectValue<ConfigDefinition>,
    ) {
        match materials {
            [(material, span)] => {
                config.set_material(self.sourced_spans(material.clone(), &[*span]));
                self.exact_spans(format!("{subject}.material"), &[*span]);
                if ownership == ResourceOwnership::External {
                    self.invalid_value_optional(
                        subject,
                        "external config cannot also declare application-managed material",
                        Some(*span),
                    );
                }
            }
            [] if ownership != ResourceOwnership::External => self.invalid_value_optional(
                subject,
                "application-managed config requires exactly one of file, environment, or content",
                definition.effective_source(),
            ),
            [] => {}
            _ => self.invalid_value_optional(
                subject,
                "config declares multiple material sources; exactly one of file, environment, or content is allowed",
                definition.effective_source(),
            ),
        }
    }

    fn map_secret_definition(&mut self, resource: &ProjectResource<SecretDefinition>) -> Option<Sourced<Secret>> {
        let native = resource.definition().value();
        let subject = format!("secrets.{}", resource.name().value());
        let name = self.identifier_optional(&subject, resource.name().value(), resource.name().effective_source())?;
        let ownership = self.resource_ownership(&subject, native.external(), native.span());
        let mut secret = Secret::new(name, ownership);

        if let Some(runtime_name) = native.custom_name() {
            secret.set_runtime_name(self.sourced_spans(
                Self::protected(runtime_name.value(), resource.definition().is_sensitive()),
                &[runtime_name.span()],
            ));
            self.exact_spans(format!("{subject}.runtime_name"), &[runtime_name.span()]);
        }

        let materials = [
            native.file().map(|value| {
                (
                    SecretMaterial::File(Self::protected(value.value(), resource.definition().is_sensitive())),
                    value.span(),
                )
            }),
            native.environment().map(|value| {
                (
                    SecretMaterial::Environment(Self::protected(value.value(), resource.definition().is_sensitive())),
                    value.span(),
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        self.map_secret_material(&subject, ownership, &materials, &mut secret, resource.definition());

        self.report_fields(&subject, "secret definition extension", native.extension_fields());
        self.report_fields(&subject, "unknown secret definition field", native.unknown_fields());
        self.exact_provenance(&subject, resource.definition().provenance());
        Some(self.sourced_provenance(secret, resource.definition().provenance()))
    }

    fn map_secret_material(
        &mut self,
        subject: &str,
        ownership: ResourceOwnership,
        materials: &[(SecretMaterial, ComposeSpan)],
        secret: &mut Secret,
        definition: &ProjectValue<SecretDefinition>,
    ) {
        match materials {
            [(material, span)] => {
                secret.set_material(self.sourced_spans(material.clone(), &[*span]));
                self.exact_spans(format!("{subject}.material"), &[*span]);
                if ownership == ResourceOwnership::External {
                    self.invalid_value_optional(
                        subject,
                        "external secret cannot also declare application-managed material",
                        Some(*span),
                    );
                }
            }
            [] if ownership != ResourceOwnership::External => self.invalid_value_optional(
                subject,
                "application-managed secret requires exactly one of file or environment",
                definition.effective_source(),
            ),
            [] => {}
            _ => self.invalid_value_optional(
                subject,
                "secret declares multiple material sources; exactly one of file or environment is allowed",
                definition.effective_source(),
            ),
        }
    }

    fn map_service_grants(
        &mut self,
        service_subject: &str,
        field: &str,
        grants: &ProjectValue<Vec<ProjectValue<ProjectGrant>>>,
        secret: bool,
        service: &mut Service,
    ) {
        for (index, native) in grants.value().iter().enumerate() {
            let subject = format!("{service_subject}.{field}[{index}]");
            let grant = match native.value() {
                ProjectGrant::Short(source) => ResourceGrant::new(
                    Self::protected(source, native.is_sensitive()),
                    ResourceGrantSyntax::Short,
                ),
                ProjectGrant::Long(long) => {
                    let Some(source) = long.source() else {
                        self.invalid_value_optional(
                            &subject,
                            "long-syntax resource grant requires source",
                            native.effective_source(),
                        );
                        continue;
                    };
                    let mut grant = match ResourceGrant::new(
                        Self::protected(source.value(), source.is_sensitive()),
                        ResourceGrantSyntax::Long,
                    ) {
                        Ok(grant) => grant,
                        Err(error) => {
                            self.invalid_model_optional(&subject, &error, source.effective_source());
                            continue;
                        }
                    };
                    Self::set_grant_field(&mut grant, long.target(), ResourceGrant::set_target, self);
                    Self::set_grant_field(&mut grant, long.uid(), ResourceGrant::set_uid, self);
                    Self::set_grant_field(&mut grant, long.gid(), ResourceGrant::set_gid, self);
                    Self::set_grant_field(&mut grant, long.mode(), ResourceGrant::set_mode, self);
                    self.report_project_fields(&subject, "resource-grant field", long.unmodeled_fields());
                    Ok(grant)
                }
            };
            match grant {
                Ok(grant) => {
                    let grant = self.sourced_provenance(grant, native.provenance());
                    self.exact_origins(&subject, grant.origins());
                    if secret {
                        service.add_secret_grant(grant);
                    } else {
                        service.add_config_grant(grant);
                    }
                }
                Err(error) => self.invalid_model_optional(&subject, &error, native.effective_source()),
            }
        }
    }

    fn set_grant_field(
        grant: &mut ResourceGrant,
        value: Option<&ProjectValue<String>>,
        setter: fn(&mut ResourceGrant, Sourced<ProtectedString>),
        mapping: &Self,
    ) {
        if let Some(value) = value {
            setter(
                grant,
                mapping.sourced_provenance(Self::protected(value.value(), value.is_sensitive()), value.provenance()),
            );
        }
    }

    fn resource_ownership(
        &mut self,
        subject: &str,
        external: Option<&compose_lens::model::Located<BooleanValue>>,
        span: ComposeSpan,
    ) -> ResourceOwnership {
        match external.map(compose_lens::model::Located::value) {
            Some(BooleanValue::Literal(true)) => ResourceOwnership::External,
            None | Some(BooleanValue::Literal(false)) => ResourceOwnership::Application,
            Some(BooleanValue::Expression(_)) => {
                self.invalid_value(subject, "external expression was not resolved", span);
                ResourceOwnership::Application
            }
        }
    }

    fn report_service_unsupported(&mut self, subject: &str, service: &ProjectService) {
        self.report_project_fields(subject, "service field", service.unmodeled_fields());
    }

    fn report_document_unsupported(&mut self, view: &ProjectView) {
        self.report_project_fields("application", "document field", view.unmodeled_fields());
    }

    fn report_fields(&mut self, subject: &str, kind: &str, fields: &[compose_lens::model::FieldReference]) {
        for field in fields {
            self.unsupported(subject, &format!("{kind} `{}`", field.name().value()), field.span());
        }
    }

    fn report_project_fields(&mut self, subject: &str, kind: &str, fields: &[ProjectFieldReference]) {
        for field in fields {
            let feature = field.path().join(".");
            self.unsupported_optional(
                subject,
                &format!("{kind} `{feature}`"),
                field
                    .provenance()
                    .effective_source()
                    .or_else(|| field.key().effective_source()),
            );
        }
    }

    fn identifier(&mut self, subject: &str, value: &str, span: ComposeSpan) -> Option<Identifier> {
        self.identifier_optional(subject, value, Some(span))
    }

    fn identifier_optional(&mut self, subject: &str, value: &str, span: Option<ComposeSpan>) -> Option<Identifier> {
        match Identifier::new(value) {
            Ok(value) => Some(value),
            Err(error) => {
                self.invalid_model_optional(subject, &error, span);
                None
            }
        }
    }

    fn sourced_provenance<T>(&self, value: T, provenance: &MergeProvenance) -> Sourced<T> {
        let mut sourced = Sourced::generated(value);
        for origin in provenance.sources().iter().filter_map(|span| self.origin(*span)) {
            sourced.add_origin(origin);
        }
        sourced
    }

    fn origins(&self, provenance: &MergeProvenance) -> Vec<Provenance> {
        provenance
            .sources()
            .iter()
            .filter_map(|span| self.origin(*span))
            .collect()
    }

    fn sourced_spans<T>(&self, value: T, source_spans: &[ComposeSpan]) -> Sourced<T> {
        let mut result = Sourced::generated(value);
        for origin in source_spans.iter().filter_map(|span| self.origin(*span)) {
            if !result.origins().contains(&origin) {
                result.add_origin(origin);
            }
        }
        result
    }

    fn sourced_project_key_value<T>(
        &self,
        value: T,
        key_sources: &[ComposeSpan],
        value_provenance: &MergeProvenance,
    ) -> Sourced<T> {
        let mut result = self.sourced_spans(value, key_sources);
        for origin in value_provenance.sources().iter().filter_map(|span| self.origin(*span)) {
            if !result.origins().contains(&origin) {
                result.add_origin(origin);
            }
        }
        result
    }

    fn sourced_host_mapping(
        &self,
        value: HostMapping,
        hostname_sources: &[ComposeSpan],
        address_provenance: &MergeProvenance,
    ) -> Sourced<HostMapping> {
        let mut sourced = Sourced::generated(value);
        for origin in hostname_sources
            .iter()
            .chain(address_provenance.sources())
            .filter_map(|span| self.origin(*span))
        {
            if !sourced.origins().contains(&origin) {
                sourced.add_origin(origin);
            }
        }
        sourced
    }

    fn sourced_metadata_label(
        &self,
        value: MetadataLabel,
        name_sources: &[ComposeSpan],
        value_provenance: &MergeProvenance,
    ) -> Sourced<MetadataLabel> {
        let mut sourced = Sourced::generated(value);
        for origin in name_sources
            .iter()
            .chain(value_provenance.sources())
            .filter_map(|span| self.origin(*span))
        {
            if !sourced.origins().contains(&origin) {
                sourced.add_origin(origin);
            }
        }
        sourced
    }

    fn exact_provenance(&mut self, subject: impl Into<String>, provenance: &MergeProvenance) {
        let outcome = ConversionOutcome::exact(subject);
        let outcome = self.with_provenance(outcome, provenance);
        self.outcomes.push(outcome);
    }

    fn exact_origins(&mut self, subject: impl Into<String>, origins: &[Provenance]) {
        let mut outcome = ConversionOutcome::exact(subject);
        for origin in origins {
            outcome = outcome.with_origin(origin.clone());
        }
        self.outcomes.push(outcome);
    }

    fn exact_spans(&mut self, subject: impl Into<String>, spans: &[ComposeSpan]) {
        let value = self.sourced_spans((), spans);
        self.exact_origins(subject, value.origins());
    }

    fn unsupported(&mut self, subject: &str, feature: &str, span: ComposeSpan) {
        self.unsupported_optional(subject, feature, Some(span));
    }

    fn unsupported_optional(&mut self, subject: &str, feature: &str, span: Option<ComposeSpan>) {
        let code = self.codes.unsupported.clone();
        self.diagnostics.push(
            Diagnostic::new(
                code.clone(),
                Severity::Warning,
                "Compose intent is not represented by the current neutral-model subset",
            )
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
            .with_field(DiagnosticField::new("feature", DiagnosticValue::plain(feature))),
        );
        if let Ok(outcome) = ConversionOutcome::loss(subject, ConversionKind::Unsupported, code) {
            self.outcomes.push(self.with_optional_origin(outcome, span));
        }
    }

    fn invalid_model(&mut self, subject: &str, error: &ModelError, span: ComposeSpan) {
        self.invalid_model_optional(subject, error, Some(span));
    }

    fn invalid_model_optional(&mut self, subject: &str, error: &ModelError, span: Option<ComposeSpan>) {
        self.invalid_optional_span(
            self.codes.invalid_model.clone(),
            subject,
            "Compose value cannot be represented in the neutral application model",
            &error.to_string(),
            span,
        );
    }

    fn invalid_value(&mut self, subject: &str, reason: &str, span: ComposeSpan) {
        self.invalid_with_code(
            self.codes.invalid_value.clone(),
            subject,
            "Compose value must be resolved or corrected before conversion",
            reason,
            span,
        );
    }

    fn invalid_value_optional(&mut self, subject: &str, reason: &str, span: Option<ComposeSpan>) {
        self.invalid_optional_span(
            self.codes.invalid_value.clone(),
            subject,
            "Compose value must be resolved or corrected before conversion",
            reason,
            span,
        );
    }

    fn invalid_with_code(
        &mut self,
        code: DiagnosticCode,
        subject: &str,
        summary: &str,
        reason: &str,
        span: ComposeSpan,
    ) {
        self.invalid_optional_span(code, subject, summary, reason, Some(span));
    }

    fn invalid_optional_span(
        &mut self,
        code: DiagnosticCode,
        subject: &str,
        summary: &str,
        reason: &str,
        span: Option<ComposeSpan>,
    ) {
        self.diagnostics.push(
            Diagnostic::new(code.clone(), Severity::Error, summary)
                .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
                .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason))),
        );
        if let Ok(outcome) = ConversionOutcome::loss(subject, ConversionKind::Invalid, code) {
            self.outcomes.push(self.with_optional_origin(outcome, span));
        }
    }

    fn report_project_diagnostics(&mut self, diagnostics: &[compose_lens::diagnostic::Diagnostic]) {
        for diagnostic in diagnostics {
            let severity = match diagnostic.severity() {
                compose_lens::diagnostic::Severity::Error => Severity::Error,
                compose_lens::diagnostic::Severity::Warning => Severity::Warning,
                compose_lens::diagnostic::Severity::Note => Severity::Note,
            };
            let code = self.codes.invalid_value.clone();
            self.diagnostics.push(
                Diagnostic::new(
                    code.clone(),
                    severity,
                    "ComposeLens could not fully type the merged project",
                )
                .with_field(DiagnosticField::new(
                    "compose_code",
                    DiagnosticValue::plain(diagnostic.code().as_str()),
                ))
                .with_field(DiagnosticField::new(
                    "reason",
                    DiagnosticValue::plain(diagnostic.message()),
                )),
            );
            let kind = if severity == Severity::Error {
                ConversionKind::Invalid
            } else {
                ConversionKind::Unsupported
            };
            if let Ok(mut outcome) = ConversionOutcome::loss("application", kind, code) {
                for origin in diagnostic.labels().iter().filter_map(|label| self.origin(label.span())) {
                    outcome = outcome.with_origin(origin);
                }
                self.outcomes.push(outcome);
            }
        }
    }

    fn with_origin(&self, outcome: ConversionOutcome, span: ComposeSpan) -> ConversionOutcome {
        match self.origin(span) {
            Some(origin) => outcome.with_origin(origin),
            None => outcome,
        }
    }

    fn with_optional_origin(&self, outcome: ConversionOutcome, span: Option<ComposeSpan>) -> ConversionOutcome {
        match span {
            Some(span) => self.with_origin(outcome, span),
            None => outcome,
        }
    }

    fn with_provenance(&self, mut outcome: ConversionOutcome, provenance: &MergeProvenance) -> ConversionOutcome {
        for origin in provenance.sources().iter().filter_map(|span| self.origin(*span)) {
            outcome = outcome.with_origin(origin);
        }
        outcome
    }

    fn origin(&self, span: ComposeSpan) -> Option<Provenance> {
        let source_id = self.source.source_id(span.source_id())?.clone();
        SourceSpan::new(span.start(), span.end())
            .ok()
            .map(|span| Provenance::spanned(source_id, span))
    }
}

fn scalar_text(value: &ComposeScalar) -> String {
    match value {
        ComposeScalar::Null => String::new(),
        ComposeScalar::Boolean(value) => value.to_string(),
        ComposeScalar::Number(value) | ComposeScalar::String(value) => value.clone(),
    }
}

fn scalar_option(value: &ComposeScalar) -> Option<String> {
    match value {
        ComposeScalar::Null => None,
        value => Some(scalar_text(value)),
    }
}

fn literal_assignment(value: &str) -> Option<(&str, &str)> {
    let (name, value) = value.split_once('=')?;
    if name.is_empty() || name.contains('$') || value.contains('$') {
        return None;
    }
    Some((name, value))
}

fn map_protocol(protocol: Option<&str>) -> Protocol {
    match protocol.unwrap_or("tcp").to_ascii_lowercase().as_str() {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        "sctp" => Protocol::Sctp,
        value => Protocol::Other(value.to_owned()),
    }
}

fn is_host_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('~')
        || value.starts_with('%')
        || value.starts_with("//")
        || value.starts_with(r"\\")
        || value
            .as_bytes()
            .get(0..2)
            .is_some_and(|prefix| prefix[0].is_ascii_alphabetic() && prefix[1] == b':')
}
