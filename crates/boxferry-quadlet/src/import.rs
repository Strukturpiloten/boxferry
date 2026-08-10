//! Quadlet-to-application mapping with explicit fidelity decisions.

use std::collections::{BTreeMap, BTreeSet};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    Application, Command, EnvironmentFile, EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable, Healthcheck,
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier,
    ImageReference, MetadataLabel, Mount, MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol,
    Provenance, ResourceGrant, ResourceGrantSyntax, ResourceOwnership, RestartPolicy, Secret, SecurityOption,
    SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition, ServiceGroup, SourceSpan, Sourced, Volume,
};
use quadlet_lens::model::{
    ContainerKey, EntryKind, PodKey, QuadletUnitType, SectionKind, TypedEntry, TypedSection, UnitReferenceKind,
    ValueKind,
};

use crate::QuadletSource;

/// Maps an explicitly parsed Quadlet document set into `BoxFerry`'s neutral model.
#[derive(Clone, Debug)]
pub struct QuadletImporter {
    codes: Codes,
}

impl QuadletImporter {
    /// Creates an importer and validates its stable machine-readable diagnostic codes.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only when a code embedded in this adapter is invalid.
    pub fn new() -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            codes: Codes {
                invalid_source: DiagnosticCode::new("BFQ1001")?,
                invalid_model: DiagnosticCode::new("BFQ1002")?,
                unsupported: DiagnosticCode::new("BFQ1003")?,
                approximate: DiagnosticCode::new("BFQ1004")?,
            },
        })
    }
}

impl ImportAdapter for QuadletImporter {
    type Source = QuadletSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let mut mapping = Mapping::new(&self.codes, source);
        if !source.documents().is_valid() {
            mapping.invalid_source();
            return ImportResult::new(None, mapping.outcomes, mapping.diagnostics);
        }

        let mut application = Application::new(source.application_name().clone());
        mapping.exact("application", None);
        let service_names: BTreeSet<_> = source
            .documents()
            .documents()
            .iter()
            .filter(|named| named.document().unit_type() == QuadletUnitType::Container)
            .filter_map(|named| {
                let stem = unit_stem(named.name().as_str());
                Identifier::new(stem).is_ok().then(|| stem.to_owned())
            })
            .collect();
        let (pod_order, mut pod_groups) = mapping.declare_resources(&mut application);

        for named in source.documents().documents() {
            let document = named.document();
            let filename = named.name().as_str();
            let stem = unit_stem(filename);
            let document_origin = mapping.document_origin(document.source_span());

            match document.unit_type() {
                QuadletUnitType::Container => {
                    let Some(name) = mapping.identifier(stem, "services", document_origin.clone()) else {
                        continue;
                    };
                    let mut service = Service::new(name);
                    mapping.map_container(
                        filename,
                        &mut application,
                        &mut service,
                        document.sections(),
                        &service_names,
                        &mut pod_groups,
                    );
                    if let Err(error) = application.add_service(Sourced::from_source(service, document_origin)) {
                        mapping.invalid_model("services", filename, &error.to_string(), None);
                    }
                }
                QuadletUnitType::Network | QuadletUnitType::Volume | QuadletUnitType::Pod => {}
                _ => mapping.unsupported(
                    &format!("quadlet.{filename}"),
                    filename,
                    "unsupported unit type",
                    document_origin,
                ),
            }
        }

        mapping.finish_pod_groups(&mut application, pod_order, pod_groups);

        ImportResult::new(Some(application), mapping.outcomes, mapping.diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_source: DiagnosticCode,
    invalid_model: DiagnosticCode,
    unsupported: DiagnosticCode,
    approximate: DiagnosticCode,
}

struct Mapping<'a> {
    codes: &'a Codes,
    source: &'a QuadletSource,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Default)]
struct DependencyRelations {
    order: Vec<String>,
    by_service: BTreeMap<String, DependencyRelation>,
}

#[derive(Default)]
struct DependencyRelation {
    activation: Option<DependencyActivation>,
    conflicting_activation: bool,
    after_origins: Vec<Provenance>,
}

struct DependencyActivation {
    required: bool,
    origins: Vec<Provenance>,
}

#[derive(Default)]
struct ContainerImportState<'entry> {
    singletons: BTreeSet<&'static str>,
    group_entry: Option<&'entry TypedEntry>,
    restart_seen: bool,
    relations: DependencyRelations,
    healthcheck: Healthcheck,
    health_origins: Vec<Provenance>,
    security_options: Vec<Sourced<SecurityOption>>,
    security_option_origins: Vec<Provenance>,
}

struct PodImportGroup {
    filename: String,
    name: Identifier,
    origin: Provenance,
    members: Vec<Sourced<Identifier>>,
}

impl DependencyRelations {
    fn record_activation(&mut self, service: &str, required: bool, origin: Provenance) {
        let relation = self.relation_mut(service);
        match relation.activation.as_mut() {
            Some(activation) if activation.required == required => activation.origins.push(origin),
            Some(_) => relation.conflicting_activation = true,
            None => {
                relation.activation = Some(DependencyActivation {
                    required,
                    origins: vec![origin],
                });
            }
        }
    }

    fn record_after(&mut self, service: &str, origin: Provenance) {
        self.relation_mut(service).after_origins.push(origin);
    }

    fn relation_mut(&mut self, service: &str) -> &mut DependencyRelation {
        if !self.by_service.contains_key(service) {
            self.order.push(service.to_owned());
        }
        self.by_service.entry(service.to_owned()).or_default()
    }
}

impl<'a> Mapping<'a> {
    fn new(codes: &'a Codes, source: &'a QuadletSource) -> Self {
        Self {
            codes,
            source,
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn declare_resources(&mut self, application: &mut Application) -> (Vec<String>, BTreeMap<String, PodImportGroup>) {
        let mut pod_groups = BTreeMap::new();
        let mut pod_order = Vec::new();

        // Native document order is not dependency order. Containers may legitimately precede
        // their owned network, volume, or pod units.
        for named in self.source.documents().documents() {
            let document = named.document();
            let filename = named.name().as_str();
            let stem = unit_stem(filename);
            let document_origin = self.document_origin(document.source_span());

            match document.unit_type() {
                QuadletUnitType::Network => {
                    if let Some(name) = self.identifier(stem, "networks", document_origin.clone()) {
                        let network = Network::new(name, ResourceOwnership::Application);
                        if let Err(error) = application.add_network(Sourced::from_source(network, document_origin)) {
                            self.invalid_model("networks", filename, &error.to_string(), None);
                        } else {
                            self.exact(format!("networks.{stem}"), None);
                        }
                    }
                    self.report_unmapped_entries(filename, document.entries());
                }
                QuadletUnitType::Volume => {
                    if let Some(name) = self.identifier(stem, "volumes", document_origin.clone()) {
                        let volume = Volume::new(name, ResourceOwnership::Application);
                        if let Err(error) = application.add_volume(Sourced::from_source(volume, document_origin)) {
                            self.invalid_model("volumes", filename, &error.to_string(), None);
                        } else {
                            self.exact(format!("volumes.{stem}"), None);
                        }
                    }
                    self.report_unmapped_entries(filename, document.entries());
                }
                QuadletUnitType::Pod => {
                    if let Some(group) = self.map_pod_definition(filename, stem, document.sections(), document_origin) {
                        pod_order.push(stem.to_owned());
                        pod_groups.insert(stem.to_owned(), group);
                    }
                }
                _ => {}
            }
        }
        (pod_order, pod_groups)
    }

    fn finish_pod_groups(
        &mut self,
        application: &mut Application,
        pod_order: Vec<String>,
        mut pod_groups: BTreeMap<String, PodImportGroup>,
    ) {
        for pod_name in pod_order {
            let Some(imported) = pod_groups.remove(&pod_name) else {
                continue;
            };
            let mut group = ServiceGroup::new(imported.name, ResourceOwnership::Application);
            for member in imported.members {
                if let Err(error) = group.add_member(member) {
                    self.invalid_model(
                        &format!("service_groups.{pod_name}.members"),
                        &imported.filename,
                        &error.to_string(),
                        Some(imported.origin.clone()),
                    );
                }
            }
            if let Err(error) = application.add_service_group(Sourced::from_source(group, imported.origin.clone())) {
                self.invalid_model(
                    &format!("service_groups.{pod_name}"),
                    &imported.filename,
                    &error.to_string(),
                    Some(imported.origin),
                );
            } else {
                self.exact(format!("service_groups.{pod_name}"), None);
            }
        }
    }

    fn map_pod_definition(
        &mut self,
        filename: &str,
        stem: &str,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Option<PodImportGroup> {
        let name = self.identifier(stem, "service_groups", document_origin.clone())?;
        let subject = format!("service_groups.{stem}.runtime_name");
        let mut pod_name_seen = false;

        for section in sections {
            for entry in section.entries() {
                match (section.kind(), entry.kind()) {
                    (SectionKind::Pod, EntryKind::Pod(PodKey::PodName)) => {
                        let origin = self.entry_origin(entry);
                        if pod_name_seen {
                            self.invalid_model(
                                &subject,
                                filename,
                                "Quadlet PodName is declared more than once",
                                Some(origin),
                            );
                            continue;
                        }
                        pod_name_seen = true;
                        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                            continue;
                        };
                        if value == stem {
                            self.exact(subject.clone(), Some(origin));
                        } else {
                            self.unsupported_value(
                                &subject,
                                filename,
                                "PodName",
                                "the neutral service-group identity cannot retain a runtime pod name different from its Quadlet unit name",
                                origin,
                            );
                        }
                    }
                    _ => self.unsupported(
                        &format!("service_groups.{stem}.quadlet.{}", entry.key().text()),
                        filename,
                        entry.key().text(),
                        self.entry_origin(entry),
                    ),
                }
            }
        }

        if !pod_name_seen {
            self.unsupported_value(
                &subject,
                filename,
                "PodName",
                "an omitted PodName uses Podman's systemd-prefixed runtime default, which the neutral service-group identity cannot retain separately",
                document_origin.clone(),
            );
        }

        Some(PodImportGroup {
            filename: filename.to_owned(),
            name,
            origin: document_origin,
            members: Vec::new(),
        })
    }

    fn map_pod_membership(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        pod_groups: &mut BTreeMap<String, PodImportGroup>,
    ) {
        let subject = format!("services.{service_name}.service_group");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some(pod_name) = value.strip_suffix(".pod") else {
            self.unsupported_value(
                &subject,
                filename,
                "Pod",
                "Quadlet pod membership must be an unescaped sibling .pod unit reference",
                origin,
            );
            return;
        };
        let Some(group) = pod_groups.get_mut(pod_name) else {
            self.unsupported_value(
                &subject,
                filename,
                "Pod",
                "Quadlet pod membership does not reference an imported sibling .pod unit",
                origin,
            );
            return;
        };
        let Ok(member) = Identifier::new(service_name) else {
            return;
        };
        group.members.push(Sourced::from_source(member, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_container(
        &mut self,
        filename: &str,
        application: &mut Application,
        service: &mut Service,
        sections: &[TypedSection],
        service_names: &BTreeSet<String>,
        pod_groups: &mut BTreeMap<String, PodImportGroup>,
    ) {
        let service_name = service.name().as_str().to_owned();
        let mut state = ContainerImportState::default();

        for section in sections {
            match section.kind() {
                SectionKind::Container => {
                    self.map_native_container_section(
                        filename,
                        application,
                        service,
                        section.entries(),
                        &mut state,
                        pod_groups,
                    );
                }
                SectionKind::Unit => self.collect_unit_relations(
                    filename,
                    &service_name,
                    section.entries(),
                    service_names,
                    &mut state.relations,
                ),
                SectionKind::Service => self.map_systemd_service_section(
                    filename,
                    &service_name,
                    service,
                    section.entries(),
                    &mut state.restart_seen,
                ),
                _ => {
                    for entry in section.entries() {
                        self.unsupported_entry(filename, &service_name, entry);
                    }
                }
            }
        }
        self.map_unit_relations(filename, &service_name, service, state.relations);
        if let Some(entry) = state.group_entry {
            self.map_group(filename, &service_name, service, entry);
        }
        if !state.security_option_origins.is_empty() {
            let has_disable = state
                .security_options
                .iter()
                .any(|option| matches!(option.value(), SecurityOption::SecurityLabelDisable(true)));
            let has_other_labels = state
                .security_options
                .iter()
                .any(|option| security_option_is_selinux_label(option.value()));
            if has_disable && has_other_labels {
                self.unsupported_value(
                    &format!("services.{service_name}.security_options"),
                    filename,
                    "SecurityLabelDisable",
                    "SecurityLabelDisable=true conflicts with additional SELinux label settings; the native values remain explicit but need caller review",
                    state.security_option_origins[0].clone(),
                );
            }
            service.set_security_options_with_origins(state.security_options, state.security_option_origins);
        }
        if !state.health_origins.is_empty() {
            service.set_healthcheck(sourced_with_origins(state.healthcheck, &state.health_origins));
        }
    }

    fn map_native_container_section<'entry>(
        &mut self,
        filename: &str,
        application: &mut Application,
        service: &mut Service,
        entries: &'entry [TypedEntry],
        state: &mut ContainerImportState<'entry>,
        pod_groups: &mut BTreeMap<String, PodImportGroup>,
    ) {
        let service_name = service.name().as_str().to_owned();
        for entry in entries {
            let duplicate_field = match entry.kind() {
                EntryKind::Container(key) => singleton_field(key).filter(|field| !state.singletons.insert(*field)),
                _ => None,
            };
            if let Some(field) = duplicate_field {
                self.invalid_model(
                    &format!("services.{service_name}.{field}"),
                    filename,
                    "Quadlet singleton key is declared more than once",
                    Some(self.entry_origin(entry)),
                );
                continue;
            }
            if is_security_container_key(entry.kind()) {
                self.map_security_option(filename, &service_name, entry, state);
                continue;
            }
            if self.map_container_health_entry(filename, &service_name, entry, state) {
                continue;
            }
            match entry.kind() {
                EntryKind::Container(ContainerKey::Image) => {
                    self.map_image(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::ContainerName) => {
                    self.map_container_name(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Exec) => {
                    self.map_command(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Environment) => {
                    self.map_environment(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::EnvironmentFile) => {
                    self.map_environment_file(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::DNS) => {
                    self.map_dns_value(filename, &service_name, service, entry, "dns");
                }
                EntryKind::Container(ContainerKey::DNSOption) => {
                    self.map_dns_value(filename, &service_name, service, entry, "dns_opt");
                }
                EntryKind::Container(ContainerKey::DNSSearch) => {
                    self.map_dns_value(filename, &service_name, service, entry, "dns_search");
                }
                EntryKind::Container(ContainerKey::PublishPort) => {
                    self.map_port(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Volume) => {
                    self.map_mount(filename, &service_name, application, service, entry);
                }
                EntryKind::Container(ContainerKey::Network) => {
                    self.map_network(filename, &service_name, application, service, entry);
                }
                EntryKind::Container(ContainerKey::Pod) => {
                    self.map_pod_membership(filename, &service_name, entry, pod_groups);
                }
                EntryKind::Container(ContainerKey::Label) => {
                    self.map_label(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Secret) => {
                    self.map_secret_grant(filename, &service_name, application, service, entry);
                }
                EntryKind::Container(ContainerKey::AddHost) => {
                    self.map_host_mapping(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::User) => {
                    self.map_user(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Group) => state.group_entry = Some(entry),
                EntryKind::Container(ContainerKey::GroupAdd) => {
                    self.map_supplementary_group(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::UserNS) => {
                    self.map_user_namespace(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::WorkingDir) => {
                    self.map_working_directory(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::ReadOnly) => {
                    self.map_read_only(filename, &service_name, service, entry);
                }
                _ => self.unsupported_entry(filename, &service_name, entry),
            }
        }
    }

    fn map_container_health_entry(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        state: &mut ContainerImportState<'_>,
    ) -> bool {
        match entry.kind() {
            EntryKind::Container(ContainerKey::HealthCmd) => {
                self.map_health_command(filename, service_name, entry, state);
            }
            EntryKind::Container(ContainerKey::HealthInterval) => {
                self.map_health_duration(filename, service_name, entry, state, HealthDurationField::Interval);
            }
            EntryKind::Container(ContainerKey::HealthTimeout) => {
                self.map_health_duration(filename, service_name, entry, state, HealthDurationField::Timeout);
            }
            EntryKind::Container(ContainerKey::HealthStartPeriod) => {
                self.map_health_duration(filename, service_name, entry, state, HealthDurationField::StartPeriod);
            }
            EntryKind::Container(ContainerKey::HealthRetries) => {
                self.map_health_retries(filename, service_name, entry, state);
            }
            _ => return false,
        }
        true
    }

    fn map_security_option(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        state: &mut ContainerImportState<'_>,
    ) {
        let origin = self.entry_origin(entry);
        state.security_option_origins.push(origin.clone());
        let index = state.security_options.len();
        let subject = format!("services.{service_name}.security_options[{index}]");
        let Some(value) = self.security_option_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let option = match entry.kind() {
            EntryKind::Container(ContainerKey::AppArmor) => SecurityOption::AppArmor(ProtectedString::sensitive(value)),
            EntryKind::Container(ContainerKey::NoNewPrivileges) => {
                let Some(value) = parse_security_option_boolean(value) else {
                    self.unsupported_value(
                        &subject,
                        filename,
                        entry.key().text(),
                        "NoNewPrivileges must use a supported systemd boolean spelling",
                        origin,
                    );
                    return;
                };
                SecurityOption::NoNewPrivileges(value)
            }
            EntryKind::Container(ContainerKey::SeccompProfile) => {
                SecurityOption::SeccompProfile(ProtectedString::sensitive(value))
            }
            EntryKind::Container(ContainerKey::SecurityLabelDisable) => {
                let Some(value) = parse_security_option_boolean(value) else {
                    self.unsupported_value(
                        &subject,
                        filename,
                        entry.key().text(),
                        "SecurityLabelDisable must use a supported systemd boolean spelling",
                        origin,
                    );
                    return;
                };
                SecurityOption::SecurityLabelDisable(value)
            }
            EntryKind::Container(ContainerKey::SecurityLabelFileType) => {
                SecurityOption::SecurityLabelFileType(ProtectedString::sensitive(value))
            }
            EntryKind::Container(ContainerKey::SecurityLabelLevel) => {
                SecurityOption::SecurityLabelLevel(ProtectedString::sensitive(value))
            }
            EntryKind::Container(ContainerKey::SecurityLabelNested) => {
                let Some(value) = parse_security_option_boolean(value) else {
                    self.unsupported_value(
                        &subject,
                        filename,
                        entry.key().text(),
                        "SecurityLabelNested must use a supported systemd boolean spelling",
                        origin,
                    );
                    return;
                };
                SecurityOption::SecurityLabelNested(value)
            }
            EntryKind::Container(ContainerKey::SecurityLabelType) => {
                SecurityOption::SecurityLabelType(ProtectedString::sensitive(value))
            }
            EntryKind::Container(ContainerKey::Mask) => SecurityOption::Mask(ProtectedString::sensitive(value)),
            EntryKind::Container(ContainerKey::Unmask) => SecurityOption::Unmask(ProtectedString::sensitive(value)),
            _ => return,
        };
        state
            .security_options
            .push(Sourced::from_source(option, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn security_option_value<'entry>(
        &mut self,
        filename: &str,
        subject: &str,
        entry: &'entry TypedEntry,
        origin: Provenance,
    ) -> Option<&'entry str> {
        if entry.value().is_continued() {
            self.unsupported_value(
                subject,
                filename,
                entry.key().text(),
                "security-option continuation requires native semantic decoding before import",
                origin,
            );
            return None;
        }
        let value = entry.value().primary().text();
        if !is_safe_security_option_value(value) {
            self.unsupported_value(
                subject,
                filename,
                entry.key().text(),
                "security-option must be a non-empty unquoted physical line without systemd specifiers",
                origin,
            );
            return None;
        }
        Some(value)
    }

    fn map_container_name(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let origin = self.entry_origin(entry);
        let subject = format!("services.{service_name}.runtime_name");
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if value.is_empty() || value.contains('\0') {
            self.invalid_model(
                &subject,
                filename,
                "container name must be non-empty and contain no NUL byte",
                Some(origin),
            );
        } else {
            service.set_runtime_name(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
            self.exact(subject, Some(origin));
        }
    }

    fn collect_unit_relations(
        &mut self,
        filename: &str,
        service_name: &str,
        entries: &[TypedEntry],
        service_names: &BTreeSet<String>,
        relations: &mut DependencyRelations,
    ) {
        for entry in entries {
            let relation = match (entry.kind(), entry.key().text()) {
                (EntryKind::GenericSystemd, "Requires") => Some(Some(true)),
                (EntryKind::GenericSystemd, "Wants") => Some(Some(false)),
                (EntryKind::GenericSystemd, "After") => Some(None),
                _ => None,
            };
            let Some(relation) = relation else {
                self.unsupported_entry(filename, service_name, entry);
                continue;
            };
            self.collect_unit_relation_entry(filename, service_name, entry, relation, service_names, relations);
        }
    }

    fn collect_unit_relation_entry(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        required: Option<bool>,
        service_names: &BTreeSet<String>,
        relations: &mut DependencyRelations,
    ) {
        let subject = format!("services.{service_name}.quadlet.{}", entry.key().text());
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let references: Vec<_> = value.split_ascii_whitespace().collect();
        if references.is_empty() {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "empty systemd dependency directives do not define a neutral service edge",
                origin,
            );
            return;
        }

        for (index, reference) in references.into_iter().enumerate() {
            let item_subject = format!("{subject}[{index}]");
            let Some(target) = sibling_service_reference(reference, service_names) else {
                self.unsupported_value(
                    &item_subject,
                    filename,
                    entry.key().text(),
                    "systemd dependency is not an unescaped reference to a sibling .container or generated .service unit",
                    origin.clone(),
                );
                continue;
            };
            if target == service_name {
                self.invalid_model(
                    &item_subject,
                    filename,
                    "a service cannot depend on itself",
                    Some(origin.clone()),
                );
                continue;
            }
            if let Some(required) = required {
                relations.record_activation(target, required, origin.clone());
            } else {
                relations.record_after(target, origin.clone());
            }
        }
    }

    fn map_unit_relations(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        mut relations: DependencyRelations,
    ) {
        for target in relations.order {
            let Some(relation) = relations.by_service.remove(&target) else {
                continue;
            };
            let index = service.dependencies().len();
            let subject = format!("services.{service_name}.dependencies[{index}]");
            if relation.conflicting_activation {
                let origin = relation
                    .activation
                    .as_ref()
                    .and_then(|activation| activation.origins.first())
                    .cloned();
                self.invalid_model(
                    &subject,
                    filename,
                    "the same service is declared through both Requires and Wants",
                    origin,
                );
                continue;
            }
            let Some(activation) = relation.activation else {
                if let Some(origin) = relation.after_origins.first().cloned() {
                    self.unsupported_value(
                        &subject,
                        filename,
                        "After",
                        "After without Requires or Wants is ordering only and cannot become a neutral service dependency",
                        origin,
                    );
                }
                continue;
            };
            if relation.after_origins.is_empty() {
                if let Some(origin) = activation.origins.first().cloned() {
                    self.unsupported_value(
                        &subject,
                        filename,
                        if activation.required { "Requires" } else { "Wants" },
                        "Requires or Wants without After activates a unit but does not express neutral startup ordering",
                        origin,
                    );
                }
                continue;
            }

            let Ok(target_identifier) = Identifier::new(&target) else {
                continue;
            };
            let mut dependency = ServiceDependency::new(target_identifier);
            dependency.set_required(sourced_with_origins(activation.required, &activation.origins));
            dependency.set_condition(sourced_with_origins(
                ServiceDependencyCondition::Started,
                &relation.after_origins,
            ));
            let mut origins = activation.origins;
            origins.extend(relation.after_origins);
            let mut sourced = Sourced::from_source(dependency, origins[0].clone());
            for origin in origins.iter().skip(1) {
                sourced.add_origin(origin.clone());
            }
            service.add_dependency(sourced);
            self.exact_origins(subject, origins);
        }
    }

    fn map_systemd_service_section(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entries: &[TypedEntry],
        restart_seen: &mut bool,
    ) {
        for entry in entries {
            if entry.kind() != EntryKind::GenericSystemd || entry.key().text() != "Restart" {
                self.unsupported_entry(filename, service_name, entry);
                continue;
            }
            let origin = self.entry_origin(entry);
            let subject = format!("services.{service_name}.restart_policy");
            if *restart_seen {
                self.invalid_model(
                    &subject,
                    filename,
                    "systemd Restart is declared more than once",
                    Some(origin),
                );
                continue;
            }
            *restart_seen = true;
            self.map_systemd_restart(filename, &subject, service, entry, origin);
        }
    }

    fn map_systemd_restart(
        &mut self,
        filename: &str,
        subject: &str,
        service: &mut Service,
        entry: &TypedEntry,
        origin: Provenance,
    ) {
        let Some(value) = self.direct_value(filename, subject, entry, origin.clone()) else {
            return;
        };
        let (policy, approximation) = match value {
            "no" => (RestartPolicy::Never, None),
            "always" => (
                RestartPolicy::Always,
                Some(
                    "systemd restarts the complete generated service and has different activation and manual-stop behavior",
                ),
            ),
            "on-failure" => (
                RestartPolicy::on_failure(None),
                Some(
                    "systemd on-failure also covers selected signals, timeouts, watchdog failures, and OOM termination",
                ),
            ),
            _ => {
                self.unsupported_value(
                    subject,
                    filename,
                    entry.key().text(),
                    "Restart value has no faithful neutral container restart-policy representation",
                    origin,
                );
                return;
            }
        };
        service.set_restart_policy(Sourced::from_source(policy, origin.clone()));
        if let Some(reason) = approximation {
            self.approximate(subject, filename, reason, origin);
        } else {
            self.exact(subject, Some(origin));
        }
    }

    fn map_image(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let origin = self.entry_origin(entry);
        let subject = format!("services.{service_name}.image");
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if matches!(
            entry.value_kind(),
            ValueKind::UnitReference(UnitReferenceKind::Image | UnitReferenceKind::Build)
        ) {
            self.unsupported(&subject, filename, entry.key().text(), origin);
            return;
        }

        match ImageReference::parse(value) {
            Ok(image) => {
                service.set_image(Sourced::from_source(image, origin.clone()));
                self.exact(subject, Some(origin));
            }
            Err(error) => self.invalid_model(&subject, filename, &error.to_string(), Some(origin)),
        }
    }

    fn map_command(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.command");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let arguments: Vec<_> = value.split_ascii_whitespace().collect();
        if arguments.is_empty() || !arguments.iter().all(|argument| is_safe_word(argument, false)) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Exec requires systemd command-line decoding outside the exact unquoted-word subset",
                origin,
            );
            return;
        }

        service.set_command(Sourced::from_source(
            Command::Exec(arguments.into_iter().map(ProtectedString::plain).collect()),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_environment(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let origin = self.entry_origin(entry);
        let fallback_subject = format!("services.{service_name}.environment");
        let Some(value) = self.direct_value(filename, &fallback_subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, value)) = value.split_once('=') else {
            self.unsupported_value(
                &fallback_subject,
                filename,
                entry.key().text(),
                "Environment must contain one explicit NAME=VALUE assignment",
                origin,
            );
            return;
        };
        let subject = format!("{fallback_subject}.{name}");
        if !is_environment_name(name) || !is_safe_word(value, true) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Environment requires systemd assignment decoding outside the exact single-assignment subset",
                origin,
            );
            return;
        }
        let Some(name) = self.identifier(name, &subject, origin.clone()) else {
            return;
        };

        service.add_environment(Sourced::from_source(
            EnvironmentVariable::new(name, EnvironmentValue::Literal(ProtectedString::sensitive(value))),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_dns_value(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entry: &TypedEntry,
        field: &str,
    ) {
        let subject = format!("services.{service_name}.{field}");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let (mut values, mut origins) = match field {
            "dns" => (
                service.dns_servers().map_or_else(Vec::new, ToOwned::to_owned),
                service.dns_servers_origins().to_vec(),
            ),
            "dns_opt" => (
                service.dns_options().map_or_else(Vec::new, ToOwned::to_owned),
                service.dns_options_origins().to_vec(),
            ),
            _ => (
                service.dns_search_domains().map_or_else(Vec::new, ToOwned::to_owned),
                service.dns_search_domains_origins().to_vec(),
            ),
        };
        if value.is_empty() {
            values.clear();
            origins = vec![origin.clone()];
            match field {
                "dns" => service.set_dns_servers_with_origins(values, origins),
                "dns_opt" => service.set_dns_options_with_origins(values, origins),
                _ => service.set_dns_search_domains_with_origins(values, origins),
            }
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "an empty native DNS assignment resets the effective list and is not a literal DNS value",
                origin,
            );
            return;
        }
        let special = (field == "dns" && value == "none") || (field == "dns_search" && value == ".");
        let unsafe_value = value.contains(['\0', '\r', '\n', '%', '$']);
        values.push(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        if !origins.contains(&origin) {
            origins.push(origin.clone());
        }
        match field {
            "dns" => service.set_dns_servers_with_origins(values, origins),
            "dns_opt" => service.set_dns_options_with_origins(values, origins),
            _ => service.set_dns_search_domains_with_origins(values, origins),
        }
        if unsafe_value {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "DNS value requires unsafe systemd/deferred interpretation",
                origin,
            );
        } else if special {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "special DNS values have target-specific resolver semantics",
                origin,
            );
        } else {
            self.exact(subject, Some(origin));
        }
    }

    fn map_environment_file(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let index = service.environment_files().len();
        let subject = format!("services.{service_name}.env_file[{index}]");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !is_safe_absolute_environment_file_path(value) {
            self.unsupported_value(
                &subject,
                filename,
                "EnvironmentFile",
                "only absolute literal environment-file paths can enter the neutral model without unit-directory or systemd-specifier context",
                origin,
            );
            return;
        }
        let declaration = match EnvironmentFile::new(ProtectedString::sensitive(value), EnvironmentFileSyntax::Short) {
            Ok(declaration) => declaration,
            Err(error) => {
                self.invalid_model(&subject, filename, &error.to_string(), Some(origin));
                return;
            }
        };
        service.add_environment_file(Sourced::from_source(declaration, origin.clone()));
        self.approximate(
            &subject,
            filename,
            "Quadlet and Compose environment-file parsers are not yet proven equivalent; the declaration is retained without reading the file",
            origin,
        );
    }

    fn map_port(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let index = service.ports().len();
        let subject = format!("services.{service_name}.ports[{index}]");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        match decode_port(value) {
            Ok(port) => {
                service.add_port(Sourced::from_source(port, origin.clone()));
                self.exact(subject, Some(origin));
            }
            Err(ValueIssue::Unsupported(reason)) => {
                self.unsupported_value(&subject, filename, entry.key().text(), reason, origin);
            }
            Err(ValueIssue::Invalid(reason)) => self.invalid_model(&subject, filename, reason, Some(origin)),
        }
    }

    fn map_mount(
        &mut self,
        filename: &str,
        service_name: &str,
        application: &mut Application,
        service: &mut Service,
        entry: &TypedEntry,
    ) {
        let index = service.mounts().len();
        let subject = format!("services.{service_name}.mounts[{index}]");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let decoded = decode_mount(value, entry.value_kind());
        let (mount, external_volume) = match decoded {
            Ok(decoded) => decoded,
            Err(ValueIssue::Unsupported(reason)) => {
                self.unsupported_value(&subject, filename, entry.key().text(), reason, origin);
                return;
            }
            Err(ValueIssue::Invalid(reason)) => {
                self.invalid_model(&subject, filename, reason, Some(origin));
                return;
            }
        };

        if let Some(name) = external_volume {
            if !self.ensure_external_volume(application, &name, filename, origin.clone()) {
                return;
            }
        }
        service.add_mount(Sourced::from_source(mount, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_network(
        &mut self,
        filename: &str,
        service_name: &str,
        application: &mut Application,
        service: &mut Service,
        entry: &TypedEntry,
    ) {
        let origin = self.entry_origin(entry);
        let raw_subject = format!("services.{service_name}.networks");
        let Some(value) = self.direct_value(filename, &raw_subject, entry, origin.clone()) else {
            return;
        };
        let (name, external) = match entry.value_kind() {
            ValueKind::UnitReference(UnitReferenceKind::Network) => {
                let Some(name) = value.strip_suffix(".network") else {
                    self.invalid_model(&raw_subject, filename, "invalid .network unit reference", Some(origin));
                    return;
                };
                (name, false)
            }
            _ if is_external_network_name(value) => (value, true),
            _ => {
                self.unsupported_value(
                    &raw_subject,
                    filename,
                    entry.key().text(),
                    "Network mode, options, aliases, or target-specific arguments are outside the named-network subset",
                    origin,
                );
                return;
            }
        };
        let subject = format!("{raw_subject}.{name}");
        let Some(identifier) = self.identifier(name, &subject, origin.clone()) else {
            return;
        };
        if external && !self.ensure_external_network(application, &identifier, filename, origin.clone()) {
            return;
        }
        service.add_network(Sourced::from_source(
            NetworkAttachment::new(identifier, Vec::new()),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_label(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let fallback_subject = format!("services.{service_name}.labels");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &fallback_subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, value)) = value.split_once('=') else {
            self.unsupported_value(
                &fallback_subject,
                filename,
                entry.key().text(),
                "Label must contain one explicit NAME=VALUE assignment",
                origin,
            );
            return;
        };
        let subject = format!("{fallback_subject}.{name}");
        if !is_label_name(name) || !is_safe_word(value, true) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Label requires systemd assignment decoding outside the exact single-assignment subset",
                origin,
            );
            return;
        }
        let Some(name) = self.identifier(name, &subject, origin.clone()) else {
            return;
        };
        service.add_label(Sourced::from_source(
            MetadataLabel::new(name, ProtectedString::plain(value)),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_secret_grant(
        &mut self,
        filename: &str,
        service_name: &str,
        application: &mut Application,
        service: &mut Service,
        entry: &TypedEntry,
    ) {
        let index = service.secret_grants().len();
        let subject = format!("services.{service_name}.secret_grants[{index}]");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let decoded = match decode_secret_grant(value) {
            Ok(decoded) => decoded,
            Err(ValueIssue::Unsupported(reason)) => {
                self.unsupported_value(&subject, filename, entry.key().text(), reason, origin);
                return;
            }
            Err(ValueIssue::Invalid(reason)) => {
                self.invalid_model(&subject, filename, reason, Some(origin));
                return;
            }
        };
        if !self.ensure_external_secret(application, &decoded.source, filename, origin.clone()) {
            return;
        }

        let syntax = if decoded.has_options {
            ResourceGrantSyntax::Long
        } else {
            ResourceGrantSyntax::Short
        };
        let Ok(mut grant) = ResourceGrant::new(ProtectedString::plain(decoded.source.as_str()), syntax) else {
            self.invalid_model(
                &subject,
                filename,
                "secret grant source cannot enter the neutral model",
                Some(origin),
            );
            return;
        };
        if let Some(target) = decoded.target {
            grant.set_target(Sourced::from_source(ProtectedString::plain(target), origin.clone()));
        }
        if let Some(uid) = decoded.uid {
            grant.set_uid(Sourced::from_source(ProtectedString::plain(uid), origin.clone()));
        }
        if let Some(gid) = decoded.gid {
            grant.set_gid(Sourced::from_source(ProtectedString::plain(gid), origin.clone()));
        }
        if let Some(mode) = decoded.mode {
            grant.set_mode(Sourced::from_source(ProtectedString::plain(mode), origin.clone()));
        }
        service.add_secret_grant(Sourced::from_source(grant, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_host_mapping(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let fallback_subject = format!("services.{service_name}.host_mappings");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &fallback_subject, entry, origin.clone()) else {
            return;
        };
        let mappings = match decode_host_mappings(value) {
            Ok(mappings) => mappings,
            Err(ValueIssue::Unsupported(reason)) => {
                self.unsupported_value(&fallback_subject, filename, entry.key().text(), reason, origin);
                return;
            }
            Err(ValueIssue::Invalid(reason)) => {
                self.invalid_model(&fallback_subject, filename, reason, Some(origin));
                return;
            }
        };
        for mapping in mappings {
            let subject = format!(
                "services.{service_name}.host_mappings[{}]",
                service.host_mappings().len()
            );
            service.add_host_mapping(Sourced::from_source(mapping, origin.clone()));
            self.exact(subject, Some(origin.clone()));
        }
    }

    fn map_user(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.user");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !is_native_atom(value) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "User requires native quoting or target-specific interpretation",
                origin,
            );
            return;
        }
        service.set_user(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_group(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.group");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if service.user().is_none() {
            self.invalid_model(
                &subject,
                filename,
                "Quadlet Group requires a valid User value",
                Some(origin),
            );
            return;
        }
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Quadlet Group requires a numeric GID in the supported native contract",
                origin,
            );
            return;
        }
        service.set_group(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_supplementary_group(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entry: &TypedEntry,
    ) {
        let subject = format!(
            "services.{service_name}.supplementary_groups[{}]",
            service.supplementary_groups().len()
        );
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !is_native_atom(value) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "GroupAdd requires native quoting or target-specific interpretation",
                origin,
            );
            return;
        }
        service.add_supplementary_group(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_user_namespace(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.user_namespace");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !is_safe_word(value, false) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "UserNS requires native quoting or target-specific interpretation",
                origin,
            );
            return;
        }
        service.set_user_namespace(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_working_directory(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.working_directory");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !value.starts_with('/') || value.contains('%') || !is_safe_mount_part(value) {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "WorkingDir must be an unquoted absolute container path in the exact subset",
                origin,
            );
            return;
        }
        service.set_working_directory(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_read_only(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.read_only_root_filesystem");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let value = match value {
            "true" => true,
            "false" => false,
            _ => {
                self.invalid_model(&subject, filename, "ReadOnly must be true or false", Some(origin));
                return;
            }
        };
        service.set_read_only_root_filesystem(Sourced::from_source(value, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_health_command(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        state: &mut ContainerImportState<'_>,
    ) {
        let subject = format!("services.{service_name}.healthcheck.test");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if value == "none" {
            state
                .healthcheck
                .set_disabled(Sourced::from_source(true, origin.clone()));
            state.health_origins.push(origin.clone());
            self.exact(subject, Some(origin));
            return;
        }

        let command = match decode_health_command(value) {
            Ok(command) => command,
            Err(ValueIssue::Unsupported(reason)) => {
                self.unsupported_value(&subject, filename, entry.key().text(), reason, origin);
                return;
            }
            Err(ValueIssue::Invalid(reason)) => {
                self.invalid_model(&subject, filename, reason, Some(origin));
                return;
            }
        };
        state
            .healthcheck
            .set_command(Sourced::from_source(command, origin.clone()));
        state.health_origins.push(origin.clone());
        self.exact(subject, Some(origin));
    }

    fn map_health_duration(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        state: &mut ContainerImportState<'_>,
        field: HealthDurationField,
    ) {
        let subject = format!("services.{service_name}.healthcheck.{}", field.name());
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if value == "disable" {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "a disabled automatic health timer is distinct from disabling the health check and has no neutral field",
                origin,
            );
            return;
        }
        if !is_podman_health_duration(value) {
            self.invalid_model(
                &subject,
                filename,
                "health duration is not a supported Podman duration spelling",
                Some(origin),
            );
            return;
        }
        let Ok(duration) = HealthcheckDuration::new(value) else {
            self.invalid_model(
                &subject,
                filename,
                "health duration cannot enter the neutral model",
                Some(origin),
            );
            return;
        };
        let duration = Sourced::from_source(duration, origin.clone());
        match field {
            HealthDurationField::Interval => state.healthcheck.set_interval(duration),
            HealthDurationField::Timeout => state.healthcheck.set_timeout(duration),
            HealthDurationField::StartPeriod => state.healthcheck.set_start_period(duration),
        }
        state.health_origins.push(origin.clone());
        self.exact(subject, Some(origin));
    }

    fn map_health_retries(
        &mut self,
        filename: &str,
        service_name: &str,
        entry: &TypedEntry,
        state: &mut ContainerImportState<'_>,
    ) {
        let subject = format!("services.{service_name}.healthcheck.retries");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Ok(retries) = HealthcheckRetries::new(value) else {
            self.invalid_model(
                &subject,
                filename,
                "HealthRetries must be a non-negative decimal integer",
                Some(origin),
            );
            return;
        };
        state
            .healthcheck
            .set_retries(Sourced::from_source(retries, origin.clone()));
        state.health_origins.push(origin.clone());
        self.exact(subject, Some(origin));
    }

    fn ensure_external_volume(
        &mut self,
        application: &mut Application,
        name: &Identifier,
        filename: &str,
        origin: Provenance,
    ) -> bool {
        if let Some(existing) = application
            .volumes()
            .iter()
            .find(|volume| volume.value().name() == name)
        {
            if existing.value().ownership() == ResourceOwnership::External {
                return true;
            }
            self.invalid_model(
                &format!("volumes.{}", name.as_str()),
                filename,
                "literal external volume conflicts with an application-owned .volume unit",
                Some(origin),
            );
            return false;
        }
        let subject = format!("volumes.{}", name.as_str());
        if let Err(error) = application.add_volume(Sourced::from_source(
            Volume::new(name.clone(), ResourceOwnership::External),
            origin.clone(),
        )) {
            self.invalid_model(&subject, filename, &error.to_string(), Some(origin));
            return false;
        }
        self.exact(subject, Some(origin));
        true
    }

    fn ensure_external_network(
        &mut self,
        application: &mut Application,
        name: &Identifier,
        filename: &str,
        origin: Provenance,
    ) -> bool {
        if let Some(existing) = application
            .networks()
            .iter()
            .find(|network| network.value().name() == name)
        {
            if existing.value().ownership() == ResourceOwnership::External {
                return true;
            }
            self.invalid_model(
                &format!("networks.{}", name.as_str()),
                filename,
                "literal external network conflicts with an application-owned .network unit",
                Some(origin),
            );
            return false;
        }
        let subject = format!("networks.{}", name.as_str());
        if let Err(error) = application.add_network(Sourced::from_source(
            Network::new(name.clone(), ResourceOwnership::External),
            origin.clone(),
        )) {
            self.invalid_model(&subject, filename, &error.to_string(), Some(origin));
            return false;
        }
        self.exact(subject, Some(origin));
        true
    }

    fn ensure_external_secret(
        &mut self,
        application: &mut Application,
        name: &Identifier,
        filename: &str,
        origin: Provenance,
    ) -> bool {
        if let Some(existing) = application
            .secrets()
            .iter()
            .find(|secret| secret.value().name() == name)
        {
            if existing.value().ownership() == ResourceOwnership::External && existing.value().material().is_none() {
                return true;
            }
            self.invalid_model(
                &format!("secrets.{}", name.as_str()),
                filename,
                "Podman secret reference conflicts with an application-owned or materialized secret",
                Some(origin),
            );
            return false;
        }
        let subject = format!("secrets.{}", name.as_str());
        if let Err(error) = application.add_secret(Sourced::from_source(
            Secret::new(name.clone(), ResourceOwnership::External),
            origin.clone(),
        )) {
            self.invalid_model(&subject, filename, &error.to_string(), Some(origin));
            return false;
        }
        self.exact(subject, Some(origin));
        true
    }

    fn direct_value<'entry>(
        &mut self,
        filename: &str,
        subject: &str,
        entry: &'entry TypedEntry,
        origin: Provenance,
    ) -> Option<&'entry str> {
        if entry.value().is_continued() {
            self.unsupported_value(
                subject,
                filename,
                entry.key().text(),
                "continued physical values require native semantic decoding before import",
                origin,
            );
            return None;
        }
        Some(entry.value().primary().text().trim())
    }

    fn report_unmapped_entries<'entry>(&mut self, filename: &str, entries: impl Iterator<Item = &'entry TypedEntry>) {
        for entry in entries {
            self.unsupported(
                &format!("quadlet.{filename}.{}", entry.key().text()),
                filename,
                entry.key().text(),
                self.entry_origin(entry),
            );
        }
    }

    fn unsupported_entry(&mut self, filename: &str, service_name: &str, entry: &TypedEntry) {
        self.unsupported(
            &format!("services.{service_name}.quadlet.{}", entry.key().text()),
            filename,
            entry.key().text(),
            self.entry_origin(entry),
        );
    }

    fn identifier(&mut self, value: &str, subject: &str, origin: Provenance) -> Option<Identifier> {
        match Identifier::new(value) {
            Ok(identifier) => Some(identifier),
            Err(error) => {
                self.invalid_model(subject, value, &error.to_string(), Some(origin));
                None
            }
        }
    }

    fn invalid_source(&mut self) {
        let subject = "quadlet.document_set";
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.codes.invalid_source.clone())
        {
            self.outcomes.push(outcome);
        }
        self.diagnostics.push(
            Diagnostic::new(
                self.codes.invalid_source.clone(),
                Severity::Error,
                "Quadlet document set contains unresolved, ambiguous, or duplicate unit references",
            )
            .with_field(DiagnosticField::new(
                "diagnostic_count",
                DiagnosticValue::plain(self.source.documents().diagnostics().len().to_string()),
            )),
        );
    }

    fn invalid_model(&mut self, subject: &str, document: &str, reason: &str, origin: Option<Provenance>) {
        if let Ok(mut outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Invalid, self.codes.invalid_model.clone())
        {
            if let Some(origin) = origin {
                outcome = outcome.with_origin(origin);
            }
            self.outcomes.push(outcome);
        }
        self.diagnostics.push(
            Diagnostic::new(
                self.codes.invalid_model.clone(),
                Severity::Error,
                "Quadlet value cannot enter the neutral application model",
            )
            .with_field(DiagnosticField::new("document", DiagnosticValue::plain(document)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason))),
        );
    }

    fn unsupported(&mut self, subject: &str, document: &str, key: &str, origin: Provenance) {
        self.unsupported_with_reason(subject, document, key, None, origin);
    }

    fn unsupported_value(&mut self, subject: &str, document: &str, key: &str, reason: &str, origin: Provenance) {
        self.unsupported_with_reason(subject, document, key, Some(reason), origin);
    }

    fn unsupported_with_reason(
        &mut self,
        subject: &str,
        document: &str,
        key: &str,
        reason: Option<&str>,
        origin: Provenance,
    ) {
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Unsupported, self.codes.unsupported.clone())
        {
            self.outcomes.push(outcome.with_origin(origin));
        }
        let mut diagnostic = Diagnostic::new(
            self.codes.unsupported.clone(),
            Severity::Warning,
            "Quadlet intent is not represented by the current neutral-model importer",
        )
        .with_field(DiagnosticField::new("document", DiagnosticValue::plain(document)))
        .with_field(DiagnosticField::new("key", DiagnosticValue::plain(key)));
        if let Some(reason) = reason {
            diagnostic = diagnostic.with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason)));
        }
        self.diagnostics.push(diagnostic);
    }

    fn approximate(&mut self, subject: &str, document: &str, reason: &str, origin: Provenance) {
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Approximate, self.codes.approximate.clone())
        {
            self.outcomes.push(outcome.with_origin(origin));
        }
        self.diagnostics.push(
            Diagnostic::new(
                self.codes.approximate.clone(),
                Severity::Warning,
                "Quadlet intent requires a documented neutral-model approximation",
            )
            .with_field(DiagnosticField::new("document", DiagnosticValue::plain(document)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(reason))),
        );
    }

    fn exact(&mut self, subject: impl Into<String>, origin: Option<Provenance>) {
        let mut outcome = ConversionOutcome::exact(subject);
        if let Some(origin) = origin {
            outcome = outcome.with_origin(origin);
        }
        self.outcomes.push(outcome);
    }

    fn exact_origins(&mut self, subject: impl Into<String>, origins: Vec<Provenance>) {
        let mut outcome = ConversionOutcome::exact(subject);
        for origin in origins {
            outcome = outcome.with_origin(origin);
        }
        self.outcomes.push(outcome);
    }

    fn entry_origin(&self, entry: &TypedEntry) -> Provenance {
        self.provenance(entry.value().primary().span())
    }

    fn document_origin(&self, span: quadlet_lens::source::SourceSpan) -> Provenance {
        self.provenance(span)
    }

    fn provenance(&self, span: quadlet_lens::source::SourceSpan) -> Provenance {
        let source_id = self.source.source_id(span.source_id()).clone();
        match SourceSpan::new(span.start(), span.end()) {
            Ok(neutral_span) => Provenance::spanned(source_id, neutral_span),
            Err(_) => Provenance::source(source_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueIssue {
    Unsupported(&'static str),
    Invalid(&'static str),
}

#[derive(Clone, Copy)]
enum HealthDurationField {
    Interval,
    Timeout,
    StartPeriod,
}

struct DecodedSecretGrant {
    source: Identifier,
    target: Option<String>,
    uid: Option<String>,
    gid: Option<String>,
    mode: Option<String>,
    has_options: bool,
}

impl HealthDurationField {
    const fn name(self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::Timeout => "timeout",
            Self::StartPeriod => "start_period",
        }
    }
}

fn unit_stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(stem, _)| stem)
}

fn sibling_service_reference<'value>(value: &'value str, service_names: &BTreeSet<String>) -> Option<&'value str> {
    if !is_safe_systemd_unit_name(value) {
        return None;
    }
    let service = value
        .strip_suffix(".container")
        .or_else(|| value.strip_suffix(".service"))?;
    service_names.contains(service).then_some(service)
}

fn sourced_with_origins<T>(value: T, origins: &[Provenance]) -> Sourced<T> {
    let mut sourced = Sourced::from_source(value, origins[0].clone());
    for origin in origins.iter().skip(1) {
        sourced.add_origin(origin.clone());
    }
    sourced
}

const fn singleton_field(key: ContainerKey) -> Option<&'static str> {
    match key {
        ContainerKey::Image => Some("image"),
        ContainerKey::ContainerName => Some("runtime_name"),
        ContainerKey::Exec => Some("command"),
        ContainerKey::User => Some("user"),
        ContainerKey::Group => Some("group"),
        ContainerKey::UserNS => Some("user_namespace"),
        ContainerKey::WorkingDir => Some("working_directory"),
        ContainerKey::ReadOnly => Some("read_only_root_filesystem"),
        ContainerKey::HealthCmd => Some("healthcheck.test"),
        ContainerKey::HealthInterval => Some("healthcheck.interval"),
        ContainerKey::HealthTimeout => Some("healthcheck.timeout"),
        ContainerKey::HealthRetries => Some("healthcheck.retries"),
        ContainerKey::HealthStartPeriod => Some("healthcheck.start_period"),
        ContainerKey::Pod => Some("service_group"),
        ContainerKey::AppArmor => Some("security_options.apparmor"),
        ContainerKey::NoNewPrivileges => Some("security_options.no_new_privileges"),
        ContainerKey::SeccompProfile => Some("security_options.seccomp_profile"),
        ContainerKey::SecurityLabelDisable => Some("security_options.security_label_disable"),
        ContainerKey::SecurityLabelFileType => Some("security_options.security_label_file_type"),
        ContainerKey::SecurityLabelLevel => Some("security_options.security_label_level"),
        ContainerKey::SecurityLabelNested => Some("security_options.security_label_nested"),
        ContainerKey::SecurityLabelType => Some("security_options.security_label_type"),
        _ => None,
    }
}

const fn is_security_container_key(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::Container(
            ContainerKey::AppArmor
                | ContainerKey::NoNewPrivileges
                | ContainerKey::SeccompProfile
                | ContainerKey::SecurityLabelDisable
                | ContainerKey::SecurityLabelFileType
                | ContainerKey::SecurityLabelLevel
                | ContainerKey::SecurityLabelNested
                | ContainerKey::SecurityLabelType
                | ContainerKey::Mask
                | ContainerKey::Unmask
        )
    )
}

fn decode_health_command(value: &str) -> Result<HealthcheckCommand, ValueIssue> {
    if !value.starts_with('[') {
        if !is_safe_health_shell_command(value) {
            return Err(ValueIssue::Unsupported(
                "HealthCmd requires JSON-array decoding or shell syntax outside the conservative plain-command subset",
            ));
        }
        return Ok(HealthcheckCommand::Shell(ProtectedString::sensitive(value)));
    }

    let values: Vec<String> = serde_json::from_str(value)
        .map_err(|_| ValueIssue::Unsupported("HealthCmd JSON array requires native command decoding"))?;
    if values
        .iter()
        .any(|value| value.contains('\0') || value.contains('%') || value.contains(['\n', '\r']))
    {
        return Err(ValueIssue::Unsupported(
            "HealthCmd contains a control byte or unresolved systemd percent specifier",
        ));
    }
    match values.as_slice() {
        [kind, arguments @ ..] if kind == "CMD" && !arguments.is_empty() => Ok(HealthcheckCommand::Exec(
            arguments.iter().map(ProtectedString::sensitive).collect(),
        )),
        [kind, command] if kind == "CMD-SHELL" && !command.is_empty() => {
            Ok(HealthcheckCommand::Shell(ProtectedString::sensitive(command)))
        }
        [kind, ..] if kind == "CMD" => Err(ValueIssue::Invalid(
            "CMD health check requires at least one command argument",
        )),
        [kind, ..] if kind == "CMD-SHELL" => Err(ValueIssue::Unsupported(
            "CMD-SHELL health check must contain exactly one non-empty shell string",
        )),
        _ => Err(ValueIssue::Unsupported(
            "HealthCmd JSON array uses an unknown health-check command mode",
        )),
    }
}

fn decode_secret_grant(value: &str) -> Result<DecodedSecretGrant, ValueIssue> {
    let mut parts = value.split(',');
    let source = parts.next().unwrap_or_default();
    if !is_safe_secret_component(source) {
        return Err(ValueIssue::Unsupported(
            "secret name requires native quoting or target-specific interpretation",
        ));
    }
    let source = Identifier::new(source)
        .map_err(|_| ValueIssue::Invalid("secret name cannot enter the neutral identifier model"))?;
    let mut target = None;
    let mut uid = None;
    let mut gid = None;
    let mut mode = None;
    let mut secret_type = None;
    let mut seen = BTreeSet::new();
    let mut has_options = false;

    for option in parts {
        has_options = true;
        let Some((name, value)) = option.split_once('=') else {
            return Err(ValueIssue::Invalid("Secret option must use name=value syntax"));
        };
        if !seen.insert(name) {
            return Err(ValueIssue::Invalid("Secret option is declared more than once"));
        }
        match name {
            "type" if matches!(value, "mount" | "env") => secret_type = Some(value),
            "type" => return Err(ValueIssue::Invalid("Secret type must be mount or env")),
            "target" if is_safe_secret_component(value) => target = Some(value.to_owned()),
            "target" => {
                return Err(ValueIssue::Unsupported(
                    "secret target requires native quoting or target-specific interpretation",
                ));
            }
            "uid" if is_decimal(value) => uid = Some(value.to_owned()),
            "gid" if is_decimal(value) => gid = Some(value.to_owned()),
            "uid" | "gid" => {
                return Err(ValueIssue::Invalid(
                    "secret UID and GID options require non-negative decimal integers",
                ));
            }
            "mode" if is_podman_secret_mode(value) => mode = Some(value.to_owned()),
            "mode" => {
                return Err(ValueIssue::Invalid(
                    "secret mode must be a one-to-four-digit octal value no greater than 0777",
                ));
            }
            _ => {
                return Err(ValueIssue::Unsupported(
                    "Secret option is outside the reviewed Podman mount-secret grammar",
                ));
            }
        }
    }
    if secret_type == Some("env") {
        return Err(ValueIssue::Unsupported(
            "environment-exposed Podman secrets require a neutral grant exposure field",
        ));
    }

    Ok(DecodedSecretGrant {
        source,
        target,
        uid,
        gid,
        mode,
        has_options,
    })
}

fn decode_host_mappings(value: &str) -> Result<Vec<HostMapping>, ValueIssue> {
    let (hostnames, address) = if value.ends_with(']') {
        let Some(index) = value.rfind(":[") else {
            return Err(ValueIssue::Unsupported(
                "AddHost bracketed address is outside the exact hostname:address subset",
            ));
        };
        (&value[..index], &value[index + 1..])
    } else {
        value
            .rsplit_once(':')
            .ok_or(ValueIssue::Invalid("AddHost must contain hostname:address"))?
    };
    if hostnames.is_empty() {
        return Err(ValueIssue::Invalid("AddHost must contain at least one hostname"));
    }
    let address = HostAddress::new(address).map_err(|_| ValueIssue::Invalid("AddHost address is empty or invalid"))?;
    if !matches!(
        address.kind(),
        HostAddressKind::Ipv4 | HostAddressKind::Ipv6 { .. } | HostAddressKind::HostGateway
    ) {
        return Err(ValueIssue::Unsupported(
            "AddHost address is deferred or implementation-specific",
        ));
    }
    hostnames
        .split(';')
        .map(|hostname| {
            if !is_native_atom(hostname) {
                return Err(ValueIssue::Unsupported(
                    "AddHost hostname requires native quoting or target-specific interpretation",
                ));
            }
            let hostname = Identifier::new(hostname)
                .map_err(|_| ValueIssue::Invalid("AddHost hostname cannot enter the neutral identifier model"))?;
            Ok(HostMapping::new(hostname, address.clone()))
        })
        .collect()
}

fn decode_port(value: &str) -> Result<Port, ValueIssue> {
    if value.is_empty() {
        return Err(ValueIssue::Invalid("PublishPort value is empty"));
    }
    if value.contains('-') {
        return Err(ValueIssue::Unsupported(
            "port ranges are not represented by the neutral port model",
        ));
    }
    let (addressing, protocol) = value
        .rsplit_once('/')
        .map_or((value, "tcp"), |(ports, protocol)| (ports, protocol));
    let protocol = match protocol {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        "sctp" => Protocol::Sctp,
        _ => return Err(ValueIssue::Invalid("PublishPort protocol must be tcp, udp, or sctp")),
    };
    let parts: Vec<_> = addressing.split(':').collect();
    let (host_address, published, container) = match parts.as_slice() {
        [container] => (None, None, *container),
        [published, container] => (None, optional_port(published)?, *container),
        [address, published, container] if address.is_empty() || is_ipv4_address(address) => (
            (!address.is_empty()).then(|| (*address).to_owned()),
            optional_port(published)?,
            *container,
        ),
        _ => {
            return Err(ValueIssue::Unsupported(
                "IPv6 host addresses and target-specific PublishPort forms are outside the exact scalar subset",
            ));
        }
    };
    let container = required_port(container)?;
    Port::new(container, published, host_address, protocol)
        .map_err(|_| ValueIssue::Invalid("PublishPort container port must be non-zero"))
}

fn optional_port(value: &str) -> Result<Option<u16>, ValueIssue> {
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| ValueIssue::Invalid("PublishPort host port is not an unsigned 16-bit integer"))
    }
}

fn required_port(value: &str) -> Result<u16, ValueIssue> {
    value
        .parse()
        .map_err(|_| ValueIssue::Invalid("PublishPort container port is not an unsigned 16-bit integer"))
}

fn decode_mount(value: &str, value_kind: ValueKind) -> Result<(Mount, Option<Identifier>), ValueIssue> {
    if value.is_empty() {
        return Err(ValueIssue::Invalid("Volume value is empty"));
    }
    let parts: Vec<_> = value.split(':').collect();
    let (source, target, options) = match parts.as_slice() {
        [target] => (None, *target, None),
        [source, target] => (Some(*source), *target, None),
        [source, target, options] => (Some(*source), *target, Some(*options)),
        _ => {
            return Err(ValueIssue::Unsupported(
                "Volume values containing additional colon-delimited fields are outside the exact compact subset",
            ));
        }
    };
    if !target.starts_with('/') || !is_safe_mount_part(target) {
        return Err(ValueIssue::Unsupported(
            "container mount target must be an unquoted absolute path in the exact compact subset",
        ));
    }

    let mut read_only = false;
    let mut writable = false;
    let mut relabel = None;
    if let Some(options) = options {
        for option in options.split(',') {
            match option {
                "ro" if !writable => read_only = true,
                "rw" if !read_only => writable = true,
                "z" if relabel.is_none() => relabel = Some(SelinuxRelabel::Shared),
                "Z" if relabel.is_none() => relabel = Some(SelinuxRelabel::Private),
                "ro" | "rw" => {
                    return Err(ValueIssue::Invalid(
                        "Volume options contain conflicting ro and rw intent",
                    ));
                }
                "z" | "Z" => {
                    return Err(ValueIssue::Invalid(
                        "Volume options contain conflicting or repeated SELinux relabel intent",
                    ));
                }
                _ => {
                    return Err(ValueIssue::Unsupported(
                        "Volume option is not represented by the neutral mount model",
                    ));
                }
            }
        }
    }

    let (mount_source, external_volume) = match source {
        None | Some("") => (MountSource::Anonymous, None),
        Some(source) if value_kind == ValueKind::UnitReference(UnitReferenceKind::Volume) => {
            let name = source
                .strip_suffix(".volume")
                .ok_or(ValueIssue::Invalid("invalid .volume unit reference"))?;
            let identifier = Identifier::new(name)
                .map_err(|_| ValueIssue::Invalid(".volume unit reference cannot enter the neutral identifier model"))?;
            (MountSource::Volume(identifier), None)
        }
        Some(source) if source.starts_with('/') && is_safe_mount_part(source) => {
            (MountSource::HostPath(source.to_owned()), None)
        }
        Some(source) if source.starts_with('.') || source.contains('%') => {
            return Err(ValueIssue::Unsupported(
                "unit-relative and systemd-specifier bind sources require explicit path resolution",
            ));
        }
        Some(source) if is_native_atom(source) => {
            let identifier = Identifier::new(source)
                .map_err(|_| ValueIssue::Invalid("named volume cannot enter the neutral identifier model"))?;
            (MountSource::Volume(identifier.clone()), Some(identifier))
        }
        Some(_) => {
            return Err(ValueIssue::Unsupported(
                "Volume source requires native quoting or target-specific path interpretation",
            ));
        }
    };
    let mut mount = Mount::new(mount_source, target, read_only)
        .map_err(|_| ValueIssue::Invalid("Volume target cannot enter the neutral mount model"))?;
    if let Some(relabel) = relabel {
        mount.set_selinux_relabel(relabel);
    }
    Ok((mount, external_volume))
}

fn is_safe_absolute_environment_file_path(value: &str) -> bool {
    value.starts_with('/')
        && value.trim() == value
        && !value.contains('%')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+'))
}

fn is_safe_word(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',')
        })
}

fn is_safe_security_option_value(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',' | b'*'
                )
        })
}

fn parse_security_option_boolean(value: &str) -> Option<bool> {
    match value {
        "1" | "yes" | "y" | "true" | "t" | "on" => Some(true),
        "0" | "no" | "n" | "false" | "f" | "off" => Some(false),
        _ => None,
    }
}

fn security_option_is_selinux_label(option: &SecurityOption) -> bool {
    matches!(
        option,
        SecurityOption::SecurityLabelFileType(_)
            | SecurityOption::SecurityLabelLevel(_)
            | SecurityOption::SecurityLabelNested(_)
            | SecurityOption::SecurityLabelType(_)
    )
}

fn is_safe_health_shell_command(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte == b' '
                || byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b',')
        })
}

fn is_podman_health_duration(mut value: &str) -> bool {
    if value == "0" {
        return true;
    }
    let mut found = false;
    while !value.is_empty() {
        let number_end = value
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
            .map(|(index, character)| index + character.len_utf8())
            .last()
            .unwrap_or(0);
        if number_end == 0 {
            return false;
        }
        let number = &value[..number_end];
        if number.matches('.').count() > 1 || number == "." {
            return false;
        }
        value = &value[number_end..];
        let Some(unit) = ["ns", "us", "µs", "μs", "ms", "s", "m", "h"]
            .into_iter()
            .find(|unit| value.starts_with(unit))
        else {
            return false;
        };
        value = &value[unit.len()..];
        found = true;
    }
    found
}

fn is_environment_name(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_label_name(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn is_native_atom(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_safe_secret_component(value: &str) -> bool {
    is_safe_word(value, false) && !value.contains([',', '='])
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_podman_secret_mode(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4
        && value.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
        && u16::from_str_radix(value, 8).is_ok_and(|mode| mode <= 0o777)
}

fn is_safe_systemd_unit_name(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_external_network_name(value: &str) -> bool {
    is_native_atom(value) && !matches!(value, "bridge" | "host" | "none" | "private" | "slirp4netns" | "pasta")
}

fn is_safe_mount_part(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@' | b'+' | b'=' | b'%')
        })
}

fn is_ipv4_address(value: &str) -> bool {
    value.parse::<std::net::Ipv4Addr>().is_ok()
}
