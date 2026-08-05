//! Compose-to-application mapping with explicit fidelity decisions.

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    Application, Command, Config, ConfigMaterial, EnvironmentFile as NeutralEnvironmentFile,
    EnvironmentFileFormat as NeutralEnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue,
    EnvironmentVariable, Healthcheck, HealthcheckCommand, HealthcheckDuration as NeutralHealthcheckDuration,
    HealthcheckRetries as NeutralHealthcheckRetries, HostAddress, HostMapping, Identifier, ImageReference,
    MetadataLabel, ModelError, Mount, MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol,
    Provenance, ResourceGrant, ResourceGrantSyntax, ResourceOwnership, RestartPolicy as NeutralRestartPolicy, Secret,
    SecretMaterial, SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition, SourceSpan, Sourced,
    Volume,
};
use compose_lens::merge::MergeProvenance;
use compose_lens::model::{
    BooleanValue, ComposeScalar, ConfigDefinition, DependencyCondition as ComposeDependencyCondition,
    EnvironmentFileFormatKind, HealthcheckDuration as ComposeHealthcheckDuration,
    HealthcheckRetries as ComposeHealthcheckRetries, HealthcheckTest, HealthcheckTestKind, LongPort, LongVolumeMount,
    MountType, NetworkDefinition, Port as ComposePort, RestartPolicyKind as ComposeRestartPolicyKind, SecretDefinition,
    SelinuxRelabel as ComposeSelinuxRelabel, ServiceNetwork, ServiceNetworks, ShortPort, ShortVolumeMount,
    VolumeDefinition, VolumeMount,
};
use compose_lens::project::{
    ProjectDependsOn, ProjectEnvironment, ProjectEnvironmentFile, ProjectFieldReference, ProjectGrant,
    ProjectHealthcheck, ProjectLabels, ProjectResource, ProjectService, ProjectValue, ProjectView, build_project_view,
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
        for service in view.services() {
            let active = if service.profiles().is_none_or(|profiles| profiles.value().is_empty()) {
                true
            } else {
                profiles_valid
                    && source
                        .profile_selection()
                        .is_some_and(|selection| selection.is_active(service.name().value()))
            };
            if !active {
                continue;
            }
            if let Some(service) = mapping.map_service(service) {
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
        self.map_service_environment(&subject, native, &mut service);
        if let Some(labels) = native.labels() {
            self.map_labels(&subject, labels.value(), &mut service);
        }
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

    fn sourced_spans<T>(&self, value: T, source_spans: &[ComposeSpan]) -> Sourced<T> {
        let mut result = Sourced::generated(value);
        for origin in source_spans.iter().filter_map(|span| self.origin(*span)) {
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
