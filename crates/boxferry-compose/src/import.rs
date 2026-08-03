//! Compose-to-application mapping with explicit fidelity decisions.

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    Application, Command, EnvironmentValue, EnvironmentVariable, Identifier, ImageReference, ModelError, Mount,
    MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, ResourceOwnership,
    SelinuxRelabel, Service, SourceSpan, Sourced, Volume,
};
use compose_lens::merge::MergeProvenance;
use compose_lens::model::{
    BooleanValue, ComposeScalar, LongPort, LongVolumeMount, MountType, NetworkDefinition, Port as ComposePort,
    SelinuxRelabel as ComposeSelinuxRelabel, ServiceNetwork, ServiceNetworks, ShortPort, ShortVolumeMount,
    VolumeDefinition, VolumeMount,
};
use compose_lens::project::{
    ProjectEnvironment, ProjectFieldReference, ProjectResource, ProjectService, ProjectView, build_project_view,
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
        if let Some(environment) = native.environment() {
            self.map_environment(&subject, environment.value(), &mut service);
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
        if let Some(networks) = native.networks() {
            self.map_service_networks(&subject, networks.value(), networks.provenance(), &mut service);
        }
        if let Some(profiles) = native.profiles() {
            if !profiles.value().is_empty() {
                self.exact_provenance(format!("{subject}.profiles"), profiles.provenance());
            }
        }

        self.report_service_unsupported(&subject, native);
        self.exact_provenance(&subject, native.provenance());
        Some(self.sourced_provenance(service, native.provenance()))
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
        for config in view.configs() {
            self.unsupported_optional(
                &format!("configs.{}", config.name().value()),
                "top-level config",
                config.definition().effective_source(),
            );
        }
        for secret in view.secrets() {
            self.unsupported_optional(
                &format!("secrets.{}", secret.name().value()),
                "top-level secret",
                secret.definition().effective_source(),
            );
        }
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

    fn exact_provenance(&mut self, subject: impl Into<String>, provenance: &MergeProvenance) {
        let outcome = ConversionOutcome::exact(subject);
        let outcome = self.with_provenance(outcome, provenance);
        self.outcomes.push(outcome);
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
