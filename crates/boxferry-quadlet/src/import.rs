//! Quadlet-to-application mapping with explicit fidelity decisions.

use std::collections::{BTreeMap, BTreeSet};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, NativeFinding, RuleId, Severity,
};
use boxferry_model::{
    Annotation, Application, BuildSettingValues, BuildSyntax, Command, Device, Entrypoint, EnvironmentFile,
    EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable, ExposedPort, GroupExitPolicy, Healthcheck,
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier,
    ImageAcquisition, ImageAcquisitionSetting, ImageArtifactAssignment, ImageBuild, ImageBuildSetting, ImageReference,
    KernelParameter, Logging, LoggingOption, MetadataLabel, Mount, MountSource, Network, NetworkAttachment,
    NetworkDriverOption, NetworkIpamConfig, Port, ProtectedString, Protocol, Provenance, PullPolicy, ReloadAction,
    ResourceGrant, ResourceGrantSyntax, ResourceLimit, ResourceOwnership, RestartPolicy, Secret, SecurityOption,
    SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition, ServiceGroup, ServiceGroupRuntime,
    SourceSpan, Sourced, StartupNotification, StopTimeout, Volume, VolumeImageSource,
};
use quadlet_lens::model::{
    AuthoredContainerEnvironmentDirective, BuildKey, ContainerKey, EntryKind, ImageKey, NetworkKey, PodKey,
    QuadletDocument, QuadletUnitType, SectionKind, SystemdUnitKey, TypedEntry, TypedSection, UnitReferenceKind,
    ValueKind, VolumeKey,
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
                invalid_source: RuleId::QuadletSourceInvalid.definition().diagnostic_code()?,
                invalid_model: RuleId::QuadletModelInvalid.definition().diagnostic_code()?,
                unsupported: RuleId::QuadletInputUnsupported.definition().diagnostic_code()?,
                approximate: RuleId::QuadletInputApproximation.definition().diagnostic_code()?,
                native_syntax: RuleId::QuadletNativeSyntax.definition().diagnostic_code()?,
                native_model: RuleId::QuadletNativeModel.definition().diagnostic_code()?,
                native_document_set: RuleId::QuadletNativeDocumentSet.definition().diagnostic_code()?,
                native_failure: RuleId::QuadletNativeFailure.definition().diagnostic_code()?,
            },
        })
    }
}

impl ImportAdapter for QuadletImporter {
    type Source = QuadletSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let mut mapping = Mapping::new(&self.codes, source);
        mapping.report_native_findings(source.native_findings());
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
                        document,
                        &service_names,
                        &mut pod_groups,
                    );
                    if let Err(error) = application.add_service(Sourced::from_source(service, document_origin)) {
                        mapping.invalid_model("services", filename, &error.to_string(), None);
                    }
                }
                QuadletUnitType::Network
                | QuadletUnitType::Volume
                | QuadletUnitType::Pod
                | QuadletUnitType::Image
                | QuadletUnitType::Build => {}
                QuadletUnitType::Kube | QuadletUnitType::Artifact => {
                    mapping.report_native_only_unit(filename, document);
                }
                _ => mapping.unsupported(
                    &format!("quadlet.{filename}"),
                    filename,
                    "unsupported unit type",
                    document_origin,
                ),
            }
        }

        mapping.finish_pod_groups(&mut application, pod_order, pod_groups);
        if let Err(error) = application.validate_image_artifact_references() {
            mapping.invalid_model("volumes", "quadlet", &error.to_string(), None);
        }

        ImportResult::new(Some(application), mapping.outcomes, mapping.diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_source: DiagnosticCode,
    invalid_model: DiagnosticCode,
    unsupported: DiagnosticCode,
    approximate: DiagnosticCode,
    native_syntax: DiagnosticCode,
    native_model: DiagnosticCode,
    native_document_set: DiagnosticCode,
    native_failure: DiagnosticCode,
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
    network_attachment_entries: Vec<&'entry TypedEntry>,
}

struct PodImportGroup {
    filename: String,
    name: Identifier,
    origin: Provenance,
    members: Vec<Sourced<Identifier>>,
    runtime: ServiceGroupRuntime,
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

    fn report_native_findings(&mut self, findings: &[NativeFinding]) {
        for finding in findings {
            let code = match finding.code().get(..3) {
                Some("QLS") => self.codes.native_syntax.clone(),
                Some("QLM") => self.codes.native_model.clone(),
                Some("QLG") => self.codes.native_document_set.clone(),
                _ => self.codes.native_failure.clone(),
            };
            self.diagnostics.push(
                Diagnostic::new(
                    code,
                    finding.severity(),
                    "QuadletLens reported a native Quadlet finding",
                )
                .with_native_finding(finding.clone()),
            );
        }
    }

    /// Records every authored Kube or Artifact entry separately. Neither unit type has a safe
    /// format-neutral meaning yet, so even generic-systemd, `[Quadlet]`, unknown, and repeated
    /// entries need a value-free outcome rather than disappearing behind the primary section.
    fn report_native_only_unit(&mut self, filename: &str, document: &QuadletDocument) {
        let unit = match document.unit_type() {
            QuadletUnitType::Kube => "kube",
            QuadletUnitType::Artifact => "artifact",
            _ => return,
        };
        for (section_index, section) in document.sections().iter().enumerate() {
            let mut occurrences = BTreeMap::<String, usize>::new();
            for entry in section.entries() {
                let key = entry.key().text();
                let occurrence = occurrences.entry(key.to_owned()).or_default();
                let subject = format!(
                    "quadlet.{unit}.{filename}.{}[{section_index}].{key}[{occurrence}]",
                    section.name().text(),
                );
                *occurrence += 1;
                self.unsupported_value(
                    &subject,
                    filename,
                    key,
                    "this native Quadlet key has no reviewed format-neutral application-model mapping",
                    self.entry_origin(entry),
                );
            }
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
                QuadletUnitType::Image => {
                    if let Some(name) = self.identifier(stem, "image_acquisitions", document_origin.clone()) {
                        let acquisition =
                            self.map_image_definition(filename, name, document.sections(), document_origin);
                        if let Err(error) = application.add_image_acquisition(acquisition) {
                            self.invalid_model("image_acquisitions", filename, &error.to_string(), None);
                        }
                    }
                }
                QuadletUnitType::Build => {
                    if let Some(name) = self.identifier(stem, "image_builds", document_origin.clone()) {
                        let build = self.map_build_definition(filename, name, document.sections(), document_origin);
                        if let Err(error) = application.add_image_build(build) {
                            self.invalid_model("image_builds", filename, &error.to_string(), None);
                        }
                    }
                }
                QuadletUnitType::Network => {
                    if let Some(name) = self.identifier(stem, "networks", document_origin.clone()) {
                        let network =
                            self.map_network_definition(filename, name, document.sections(), document_origin.clone());
                        if let Err(error) = application.add_network(Sourced::from_source(network, document_origin)) {
                            self.invalid_model("networks", filename, &error.to_string(), None);
                        }
                    }
                }
                QuadletUnitType::Volume => {
                    if let Some(name) = self.identifier(stem, "volumes", document_origin.clone()) {
                        let volume =
                            self.map_volume_definition(filename, name, document.sections(), document_origin.clone());
                        if let Err(error) = application.add_volume(Sourced::from_source(volume, document_origin)) {
                            self.invalid_model("volumes", filename, &error.to_string(), None);
                        } else {
                            self.exact(format!("volumes.{stem}"), None);
                        }
                    }
                }
                QuadletUnitType::Pod => {
                    if let Some(group) =
                        self.map_pod_definition(filename, stem, application, document.sections(), document_origin)
                    {
                        pod_order.push(stem.to_owned());
                        pod_groups.insert(stem.to_owned(), group);
                    }
                }
                _ => {}
            }
        }
        (pod_order, pod_groups)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the typed volume keys share one ordered import contract"
    )]
    fn map_volume_definition(
        &mut self,
        filename: &str,
        name: Identifier,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Volume {
        let subject = format!("volumes.{}", name.as_str());
        let mut volume = Volume::new(name, ResourceOwnership::Application);
        let mut singletons = BTreeSet::new();
        let mut labels = None;
        let mut label_origins = Vec::new();
        let mut modules = None;
        let mut module_origins = Vec::new();
        let mut global_args = None;
        let mut global_args_origins = Vec::new();
        let mut podman_args = None;
        let mut podman_args_origins = Vec::new();
        for section in sections {
            for entry in section.entries() {
                let origin = self.entry_origin(entry);
                let EntryKind::Volume(key) = entry.kind() else {
                    self.unsupported(
                        &format!("quadlet.{filename}.{}", entry.key().text()),
                        filename,
                        entry.key().text(),
                        origin,
                    );
                    continue;
                };
                if !entry.kind().is_repeatable() && !singletons.insert(key) {
                    self.invalid_model(
                        &format!("{subject}.{}", entry.key().text()),
                        filename,
                        "Quadlet volume singleton is declared more than once",
                        Some(origin),
                    );
                    continue;
                }
                let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                    continue;
                };
                let sourced = |value: &str| Sourced::from_source(ProtectedString::sensitive(value), origin.clone());
                match key {
                    VolumeKey::VolumeName => {
                        volume.set_runtime_name(sourced(value));
                        self.exact(format!("{subject}.runtime_name"), Some(origin));
                    }
                    VolumeKey::Driver => {
                        volume.set_driver(sourced(value));
                        self.exact(format!("{subject}.driver"), Some(origin));
                    }
                    VolumeKey::Device => {
                        volume.set_device(sourced(value));
                        self.exact(format!("{subject}.device"), Some(origin));
                    }
                    VolumeKey::Type => {
                        volume.set_volume_type(sourced(value));
                        self.exact(format!("{subject}.type"), Some(origin));
                    }
                    VolumeKey::Options => {
                        volume.set_options(sourced(value));
                        self.exact(format!("{subject}.options"), Some(origin));
                    }
                    VolumeKey::Copy => match value {
                        "true" => {
                            volume.set_copy(Sourced::from_source(true, origin.clone()));
                            self.exact(format!("{subject}.copy"), Some(origin));
                        }
                        "false" => {
                            volume.set_copy(Sourced::from_source(false, origin.clone()));
                            self.exact(format!("{subject}.copy"), Some(origin));
                        }
                        _ => self.invalid_model(
                            &format!("{subject}.copy"),
                            filename,
                            "Copy must be true or false",
                            Some(origin),
                        ),
                    },
                    VolumeKey::Label if value.is_empty() => {
                        labels = Some(Vec::new());
                        label_origins = vec![origin.clone()];
                        self.unsupported_value(
                            &format!("{subject}.labels"),
                            filename,
                            "Label",
                            "an empty Label assignment resets the effective list and cannot be regenerated exactly",
                            origin,
                        );
                    }
                    VolumeKey::Label => match parse_network_label(value) {
                        Some((name, value)) => {
                            if let Some(name) = self.identifier(name, &format!("{subject}.labels"), origin.clone()) {
                                let labels = labels.get_or_insert_default();
                                labels.push(Sourced::from_source(
                                    MetadataLabel::new(name, ProtectedString::sensitive(value)),
                                    origin.clone(),
                                ));
                                self.exact(format!("{subject}.labels[{}]", labels.len() - 1), Some(origin));
                            }
                        }
                        None => self.unsupported_value(
                            &format!("{subject}.labels"),
                            filename,
                            "Label",
                            "Label requires one safe NAME=VALUE assignment",
                            origin,
                        ),
                    },
                    VolumeKey::ContainersConfModule => {
                        if value.is_empty() {
                            modules = Some(Vec::new());
                            module_origins = vec![origin.clone()];
                            self.unsupported_value(
                                &format!("{subject}.containers_conf_modules"),
                                filename,
                                "ContainersConfModule",
                                "an empty assignment reset cannot be regenerated exactly",
                                origin,
                            );
                        } else {
                            let modules = modules.get_or_insert_default();
                            modules.push(sourced(value));
                            self.exact(
                                format!("{subject}.containers_conf_modules[{}]", modules.len() - 1),
                                Some(origin),
                            );
                        }
                    }
                    VolumeKey::GlobalArgs => {
                        if value.is_empty() {
                            global_args = Some(Vec::new());
                            global_args_origins = vec![origin.clone()];
                            self.unsupported_value(
                                &format!("{subject}.global_args"),
                                filename,
                                "GlobalArgs",
                                "an empty assignment reset cannot be regenerated exactly",
                                origin,
                            );
                        } else {
                            let global_args = global_args.get_or_insert_default();
                            global_args.push(sourced(value));
                            self.exact(
                                format!("{subject}.global_args[{}]", global_args.len() - 1),
                                Some(origin),
                            );
                        }
                    }
                    VolumeKey::PodmanArgs => {
                        if value.is_empty() {
                            podman_args = Some(Vec::new());
                            podman_args_origins = vec![origin.clone()];
                            self.unsupported_value(
                                &format!("{subject}.podman_args"),
                                filename,
                                "PodmanArgs",
                                "an empty assignment reset cannot be regenerated exactly",
                                origin,
                            );
                        } else {
                            let podman_args = podman_args.get_or_insert_default();
                            podman_args.push(sourced(value));
                            self.exact(
                                format!("{subject}.podman_args[{}]", podman_args.len() - 1),
                                Some(origin),
                            );
                        }
                    }
                    VolumeKey::User => {
                        volume.set_user(sourced(value));
                        self.exact(format!("{subject}.user"), Some(origin));
                    }
                    VolumeKey::Group => {
                        volume.set_group(sourced(value));
                        self.exact(format!("{subject}.group"), Some(origin));
                    }
                    VolumeKey::UID => {
                        volume.set_uid(sourced(value));
                        self.exact(format!("{subject}.uid"), Some(origin));
                    }
                    VolumeKey::GID => {
                        volume.set_gid(sourced(value));
                        self.exact(format!("{subject}.gid"), Some(origin));
                    }
                    VolumeKey::ServiceName => {
                        volume.set_service_name(sourced(value));
                        self.exact(format!("{subject}.service_name"), Some(origin));
                    }
                    VolumeKey::Image => {
                        let image = if let Some(stem) = value.strip_suffix(".image") {
                            self.identifier(stem, &format!("{subject}.image"), origin.clone())
                                .map(VolumeImageSource::ImageAcquisition)
                        } else if let Some(stem) = value.strip_suffix(".build") {
                            self.identifier(stem, &format!("{subject}.image"), origin.clone())
                                .map(VolumeImageSource::ImageBuild)
                        } else {
                            Some(VolumeImageSource::Literal(ProtectedString::sensitive(value)))
                        };
                        if let Some(image) = image {
                            if let Err(error) = volume.set_image_source(Sourced::from_source(image, origin.clone())) {
                                self.invalid_model(
                                    &format!("{subject}.image"),
                                    filename,
                                    &error.to_string(),
                                    Some(origin),
                                );
                            } else {
                                self.exact(format!("{subject}.image"), Some(origin));
                            }
                        }
                    }
                    _ => self.unsupported(
                        &format!("{subject}.quadlet.{}", entry.key().text()),
                        filename,
                        entry.key().text(),
                        origin,
                    ),
                }
            }
        }
        if let Some(labels) = labels {
            volume.set_labels_with_origins(labels, label_origins);
        }
        if let Some(modules) = modules {
            volume.set_containers_conf_modules_with_origins(modules, module_origins);
        }
        if let Some(global_args) = global_args {
            volume.set_global_args_with_origins(global_args, global_args_origins);
        }
        if let Some(podman_args) = podman_args {
            volume.set_podman_args_with_origins(podman_args, podman_args_origins);
        }
        if volume.volume_type().is_some() && volume.device().is_none() {
            self.invalid_model(
                &format!("{subject}.type"),
                filename,
                "Type requires Device",
                Some(document_origin),
            );
        }
        volume
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the ten related typed network keys share one source-order and IPAM-association contract"
    )]
    fn map_network_definition(
        &mut self,
        filename: &str,
        name: Identifier,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Network {
        let subject = format!("networks.{}", name.as_str());
        let mut network = Network::new(name, ResourceOwnership::Application);
        let mut singletons = BTreeSet::new();
        let mut options = None;
        let mut option_names = BTreeSet::new();
        let mut option_reset_origins = Vec::new();
        let mut labels = None;
        let mut label_names = BTreeSet::new();
        let mut label_reset_origins = Vec::new();
        let mut ipam_entries = Vec::new();

        for section in sections {
            for entry in section.entries() {
                let origin = self.entry_origin(entry);
                let EntryKind::Network(key) = entry.kind() else {
                    self.unsupported(
                        &format!("quadlet.{filename}.{}", entry.key().text()),
                        filename,
                        entry.key().text(),
                        origin,
                    );
                    continue;
                };
                if matches!(
                    key,
                    NetworkKey::NetworkName
                        | NetworkKey::Driver
                        | NetworkKey::Internal
                        | NetworkKey::IPv6
                        | NetworkKey::IPAMDriver
                ) && !singletons.insert(key)
                {
                    self.invalid_model(
                        &format!("{subject}.{}", entry.key().text()),
                        filename,
                        "Quadlet network singleton is declared more than once",
                        Some(origin),
                    );
                    continue;
                }
                let Some(value) = self.network_direct_value(filename, &subject, entry, origin.clone()) else {
                    continue;
                };
                match key {
                    NetworkKey::NetworkName => {
                        if is_safe_network_scalar(value, false) {
                            network
                                .set_runtime_name(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
                            self.exact(format!("{subject}.runtime_name"), Some(origin));
                        } else {
                            self.unsupported_value(
                                &subject,
                                filename,
                                "NetworkName",
                                "NetworkName requires an unquoted systemd-safe scalar",
                                origin,
                            );
                        }
                    }
                    NetworkKey::Driver => {
                        if is_safe_network_scalar(value, false) {
                            network.set_driver(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
                            self.exact(format!("{subject}.driver"), Some(origin));
                        } else {
                            self.unsupported_value(
                                &subject,
                                filename,
                                "Driver",
                                "Driver requires an unquoted systemd-safe scalar",
                                origin,
                            );
                        }
                    }
                    NetworkKey::Options if value.is_empty() => {
                        options = Some(Vec::new());
                        option_names.clear();
                        option_reset_origins = vec![origin.clone()];
                        self.unsupported_value(
                            &format!("{subject}.driver_options"),
                            filename,
                            "Options",
                            "an empty Options assignment resets the effective list and cannot be regenerated exactly",
                            origin,
                        );
                    }
                    NetworkKey::Options => match parse_network_assignment(value) {
                        Some((option_name, option_value)) => {
                            let Some(option_name) =
                                self.identifier(option_name, &format!("{subject}.driver_options"), origin.clone())
                            else {
                                continue;
                            };
                            if !option_names.insert(option_name.as_str().to_owned()) {
                                self.unsupported_value(
                                    &format!("{subject}.driver_options"),
                                    filename,
                                    "Options",
                                    "duplicate network option names are collapsed by native processing and cannot be imported exactly",
                                    origin,
                                );
                                continue;
                            }
                            match NetworkDriverOption::new(
                                Sourced::from_source(option_name, origin.clone()),
                                Sourced::from_source(ProtectedString::sensitive(option_value), origin.clone()),
                            ) {
                                Ok(option) => {
                                    let options = options.get_or_insert_default();
                                    options.push(Sourced::from_source(option, origin.clone()));
                                    self.exact(
                                        format!("{subject}.driver_options[{}]", options.len() - 1),
                                        Some(origin),
                                    );
                                }
                                Err(error) => self.invalid_model(&subject, filename, &error.to_string(), Some(origin)),
                            }
                        }
                        None => self.unsupported_value(
                            &format!("{subject}.driver_options"),
                            filename,
                            "Options",
                            "Options requires one explicit unquoted systemd-safe NAME=VALUE assignment",
                            origin,
                        ),
                    },
                    NetworkKey::Label if value.is_empty() => {
                        labels = Some(Vec::new());
                        label_names.clear();
                        label_reset_origins = vec![origin.clone()];
                        self.unsupported_value(
                            &format!("{subject}.labels"),
                            filename,
                            "Label",
                            "an empty Label assignment resets the effective list and cannot be regenerated exactly",
                            origin,
                        );
                    }
                    NetworkKey::Label => match parse_network_label(value) {
                        Some((label_name, label_value)) => {
                            let Some(label_name) =
                                self.identifier(label_name, &format!("{subject}.labels"), origin.clone())
                            else {
                                continue;
                            };
                            if !label_names.insert(label_name.as_str().to_owned()) {
                                self.unsupported_value(
                                    &format!("{subject}.labels"),
                                    filename,
                                    "Label",
                                    "duplicate network label names are collapsed by native processing and cannot be imported exactly",
                                    origin,
                                );
                                continue;
                            }
                            let label = MetadataLabel::new(label_name, ProtectedString::sensitive(label_value));
                            let labels = labels.get_or_insert_default();
                            labels.push(Sourced::from_source(label, origin.clone()));
                            self.exact(format!("{subject}.labels[{}]", labels.len() - 1), Some(origin));
                        }
                        None => self.unsupported_value(
                            &format!("{subject}.labels"),
                            filename,
                            "Label",
                            "Label requires one explicit unquoted systemd-safe NAME=VALUE assignment",
                            origin,
                        ),
                    },
                    NetworkKey::Internal | NetworkKey::IPv6 => {
                        let Some(value) = parse_canonical_network_bool(value) else {
                            self.invalid_model(
                                &format!(
                                    "{subject}.{}",
                                    if key == NetworkKey::Internal {
                                        "internal"
                                    } else {
                                        "ipv6"
                                    }
                                ),
                                filename,
                                "network booleans must use canonical true or false spelling",
                                Some(origin),
                            );
                            continue;
                        };
                        if key == NetworkKey::Internal {
                            network.set_internal(Sourced::from_source(value, origin.clone()));
                            self.exact(format!("{subject}.internal"), Some(origin));
                        } else {
                            network.set_ipv6(Sourced::from_source(value, origin.clone()));
                            self.exact(format!("{subject}.ipv6"), Some(origin));
                        }
                    }
                    NetworkKey::IPAMDriver => {
                        if is_safe_network_scalar(value, false) {
                            network.set_ipam_driver(Sourced::from_source(
                                ProtectedString::sensitive(value),
                                origin.clone(),
                            ));
                            self.exact(format!("{subject}.ipam_driver"), Some(origin));
                        } else {
                            self.unsupported_value(
                                &subject,
                                filename,
                                "IPAMDriver",
                                "IPAMDriver requires an unquoted systemd-safe scalar",
                                origin,
                            );
                        }
                    }
                    NetworkKey::Subnet | NetworkKey::Gateway | NetworkKey::IPRange => {
                        ipam_entries.push((key, value.to_owned(), origin));
                    }
                    _ => self.unsupported_value(
                        &format!("quadlet.{filename}.{}", entry.key().text()),
                        filename,
                        entry.key().text(),
                        "unrecognized typed network key",
                        origin,
                    ),
                }
            }
        }
        if let Some(options) = options {
            network.set_driver_options_with_origins(options, option_reset_origins);
        }
        if let Some(labels) = labels {
            network.set_labels_with_origins(labels, label_reset_origins);
        }
        self.map_network_ipam(filename, &subject, &mut network, &ipam_entries, document_origin);
        self.exact(subject, None);
        network
    }

    #[expect(
        clippy::too_many_lines,
        reason = "IPAM association deliberately checks every native column before retaining one neutral row"
    )]
    fn map_network_ipam(
        &mut self,
        filename: &str,
        subject: &str,
        network: &mut Network,
        entries: &[(NetworkKey, String, Provenance)],
        document_origin: Provenance,
    ) {
        if entries.is_empty() {
            return;
        }
        if entries.iter().any(|(_, value, _)| value.is_empty()) {
            if entries.iter().all(|(_, value, _)| value.is_empty()) {
                network.set_ipam_configs_with_origins(
                    Vec::new(),
                    entries.iter().map(|(_, _, origin)| origin.clone()).collect(),
                );
            }
            self.unsupported_value(
                &format!("{subject}.ipam_configs"),
                filename,
                "IPAM",
                "an empty IPAM column assignment resets native state and cannot be regenerated exactly",
                document_origin,
            );
            return;
        }
        if entries
            .iter()
            .any(|(_, value, _)| !is_safe_network_scalar(value, false))
        {
            self.unsupported_value(
                &format!("{subject}.ipam_configs"),
                filename,
                "IPAM",
                "IPAM columns require unquoted systemd-safe scalar values",
                document_origin,
            );
            return;
        }
        let subnets = entries.iter().filter(|(key, _, _)| *key == NetworkKey::Subnet).count();
        let gateways = entries.iter().filter(|(key, _, _)| *key == NetworkKey::Gateway).count();
        let ranges = entries.iter().filter(|(key, _, _)| *key == NetworkKey::IPRange).count();
        if subnets == 0 && (gateways > 0 || ranges > 0) {
            self.invalid_model(
                &format!("{subject}.ipam_configs"),
                filename,
                "Gateway or IPRange requires a Subnet",
                Some(document_origin),
            );
            return;
        }
        if subnets != 1 || gateways > 1 || ranges > 1 {
            self.unsupported_value(
                &format!("{subject}.ipam_configs"),
                filename,
                "IPAM",
                "independent repeated IPAM columns cannot be associated safely without positional zipping",
                document_origin,
            );
            return;
        }
        let Some(subnet_index) = entries.iter().position(|(key, _, _)| *key == NetworkKey::Subnet) else {
            self.invalid_model(
                &format!("{subject}.ipam_configs"),
                filename,
                "IPAM association unexpectedly lacks its required Subnet",
                Some(document_origin),
            );
            return;
        };
        if entries[..subnet_index]
            .iter()
            .any(|(key, _, _)| matches!(key, NetworkKey::Gateway | NetworkKey::IPRange))
        {
            self.unsupported_value(
                &format!("{subject}.ipam_configs"),
                filename,
                "IPAM",
                "Gateway and IPRange must physically follow their Subnet to establish one safe row",
                document_origin,
            );
            return;
        }
        let (subnet, subnet_origin) = (&entries[subnet_index].1, entries[subnet_index].2.clone());
        let Ok(mut row) = NetworkIpamConfig::new(Sourced::from_source(
            ProtectedString::sensitive(subnet),
            subnet_origin.clone(),
        )) else {
            self.invalid_model(
                &format!("{subject}.ipam_configs"),
                filename,
                "invalid IPAM subnet",
                Some(subnet_origin),
            );
            return;
        };
        for (key, value, origin) in entries.iter().skip(subnet_index + 1) {
            let result = match key {
                NetworkKey::Gateway => {
                    row.set_gateway(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()))
                }
                NetworkKey::IPRange => {
                    row.set_ip_range(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()))
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                self.invalid_model(
                    &format!("{subject}.ipam_configs"),
                    filename,
                    &error.to_string(),
                    Some(origin.clone()),
                );
                return;
            }
        }
        network.add_ipam_config(Sourced::from_source(row, subnet_origin));
        self.exact(format!("{subject}.ipam_configs[0]"), None);
    }

    fn map_image_definition(
        &mut self,
        filename: &str,
        name: Identifier,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Sourced<ImageAcquisition> {
        let subject = format!("image_acquisitions.{}", name.as_str());
        let mut resource = ImageAcquisition::new(name);
        let mut settings = Vec::new();
        let mut origins = Vec::new();
        let mut singletons = BTreeSet::new();
        for section in sections {
            for entry in section.entries() {
                let origin = self.entry_origin(entry);
                origins.push(origin.clone());
                if !entry.kind().is_repeatable() && !singletons.insert(entry.kind()) {
                    self.invalid_model(
                        &format!("{subject}.{}", entry.key().text()),
                        filename,
                        "Quadlet image singleton is declared more than once",
                        Some(origin),
                    );
                    continue;
                }
                let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                    continue;
                };
                let protected = |s: &str| ProtectedString::plain(s);
                let setting = match (section.kind(), entry.kind()) {
                    (SectionKind::Image, EntryKind::Image(ImageKey::Image)) => {
                        ImageAcquisitionSetting::Image(protected(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::ImageTag)) => {
                        ImageAcquisitionSetting::ImageTags(repeated(protected(value), origin.clone()))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::ServiceName)) => {
                        ImageAcquisitionSetting::ServiceName(protected(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::AllTags)) => {
                        let Some(value) = parse_quadlet_bool(value) else {
                            self.invalid_model(
                                &format!("{subject}.all_tags"),
                                filename,
                                "AllTags must be a Quadlet boolean",
                                Some(origin),
                            );
                            continue;
                        };
                        ImageAcquisitionSetting::AllTags(value)
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::Arch)) => {
                        ImageAcquisitionSetting::Architecture(protected(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::AuthFile)) => {
                        ImageAcquisitionSetting::AuthFile(ProtectedString::sensitive(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::CertDir)) => {
                        ImageAcquisitionSetting::CertificateDirectory(protected(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::ContainersConfModule)) => {
                        ImageAcquisitionSetting::ContainersConfigModules(repeated(protected(value), origin.clone()))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::Creds)) => {
                        ImageAcquisitionSetting::Credentials(ProtectedString::sensitive(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::DecryptionKey)) => {
                        ImageAcquisitionSetting::DecryptionKey(ProtectedString::sensitive(value))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::GlobalArgs)) => {
                        ImageAcquisitionSetting::GlobalArguments(repeated(protected(value), origin.clone()))
                    }
                    (SectionKind::Image, EntryKind::Image(ImageKey::OS)) => {
                        ImageAcquisitionSetting::OperatingSystem(protected(value))
                    }
                    _ => {
                        self.unsupported(
                            &format!("{subject}.quadlet.{}", entry.key().text()),
                            filename,
                            entry.key().text(),
                            origin,
                        );
                        continue;
                    }
                };
                self.exact(format!("{subject}.{}", entry.key().text()), Some(origin.clone()));
                settings.push(Sourced::from_source(setting, origin));
            }
        }
        resource.set_settings_with_origins(settings, origins);
        Sourced::from_source(resource, document_origin)
    }

    fn map_build_definition(
        &mut self,
        filename: &str,
        name: Identifier,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Sourced<ImageBuild> {
        let subject = format!("image_builds.{}", name.as_str());
        let mut resource = ImageBuild::new(name);
        let mut settings = Vec::new();
        let mut origins = Vec::new();
        let mut singletons = BTreeSet::new();
        for section in sections {
            for entry in section.entries() {
                let origin = self.entry_origin(entry);
                origins.push(origin.clone());
                if !entry.kind().is_repeatable() && !singletons.insert(entry.kind()) {
                    self.invalid_model(
                        &format!("{subject}.{}", entry.key().text()),
                        filename,
                        "Quadlet build singleton is declared more than once",
                        Some(origin),
                    );
                    continue;
                }
                let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                    continue;
                };
                let setting = if is_repeated_build_key(entry.kind()) {
                    self.map_repeated_build_setting(filename, &subject, section, entry, value, &origin)
                } else {
                    self.map_singleton_build_setting(filename, &subject, section, entry, value, &origin)
                };
                let Some(setting) = setting else { continue };
                self.exact(format!("{subject}.{}", entry.key().text()), Some(origin.clone()));
                settings.push(Sourced::from_source(setting, origin));
            }
        }
        resource.set_settings_with_origins(settings, origins);
        Sourced::from_source(resource, document_origin)
    }

    fn map_singleton_build_setting(
        &mut self,
        filename: &str,
        subject: &str,
        section: &TypedSection,
        entry: &TypedEntry,
        value: &str,
        origin: &Provenance,
    ) -> Option<ImageBuildSetting> {
        let plain = ProtectedString::plain;
        match (section.kind(), entry.kind()) {
            (SectionKind::Build, EntryKind::Build(BuildKey::SetWorkingDirectory)) => {
                Some(ImageBuildSetting::SetWorkingDirectory(plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::File)) => Some(ImageBuildSetting::RecipeFile(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::Target)) => Some(ImageBuildSetting::Target(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::Network)) => Some(ImageBuildSetting::Network(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::Arch)) => {
                Some(ImageBuildSetting::Architecture(plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Variant)) => Some(ImageBuildSetting::Variant(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::Pull)) => Some(ImageBuildSetting::PullPolicy(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::Retry)) => Some(ImageBuildSetting::Retry(plain(value))),
            (SectionKind::Build, EntryKind::Build(BuildKey::RetryDelay)) => {
                Some(ImageBuildSetting::RetryDelay(plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::AuthFile)) => {
                Some(ImageBuildSetting::AuthFile(ProtectedString::sensitive(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::IgnoreFile)) => {
                Some(ImageBuildSetting::IgnoreFile(plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::ServiceName)) => {
                Some(ImageBuildSetting::ServiceName(plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::TLSVerify)) => {
                let Some(value) = parse_quadlet_bool(value) else {
                    self.invalid_model(
                        &format!("{subject}.tls_verify"),
                        filename,
                        "TLSVerify must be a Quadlet boolean",
                        Some(origin.clone()),
                    );
                    return None;
                };
                Some(ImageBuildSetting::TlsVerify(value))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::ForceRM)) => {
                let Some(value) = parse_quadlet_bool(value) else {
                    self.invalid_model(
                        &format!("{subject}.force_remove"),
                        filename,
                        "ForceRM must be a Quadlet boolean",
                        Some(origin.clone()),
                    );
                    return None;
                };
                Some(ImageBuildSetting::ForceRemove(value))
            }
            _ => {
                self.unsupported(
                    &format!("{subject}.quadlet.{}", entry.key().text()),
                    filename,
                    entry.key().text(),
                    origin.clone(),
                );
                None
            }
        }
    }

    fn map_repeated_build_setting(
        &mut self,
        filename: &str,
        subject: &str,
        section: &TypedSection,
        entry: &TypedEntry,
        value: &str,
        origin: &Provenance,
    ) -> Option<ImageBuildSetting> {
        let plain = ProtectedString::plain;
        let repeated_plain = |value| repeated(plain(value), origin.clone());
        let repeated_assignment = |value| repeated(artifact_assignment(value, true), origin.clone());
        match (section.kind(), entry.kind()) {
            (SectionKind::Build, EntryKind::Build(BuildKey::ImageTag)) => {
                Some(ImageBuildSetting::ImageTags(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Label)) => {
                Some(ImageBuildSetting::Labels(repeated_assignment(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::BuildArg)) => {
                Some(ImageBuildSetting::BuildArguments(repeated_assignment(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Secret)) => Some(ImageBuildSetting::Secrets(repeated(
                ProtectedString::sensitive(value),
                origin.clone(),
            ))),
            (SectionKind::Build, EntryKind::Build(BuildKey::PodmanArgs)) => Some(ImageBuildSetting::RuntimeArguments(
                repeated(ProtectedString::sensitive(value), origin.clone()),
            )),
            (SectionKind::Build, EntryKind::Build(BuildKey::GroupAdd)) => {
                Some(ImageBuildSetting::GroupAdd(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::DNS)) => {
                Some(ImageBuildSetting::DnsServers(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::DNSOption)) => {
                Some(ImageBuildSetting::DnsOptions(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::DNSSearch)) => {
                Some(ImageBuildSetting::DnsSearchDomains(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Annotation)) => {
                Some(ImageBuildSetting::Annotations(repeated_assignment(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Environment)) => {
                Some(ImageBuildSetting::Environment(repeated_assignment(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::ContainersConfModule)) => {
                Some(ImageBuildSetting::ContainersConfigModules(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::GlobalArgs)) => {
                Some(ImageBuildSetting::GlobalArguments(repeated_plain(value)))
            }
            (SectionKind::Build, EntryKind::Build(BuildKey::Volume)) => {
                Some(ImageBuildSetting::Volumes(repeated_plain(value)))
            }
            _ => {
                self.unsupported(
                    &format!("{subject}.quadlet.{}", entry.key().text()),
                    filename,
                    entry.key().text(),
                    origin.clone(),
                );
                None
            }
        }
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
            group.set_runtime(Sourced::from_source(imported.runtime, imported.origin.clone()));
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
        application: &mut Application,
        sections: &[TypedSection],
        document_origin: Provenance,
    ) -> Option<PodImportGroup> {
        let name = self.identifier(stem, "service_groups", document_origin.clone())?;
        let subject = format!("service_groups.{stem}.runtime");
        let mut pod_name_seen = false;
        let mut singletons = BTreeSet::new();
        let mut runtime = ServiceGroupRuntime::new();

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
                        runtime.set_runtime_name(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
                        self.exact(format!("{subject}.pod_name"), Some(origin));
                    }
                    (SectionKind::Pod, EntryKind::Pod(key)) => {
                        if !entry.kind().is_repeatable() && !singletons.insert(key) {
                            self.invalid_model(
                                &subject,
                                filename,
                                "Quadlet singleton pod key is declared more than once",
                                Some(self.entry_origin(entry)),
                            );
                            continue;
                        }
                        self.map_pod_runtime_entry(filename, stem, application, &mut runtime, entry);
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

        if runtime.networks().is_some_and(|networks| {
            networks
                .iter()
                .any(|network| network.value().network().as_str() == "host")
        }) && runtime.ports().is_some_and(|ports| !ports.is_empty())
        {
            self.invalid_model(
                &format!("service_groups.{stem}.runtime.ports"),
                filename,
                "Network=host conflicts with PublishPort",
                Some(document_origin.clone()),
            );
        }

        Some(PodImportGroup {
            filename: filename.to_owned(),
            name,
            origin: document_origin,
            members: Vec::new(),
            runtime,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn map_pod_runtime_entry(
        &mut self,
        filename: &str,
        stem: &str,
        application: &mut Application,
        runtime: &mut ServiceGroupRuntime,
        entry: &TypedEntry,
    ) {
        let subject = format!("service_groups.{stem}.runtime.{}", entry.key().text());
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        match entry.kind() {
            EntryKind::Pod(PodKey::AddHost) if value.is_empty() => {
                runtime.set_host_mappings_with_origins(Vec::new(), vec![origin.clone()]);
                self.unsupported_value(
                    &subject,
                    filename,
                    "AddHost",
                    "an empty AddHost assignment resets the effective list",
                    origin,
                );
            }
            EntryKind::Pod(PodKey::AddHost) => match decode_host_mappings(value) {
                Ok(values) => {
                    for value in values {
                        runtime.add_host_mapping(Sourced::from_source(value, origin.clone()));
                    }
                    self.exact(subject, Some(origin));
                }
                Err(ValueIssue::Unsupported(reason)) => {
                    self.unsupported_value(&subject, filename, "AddHost", reason, origin);
                }
                Err(ValueIssue::Invalid(reason)) => self.invalid_model(&subject, filename, reason, Some(origin)),
            },
            EntryKind::Pod(PodKey::PublishPort) if value.is_empty() => {
                runtime.set_ports_with_origins(Vec::new(), vec![origin.clone()]);
                self.unsupported_value(
                    &subject,
                    filename,
                    "PublishPort",
                    "an empty PublishPort assignment resets the effective list",
                    origin,
                );
            }
            EntryKind::Pod(PodKey::PublishPort) => match decode_port(value) {
                Ok(value) => {
                    runtime.add_port(Sourced::from_source(value, origin.clone()));
                    self.exact(subject, Some(origin));
                }
                Err(ValueIssue::Unsupported(reason)) => {
                    self.unsupported_value(&subject, filename, "PublishPort", reason, origin);
                }
                Err(ValueIssue::Invalid(reason)) => self.invalid_model(&subject, filename, reason, Some(origin)),
            },
            EntryKind::Pod(PodKey::Network) => {
                if value.is_empty() {
                    runtime.set_networks_with_origins(Vec::new(), vec![origin.clone()]);
                    self.unsupported_value(
                        &subject,
                        filename,
                        "Network",
                        "an empty Network assignment resets the effective list",
                        origin,
                    );
                    return;
                }
                let (name, external) = match entry.value_kind() {
                    ValueKind::UnitReference(UnitReferenceKind::Network) => (value.strip_suffix(".network"), false),
                    _ if value == "host" => (Some(value), false),
                    _ if is_external_network_name(value) => (Some(value), true),
                    _ => {
                        self.unsupported_value(
                            &subject,
                            filename,
                            "Network",
                            "Network mode and target-specific arguments remain unsupported",
                            origin,
                        );
                        return;
                    }
                };
                let Some(name) = name else {
                    self.invalid_model(&subject, filename, "invalid .network unit reference", Some(origin));
                    return;
                };
                let Some(name) = self.identifier(name, &subject, origin.clone()) else {
                    return;
                };
                if external && !self.ensure_external_network(application, &name, filename, origin.clone()) {
                    return;
                }
                runtime.add_network(Sourced::from_source(
                    NetworkAttachment::new(name, Vec::new()),
                    origin.clone(),
                ));
                self.exact(subject, Some(origin));
            }
            EntryKind::Pod(PodKey::Volume) if value.is_empty() => {
                runtime.set_mounts_with_origins(Vec::new(), vec![origin.clone()]);
                self.unsupported_value(
                    &subject,
                    filename,
                    "Volume",
                    "an empty Volume assignment resets the effective list",
                    origin,
                );
            }
            EntryKind::Pod(PodKey::Volume) => match decode_mount(value, entry.value_kind()) {
                Ok((mount, external)) => {
                    if let Some(name) = external {
                        if !self.ensure_external_volume(application, &name, filename, origin.clone()) {
                            return;
                        }
                    }
                    runtime.add_mount(Sourced::from_source(mount, origin.clone()));
                    self.exact(subject, Some(origin));
                }
                Err(ValueIssue::Unsupported(reason)) => {
                    self.unsupported_value(&subject, filename, "Volume", reason, origin);
                }
                Err(ValueIssue::Invalid(reason)) => self.invalid_model(&subject, filename, reason, Some(origin)),
            },
            EntryKind::Pod(PodKey::UserNS) => {
                runtime.set_user_namespace(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
                self.exact(subject, Some(origin));
            }
            EntryKind::Pod(PodKey::ShmSize) => {
                runtime.set_shm_size(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
                if is_positive_memory(value) || value == "0" {
                    self.exact(subject, Some(origin));
                } else {
                    self.unsupported_value(
                        &subject,
                        filename,
                        "ShmSize",
                        "pod shared-memory size is retained but outside the reviewed grammar",
                        origin,
                    );
                }
            }
            EntryKind::Pod(PodKey::ExitPolicy) => {
                let policy = match value {
                    "stop" => GroupExitPolicy::Stop,
                    "continue" => GroupExitPolicy::Continue,
                    _ => GroupExitPolicy::Raw(ProtectedString::sensitive(value)),
                };
                runtime.set_exit_policy(Sourced::from_source(policy, origin.clone()));
                if matches!(value, "stop" | "continue") {
                    self.exact(subject, Some(origin));
                } else {
                    self.unsupported_value(
                        &subject,
                        filename,
                        "ExitPolicy",
                        "native exit policy is retained as a raw target-specific value",
                        origin,
                    );
                }
            }
            EntryKind::Pod(PodKey::StopTimeout) => match StopTimeout::new(value) {
                Ok(value) => {
                    let exact = is_canonical_nonnegative_seconds(value.as_str());
                    runtime.set_stop_timeout(Sourced::from_source(value, origin.clone()));
                    if exact {
                        self.exact(subject, Some(origin));
                    } else {
                        self.unsupported_value(
                            &subject,
                            filename,
                            "StopTimeout",
                            "pod StopTimeout is retained but must be canonical integral seconds",
                            origin,
                        );
                    }
                }
                Err(error) => self.invalid_model(&subject, filename, &error.to_string(), Some(origin)),
            },
            EntryKind::Pod(PodKey::ServiceName) => {
                if value.is_empty() || value.ends_with(".service") {
                    self.invalid_model(
                        &subject,
                        filename,
                        "Pod ServiceName must be an unsuffixed service-name stem",
                        Some(origin),
                    );
                    return;
                }
                runtime.set_service_name(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
                self.exact(subject, Some(origin));
            }
            _ => self.unsupported(&subject, filename, entry.key().text(), origin),
        }
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
        document: &QuadletDocument,
        service_names: &BTreeSet<String>,
        pod_groups: &mut BTreeMap<String, PodImportGroup>,
    ) {
        let service_name = service.name().as_str().to_owned();
        let mut state = ContainerImportState::default();

        for section in document.sections() {
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
        self.map_container_environment(filename, &service_name, service, document);
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
        self.map_deferred_network_attachment_entries(
            filename,
            &service_name,
            service,
            &state.network_attachment_entries,
        );
    }

    #[allow(clippy::too_many_lines)] // Dispatch preserves typed native entry order in one place.
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
                EntryKind::Container(ContainerKey::Rootfs) => {
                    self.map_rootfs(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Notify) => {
                    self.map_startup_notification(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::PodmanArgs) => {
                    self.map_podman_args(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::ContainerName) => {
                    self.map_container_name(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Exec) => {
                    self.map_command(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Entrypoint) => {
                    self.map_entrypoint(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::RunInit) => {
                    self.map_run_init(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::StopTimeout) => {
                    self.map_stop_timeout(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Pull) => self.map_pull(filename, &service_name, service, entry),
                EntryKind::Container(ContainerKey::Memory) => self.map_memory(filename, &service_name, service, entry),
                // Environment= is decoded once for the complete document below. The Lens semantic
                // view handles systemd quoting, multiple assignments, resets, and deferred forms.
                EntryKind::Container(ContainerKey::Environment) => {}
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
                EntryKind::Container(ContainerKey::ExposeHostPort) => {
                    self.map_exposed_port(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Volume) => {
                    self.map_mount(filename, &service_name, application, service, entry);
                }
                EntryKind::Container(ContainerKey::Network) => {
                    self.map_network(filename, &service_name, application, service, entry);
                }
                EntryKind::Container(ContainerKey::IP | ContainerKey::IP6 | ContainerKey::NetworkAlias) => {
                    state.network_attachment_entries.push(entry);
                }
                EntryKind::Container(ContainerKey::Pod) => {
                    self.map_pod_membership(filename, &service_name, entry, pod_groups);
                }
                EntryKind::Container(ContainerKey::Label) => {
                    self.map_label(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::Annotation) => {
                    self.map_annotation(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::LogDriver) => {
                    self.map_log_driver(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::LogOpt) => self.map_log_opt(filename, &service_name, service, entry),
                EntryKind::Container(ContainerKey::ReloadCmd) => {
                    self.map_reload_command(filename, &service_name, service, entry);
                }
                EntryKind::Container(ContainerKey::ReloadSignal) => {
                    self.map_reload_signal(filename, &service_name, service, entry);
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
                EntryKind::Container(ContainerKey::HostName) => {
                    self.map_raw_service_value(filename, &service_name, service, entry, "hostname");
                }
                EntryKind::Container(ContainerKey::PidsLimit) => {
                    self.map_raw_service_value(filename, &service_name, service, entry, "pids_limit");
                }
                EntryKind::Container(ContainerKey::ShmSize) => {
                    self.map_raw_service_value(filename, &service_name, service, entry, "shm_size");
                }
                EntryKind::Container(ContainerKey::StopSignal) => {
                    self.map_raw_service_value(filename, &service_name, service, entry, "stop_signal");
                }
                EntryKind::Container(ContainerKey::AddCapability) => {
                    self.map_capability(filename, &service_name, service, entry, true);
                }
                EntryKind::Container(ContainerKey::DropCapability) => {
                    self.map_capability(filename, &service_name, service, entry, false);
                }
                EntryKind::Container(ContainerKey::Tmpfs) => self.map_tmpfs(filename, &service_name, service, entry),
                EntryKind::Container(ContainerKey::Sysctl) => self.map_sysctl(filename, &service_name, service, entry),
                EntryKind::Container(ContainerKey::Ulimit) => self.map_ulimit(filename, &service_name, service, entry),
                EntryKind::Container(ContainerKey::AddDevice) => {
                    self.map_device(filename, &service_name, service, entry);
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
            let relation = match entry.kind() {
                EntryKind::SystemdUnit(SystemdUnitKey::Requires) => Some(Some(true)),
                EntryKind::SystemdUnit(SystemdUnitKey::Wants) => Some(Some(false)),
                EntryKind::SystemdUnit(SystemdUnitKey::After) => Some(None),
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
        match entry.value_kind() {
            ValueKind::UnitReference(UnitReferenceKind::Image) => {
                let Some(name) = self.identifier(unit_stem(value), &format!("{subject}.acquisition"), origin.clone())
                else {
                    return;
                };
                service.set_image_acquisition(Sourced::from_source(name, origin.clone()));
                self.exact(format!("{subject}.acquisition"), Some(origin));
                return;
            }
            ValueKind::UnitReference(UnitReferenceKind::Build) => {
                let Some(name) = self.identifier(unit_stem(value), &format!("{subject}.build"), origin.clone()) else {
                    return;
                };
                service.set_image_build(Sourced::from_source(name, origin.clone()));
                self.exact(format!("{subject}.build"), Some(origin));
                return;
            }
            _ => {}
        }

        match ImageReference::parse(value) {
            Ok(image) => {
                service.set_image(Sourced::from_source(image, origin.clone()));
                self.exact(subject, Some(origin));
            }
            Err(error) => self.invalid_model(&subject, filename, &error.to_string(), Some(origin)),
        }
    }

    fn map_rootfs(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.rootfs");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if !is_safe_absolute_container_path(value) {
            self.unsupported_value(
                &subject,
                filename,
                "Rootfs",
                "Rootfs requires a safe absolute literal path",
                origin,
            );
            return;
        }
        match service.set_rootfs(Sourced::from_source(ProtectedString::sensitive(value), origin.clone())) {
            Ok(()) => self.exact(subject, Some(origin)),
            Err(error) => self.invalid_model(&subject, filename, &error.to_string(), Some(origin)),
        }
    }

    fn map_startup_notification(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entry: &TypedEntry,
    ) {
        let subject = format!("services.{service_name}.startup_notification");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let notification = match value {
            "false" => StartupNotification::Runtime,
            "true" => StartupNotification::Application,
            "healthy" => StartupNotification::Healthy,
            _ => {
                self.unsupported_value(
                    &subject,
                    filename,
                    "Notify",
                    "Notify must be false, true, or healthy",
                    origin,
                );
                return;
            }
        };
        service.set_startup_notification(Sourced::from_source(notification, origin.clone()));
        self.exact(subject, Some(origin));
    }

    fn map_podman_args(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.podman_args");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let mut values = service.podman_args().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = service.podman_args_origins().to_vec();
        if value.is_empty() {
            values.clear();
            origins = vec![origin.clone()];
            self.unsupported_value(
                &subject,
                filename,
                "PodmanArgs",
                "an empty PodmanArgs assignment resets the effective list",
                origin,
            );
        } else {
            values.push(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
            if !origins.contains(&origin) {
                origins.push(origin.clone());
            }
            self.unsupported_value(
                &subject,
                filename,
                "PodmanArgs",
                "PodmanArgs is retained as authored native evidence and is never synthesized",
                origin,
            );
        }
        service.set_podman_args_with_origins(values, origins);
    }

    fn map_raw_service_value(
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
        let raw = value;
        let value = Sourced::from_source(ProtectedString::plain(raw), origin.clone());
        let exact = match field {
            "hostname" => is_safe_hostname(raw),
            "pids_limit" => raw == "-1" || is_positive_canonical_decimal(raw),
            "shm_size" => is_positive_canonical_size(raw),
            "stop_signal" => is_safe_signal(raw),
            _ => true,
        };
        match field {
            "hostname" => service.set_hostname(value),
            "pids_limit" => service.set_pids_limit(value),
            "shm_size" => service.set_shm_size(value),
            "stop_signal" => service.set_stop_signal(value),
            _ => return,
        }
        if exact {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "native value is retained but outside the reviewed exact grammar",
                origin,
            );
        }
    }

    fn map_capability(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entry: &TypedEntry,
        add: bool,
    ) {
        let field = if add { "cap_add" } else { "cap_drop" };
        let subject = format!("services.{service_name}.{field}");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let mut values = if add {
            service.cap_add().map_or_else(Vec::new, ToOwned::to_owned)
        } else {
            service.cap_drop().map_or_else(Vec::new, ToOwned::to_owned)
        };
        let mut origins = if add {
            service.cap_add_origins().to_vec()
        } else {
            service.cap_drop_origins().to_vec()
        };
        if value.is_empty() {
            values.clear();
            origins = vec![origin.clone()];
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "an empty native capability assignment resets the effective list",
                origin,
            );
        } else if value.split_ascii_whitespace().all(is_safe_capability) {
            values.extend(
                value
                    .split_ascii_whitespace()
                    .map(|capability| Sourced::from_source(ProtectedString::plain(capability), origin.clone())),
            );
            if !origins.contains(&origin) {
                origins.push(origin.clone());
            }
            self.exact(subject, Some(origin));
        } else {
            values.push(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
            if !origins.contains(&origin) {
                origins.push(origin.clone());
            }
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "capabilities are retained but require a plain whitespace-separated capability list",
                origin,
            );
        }
        if add {
            service.set_cap_add_with_origins(values, origins);
        } else {
            service.set_cap_drop_with_origins(values, origins);
        }
    }

    fn map_tmpfs(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.tmpfs");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let mut values = service.tmpfs().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = service.tmpfs_origins().to_vec();
        if value.is_empty() {
            values.clear();
            origins = vec![origin.clone()];
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "an empty native tmpfs assignment resets the effective list",
                origin,
            );
        } else if is_safe_tmpfs(value) {
            values.push(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
            if !origins.contains(&origin) {
                origins.push(origin.clone());
            }
            self.exact(subject, Some(origin));
        } else {
            values.push(Sourced::from_source(ProtectedString::plain(value), origin.clone()));
            if !origins.contains(&origin) {
                origins.push(origin.clone());
            }
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "tmpfs is retained but must use an absolute target and plain comma-separated options",
                origin,
            );
        }
        service.set_tmpfs_with_origins(values, origins);
    }

    fn map_sysctl(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.sysctls");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, value)) = value.split_once('=') else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Sysctl must contain one NAME=VALUE assignment",
                origin,
            );
            return;
        };
        let mut values = service.sysctls().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = service.sysctls_origins().to_vec();
        values.push(Sourced::from_source(
            KernelParameter::new(ProtectedString::plain(name), ProtectedString::plain(value)),
            origin.clone(),
        ));
        if !origins.contains(&origin) {
            origins.push(origin.clone());
        }
        service.set_sysctls_with_origins(values, origins);
        if is_safe_sysctl_assignment(name, value) {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Sysctl is retained but must have one safe nonempty NAME=VALUE assignment",
                origin,
            );
        }
    }

    fn map_ulimit(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.ulimits");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, values)) = value.split_once('=') else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Ulimit must contain NAME=SOFT:HARD",
                origin,
            );
            return;
        };
        let Some((soft, hard)) = values.split_once(':') else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Ulimit must contain NAME=SOFT:HARD",
                origin,
            );
            return;
        };
        let mut entries = service.ulimits().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = service.ulimits_origins().to_vec();
        entries.push(Sourced::from_source(
            ResourceLimit::new(
                ProtectedString::plain(name),
                Some(Sourced::from_source(ProtectedString::plain(soft), origin.clone())),
                Some(Sourced::from_source(ProtectedString::plain(hard), origin.clone())),
            ),
            origin.clone(),
        ));
        if !origins.contains(&origin) {
            origins.push(origin.clone());
        }
        service.set_ulimits_with_origins(entries, origins);
        if is_safe_ulimit_assignment(name, soft, hard) {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Ulimit is retained but must be NAME=SOFT:HARD with safe decimal or unlimited values",
                origin,
            );
        }
    }

    fn map_device(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.devices");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let mut entries = service.devices().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = service.devices_origins().to_vec();
        entries.push(Sourced::from_source(
            Device::Short(ProtectedString::plain(value)),
            origin.clone(),
        ));
        if !origins.contains(&origin) {
            origins.push(origin.clone());
        }
        service.set_devices_with_origins(entries, origins);
        if is_safe_device(value) {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "device is retained but outside the reviewed absolute host[:container][:rwm] grammar",
                origin,
            );
        }
    }

    fn map_entrypoint(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.entrypoint");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some(args) = decode_json_exec_array(value) else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Entrypoint is exact only for reviewed JSON exec arrays",
                origin,
            );
            return;
        };
        service.set_entrypoint(Sourced::from_source(
            Entrypoint::Exec(args.into_iter().map(ProtectedString::plain).collect()),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_run_init(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.run_init");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some(value) = parse_quadlet_bool(value) else {
            self.invalid_model(&subject, filename, "RunInit must be a Quadlet boolean", Some(origin));
            return;
        };
        service.set_run_init(Sourced::from_source(value, origin.clone()));
        self.approximate(
            &subject,
            filename,
            "RunInit runtime equivalence remains reviewable",
            origin,
        );
    }

    fn map_stop_timeout(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.stop_timeout");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Ok(value) = StopTimeout::new(value) else {
            self.invalid_model(&subject, filename, "StopTimeout is invalid", Some(origin));
            return;
        };
        let exact = value.as_str().parse::<u64>().is_ok_and(|seconds| seconds > 0);
        service.set_stop_timeout(Sourced::from_source(value, origin.clone()));
        if exact {
            self.exact(subject, Some(origin));
        } else {
            self.approximate(
                &subject,
                filename,
                "fractional, zero, and default stop-timeout semantics remain reviewable",
                origin,
            );
        }
    }

    fn map_pull(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.pull_policy");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let (policy, exact) = match value {
            "always" => (PullPolicy::Always, true),
            "missing" => (PullPolicy::Missing, true),
            "never" => (PullPolicy::Never, true),
            _ => (PullPolicy::Raw(ProtectedString::sensitive(value)), false),
        };
        service.set_pull_policy(Sourced::from_source(policy, origin.clone()));
        if exact {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "native pull policy is retained as a raw target-specific value",
                origin,
            );
        }
    }

    fn map_memory(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.memory_limit");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        service.set_memory_limit(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
        if is_positive_memory(value) {
            self.exact(subject, Some(origin));
        } else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Memory must be a positive canonical byte quantity",
                origin,
            );
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

    fn map_container_environment(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        document: &QuadletDocument,
    ) {
        let environment = document.container_environment();
        for diagnostic in environment.diagnostics() {
            let mut finding = NativeFinding::new(
                "quadlet",
                "quadlet-lens",
                diagnostic.code().as_str(),
                "container-environment",
                match diagnostic.severity() {
                    quadlet_lens::diagnostic::Severity::Error => Severity::Error,
                    quadlet_lens::diagnostic::Severity::Warning => Severity::Warning,
                    quadlet_lens::diagnostic::Severity::Note => Severity::Note,
                },
                diagnostic.summary(),
            );
            for label in diagnostic.labels() {
                finding = finding.with_label(boxferry_engine::NativeFindingLabel::new(
                    boxferry_engine::NativeFindingLabelKind::Primary,
                    label.span().source_id().get(),
                    label.span().start(),
                    label.span().end(),
                    label.message(),
                ));
            }
            self.diagnostics.push(
                Diagnostic::new(
                    self.codes.native_model.clone(),
                    finding.severity(),
                    "QuadletLens reported a native container Environment finding",
                )
                .with_native_finding(finding),
            );
        }
        for directive in environment.directives() {
            let origin = self.provenance(directive.span());
            match directive {
                AuthoredContainerEnvironmentDirective::Assignment { name, value, .. } => {
                    let subject = format!("services.{service_name}.environment.{name}");
                    let Some(name) = self.identifier(name, &subject, origin.clone()) else {
                        continue;
                    };
                    service.add_environment(Sourced::from_source(
                        EnvironmentVariable::new(name, EnvironmentValue::Literal(ProtectedString::sensitive(value))),
                        origin.clone(),
                    ));
                    self.exact(subject, Some(origin));
                }
                AuthoredContainerEnvironmentDirective::Reset { .. } => self.invalid_model(
                    &format!("services.{service_name}.environment"),
                    filename,
                    "an empty Environment directive resets prior values and cannot be represented by the neutral model",
                    Some(origin),
                ),
                AuthoredContainerEnvironmentDirective::BareName { name, .. } => self.unsupported_value(
                    &format!("services.{service_name}.environment.{name}"),
                    filename,
                    "Environment",
                    "a bare Environment name requires systemd manager or process context",
                    origin,
                ),
                AuthoredContainerEnvironmentDirective::Deferred { name, .. } => self.unsupported_value(
                    &format!("services.{service_name}.environment.{name}"),
                    filename,
                    "Environment",
                    "an Environment value with a systemd specifier is deferred until manager expansion",
                    origin,
                ),
                AuthoredContainerEnvironmentDirective::Unmodeled { .. } => self.unsupported_value(
                    &format!("services.{service_name}.environment"),
                    filename,
                    "Environment",
                    "Environment syntax is not represented by the reviewed semantic subset",
                    origin,
                ),
                _ => self.unsupported_value(
                    &format!("services.{service_name}.environment"),
                    filename,
                    "Environment",
                    "a newer QuadletLens Environment directive is not represented by this neutral-model adapter",
                    origin,
                ),
            }
        }
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

    fn map_annotation(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.annotations");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, value)) = value.split_once('=') else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "Annotation must contain one explicit NAME=VALUE assignment",
                origin,
            );
            return;
        };
        let Some(name) = self.identifier(name, &subject, origin.clone()) else {
            return;
        };
        service.add_annotation(Sourced::from_source(
            Annotation::new(
                Sourced::from_source(name, origin.clone()),
                Sourced::from_source(ProtectedString::sensitive(value), origin.clone()),
            ),
            origin.clone(),
        ));
        self.exact(subject, Some(origin));
    }

    fn map_log_driver(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.logging.driver");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let mut logging = service
            .logging()
            .map(Sourced::value)
            .cloned()
            .unwrap_or_else(Logging::new);
        logging.set_driver(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
        service.set_logging(Sourced::from_source(logging, origin.clone()));
        self.unsupported_value(
            &subject,
            filename,
            entry.key().text(),
            "provider logging remains a reviewable partial cross-format mapping",
            origin,
        );
    }

    fn map_log_opt(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.logging.options");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let Some((name, value)) = value.split_once('=') else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "LogOpt must contain one explicit NAME=VALUE assignment",
                origin,
            );
            return;
        };
        let Some(name) = self.identifier(name, &subject, origin.clone()) else {
            return;
        };
        let mut logging = service
            .logging()
            .map(Sourced::value)
            .cloned()
            .unwrap_or_else(Logging::new);
        let mut options = logging.options().map_or_else(Vec::new, ToOwned::to_owned);
        let mut origins = logging.options_origins().to_vec();
        options.push(Sourced::from_source(
            LoggingOption::new(
                Sourced::from_source(name, origin.clone()),
                Sourced::from_source(ProtectedString::sensitive(value), origin.clone()),
            ),
            origin.clone(),
        ));
        if !origins.contains(&origin) {
            origins.push(origin.clone());
        }
        logging.set_options_with_origins(options, origins);
        service.set_logging(Sourced::from_source(logging, origin.clone()));
        self.unsupported_value(
            &subject,
            filename,
            entry.key().text(),
            "provider logging remains a reviewable partial cross-format mapping",
            origin,
        );
    }

    fn map_exposed_port(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        let subject = format!("services.{service_name}.exposed_ports");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        let (port, protocol) = match value.split_once('/') {
            Some((port, "tcp")) => (port, Protocol::Tcp),
            Some((port, "udp")) => (port, Protocol::Udp),
            None => (value, Protocol::Tcp),
            _ => {
                self.unsupported_value(
                    &subject,
                    filename,
                    entry.key().text(),
                    "ExposeHostPort supports only one tcp or udp port",
                    origin,
                );
                return;
            }
        };
        let Ok(port) = port.parse::<u16>() else {
            self.unsupported_value(
                &subject,
                filename,
                entry.key().text(),
                "ExposeHostPort ranges and deferred values are not exposure metadata",
                origin,
            );
            return;
        };
        let Ok(port) = ExposedPort::new(port, protocol) else {
            self.invalid_model(&subject, filename, "ExposeHostPort cannot be zero", Some(origin));
            return;
        };
        let mut ports = service.exposed_ports().map_or_else(Vec::new, ToOwned::to_owned);
        ports.push(Sourced::from_source(port, origin.clone()));
        service.set_exposed_ports_with_origins(ports, vec![origin.clone()]);
        self.exact(subject, Some(origin));
    }

    fn map_deferred_network_attachment_entries(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entries: &[&TypedEntry],
    ) {
        if entries.is_empty() {
            return;
        }
        if service.networks().len() != 1 {
            for entry in entries {
                let (field, reason) = match entry.kind() {
                    EntryKind::Container(ContainerKey::IP | ContainerKey::IP6) => (
                        "networks.address",
                        "IP/IP6 requires exactly one compatible service network; no attachment was selected",
                    ),
                    EntryKind::Container(ContainerKey::NetworkAlias) => (
                        "networks.aliases",
                        "NetworkAlias requires exactly one compatible service network; no attachment was selected",
                    ),
                    _ => continue,
                };
                let subject = format!("services.{service_name}.{field}");
                let mut origins = service
                    .networks()
                    .iter()
                    .flat_map(|network| network.origins().iter().cloned())
                    .collect::<Vec<_>>();
                origins.push(self.entry_origin(entry));
                self.unsupported_value_with_origins(&subject, filename, entry.key().text(), reason, origins);
            }
            return;
        }

        let source = service.networks()[0].clone();
        let (mut attachment, origins) = source.into_parts();
        for entry in entries {
            let origin = self.entry_origin(entry);
            match entry.kind() {
                EntryKind::Container(ContainerKey::IP) => {
                    let subject = format!("services.{service_name}.networks.ipv4_address");
                    let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                        continue;
                    };
                    if value.parse::<std::net::Ipv4Addr>().is_ok() {
                        attachment
                            .set_ipv4_address(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
                        self.exact(subject, Some(origin));
                    } else {
                        self.unsupported_value(
                            &subject,
                            filename,
                            entry.key().text(),
                            "IP is retained as source evidence but is not a reviewed IPv4 address",
                            origin,
                        );
                    }
                }
                EntryKind::Container(ContainerKey::IP6) => {
                    let subject = format!("services.{service_name}.networks.ipv6_address");
                    let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                        continue;
                    };
                    if value.parse::<std::net::Ipv6Addr>().is_ok() {
                        attachment
                            .set_ipv6_address(Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
                        self.exact(subject, Some(origin));
                    } else {
                        self.unsupported_value(
                            &subject,
                            filename,
                            entry.key().text(),
                            "IP6 is retained as source evidence but is not a reviewed IPv6 address",
                            origin,
                        );
                    }
                }
                EntryKind::Container(ContainerKey::NetworkAlias) => {
                    let subject = format!("services.{service_name}.networks.aliases");
                    let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                        continue;
                    };
                    if is_native_atom(value) {
                        attachment.add_alias(&Sourced::from_source(ProtectedString::sensitive(value), origin.clone()));
                        self.exact(subject, Some(origin));
                    } else {
                        self.unsupported_value(
                            &subject,
                            filename,
                            entry.key().text(),
                            "NetworkAlias is retained as source evidence but is not a nonempty native atom",
                            origin,
                        );
                    }
                }
                _ => {}
            }
        }
        let mut replacement = Sourced::generated(attachment);
        for origin in origins {
            replacement.add_origin(origin);
        }
        let _ = service.replace_network(0, replacement);
    }

    fn map_reload_command(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        self.map_reload(filename, service_name, service, entry, true);
    }
    fn map_reload_signal(&mut self, filename: &str, service_name: &str, service: &mut Service, entry: &TypedEntry) {
        self.map_reload(filename, service_name, service, entry, false);
    }
    fn map_reload(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        entry: &TypedEntry,
        command: bool,
    ) {
        let subject = format!("services.{service_name}.reload_action");
        let origin = self.entry_origin(entry);
        let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
            return;
        };
        if service.reload_action().is_some() {
            self.invalid_model(
                &subject,
                filename,
                "ReloadCmd and ReloadSignal are mutually exclusive",
                Some(origin),
            );
            return;
        }
        let action = if command {
            let args: Vec<_> = value.split_ascii_whitespace().collect();
            if args.is_empty() || !args.iter().all(|arg| is_safe_word(arg, false)) {
                self.unsupported_value(
                    &subject,
                    filename,
                    entry.key().text(),
                    "ReloadCmd requires unsupported systemd command-line decoding",
                    origin,
                );
                return;
            }
            ReloadAction::Command(Command::Exec(args.into_iter().map(ProtectedString::plain).collect()))
        } else {
            ReloadAction::Signal(ProtectedString::sensitive(value))
        };
        service.set_reload_action(Sourced::from_source(action, origin.clone()));
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

    fn network_direct_value<'entry>(
        &mut self,
        filename: &str,
        subject: &str,
        entry: &'entry TypedEntry,
        origin: Provenance,
    ) -> Option<&'entry str> {
        let value = self.direct_value(filename, subject, entry, origin.clone())?;
        if entry.value().primary().text() != value || value.contains(['\'', '"', '\\', '%']) {
            self.unsupported_value(
                subject,
                filename,
                entry.key().text(),
                "quoted, escaped, specifier-bearing, or whitespace-padded network values require native semantic decoding",
                origin,
            );
            return None;
        }
        Some(value)
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

    fn unsupported_value_with_origins(
        &mut self,
        subject: &str,
        document: &str,
        key: &str,
        reason: &str,
        origins: Vec<Provenance>,
    ) {
        self.unsupported_with_reason_and_origins(subject, document, key, Some(reason), origins);
    }

    fn unsupported_with_reason(
        &mut self,
        subject: &str,
        document: &str,
        key: &str,
        reason: Option<&str>,
        origin: Provenance,
    ) {
        self.unsupported_with_reason_and_origins(subject, document, key, reason, vec![origin]);
    }

    fn unsupported_with_reason_and_origins(
        &mut self,
        subject: &str,
        document: &str,
        key: &str,
        reason: Option<&str>,
        origins: Vec<Provenance>,
    ) {
        if let Ok(outcome) =
            ConversionOutcome::loss(subject, ConversionKind::Unsupported, self.codes.unsupported.clone())
        {
            self.outcomes
                .push(origins.into_iter().fold(outcome, ConversionOutcome::with_origin));
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
        ContainerKey::Rootfs => Some("rootfs"),
        ContainerKey::ContainerName => Some("runtime_name"),
        ContainerKey::Exec => Some("command"),
        ContainerKey::Entrypoint => Some("entrypoint"),
        ContainerKey::RunInit => Some("run_init"),
        ContainerKey::StopTimeout => Some("stop_timeout"),
        ContainerKey::StopSignal => Some("stop_signal"),
        ContainerKey::Pull => Some("pull_policy"),
        ContainerKey::Memory => Some("memory_limit"),
        ContainerKey::IP => Some("networks.ipv4_address"),
        ContainerKey::IP6 => Some("networks.ipv6_address"),
        ContainerKey::LogDriver => Some("logging.driver"),
        ContainerKey::ReloadCmd | ContainerKey::ReloadSignal => Some("reload_action"),
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
        ContainerKey::Notify => Some("startup_notification"),
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

fn decode_json_exec_array(value: &str) -> Option<Vec<String>> {
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() {
        return None;
    }
    let mut values = Vec::new();
    for item in inner.split(',') {
        let item = item.strip_prefix('"')?.strip_suffix('"')?;
        if item.is_empty() || item.contains(['\\', '\r', '\n', '\0']) {
            return None;
        }
        values.push(item.to_owned());
    }
    Some(values)
}

fn is_positive_memory(value: &str) -> bool {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (amount, suffix) = value.split_at(split);
    !amount.is_empty()
        && !amount.starts_with('0')
        && amount.parse::<u64>().is_ok_and(|amount| amount > 0)
        && matches!(suffix, "" | "b" | "k" | "m" | "g")
}

fn is_positive_canonical_decimal(value: &str) -> bool {
    !value.starts_with('0') && value.parse::<u64>().is_ok_and(|value| value > 0)
}
fn is_canonical_decimal(value: &str) -> bool {
    value == "0" || is_positive_canonical_decimal(value)
}
fn is_positive_canonical_size(value: &str) -> bool {
    let split = value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len());
    let (amount, suffix) = value.split_at(split);
    is_positive_canonical_decimal(amount) && matches!(suffix, "" | "b" | "k" | "m" | "g")
}
fn is_safe_signal(value: &str) -> bool {
    value
        .parse::<u8>()
        .is_ok_and(|number| (1..=64).contains(&number) && is_canonical_decimal(value))
        || value.strip_prefix("SIG").is_some_and(|name| {
            name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
}
fn is_safe_sysctl_assignment(name: &str, value: &str) -> bool {
    !name.is_empty()
        && !value.is_empty()
        && !value.contains('=')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'/' | b':'))
}
fn is_safe_ulimit_assignment(name: &str, soft: &str, hard: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !soft.contains(':')
        && !hard.contains(':')
        && [soft, hard]
            .iter()
            .all(|value| *value == "-1" || is_canonical_decimal(value))
}
fn is_safe_device(value: &str) -> bool {
    let parts: Vec<_> = value.split(':').collect();
    if !(1..=3).contains(&parts.len()) || !is_safe_device_path(parts[0]) {
        return false;
    }
    if parts.len() >= 2 && !is_safe_device_path(parts[1]) {
        return false;
    }
    parts.len() != 3
        || (!parts[2].is_empty()
            && parts[2].bytes().all(|byte| matches!(byte, b'r' | b'w' | b'm'))
            && parts[2].bytes().collect::<BTreeSet<_>>().len() == parts[2].len())
}

fn is_safe_device_path(value: &str) -> bool {
    value.starts_with("/dev/") && value.len() > 5 && is_safe_absolute_container_path(value)
}

fn is_safe_capability(value: &str) -> bool {
    let name = value.strip_prefix("CAP_").unwrap_or(value);
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_safe_tmpfs(value: &str) -> bool {
    let (target, options) = value
        .split_once(':')
        .map_or((value, None), |(target, options)| (target, Some(options)));
    is_safe_absolute_container_path(target)
        && options.is_none_or(|options| {
            !options.is_empty()
                && options
                    .split(',')
                    .all(|option| is_safe_word(option, false) && !option.contains([':', '%']))
        })
}

fn is_safe_absolute_container_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.contains('%')
        && value.split('/').all(|part| part != "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'@' | b'+'))
}

fn is_canonical_nonnegative_seconds(value: &str) -> bool {
    value == "0" || (!value.is_empty() && !value.starts_with('0') && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_safe_hostname(value: &str) -> bool {
    value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && label.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
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

fn repeated<T>(value: T, origin: Provenance) -> BuildSettingValues<T> {
    BuildSettingValues::new(BuildSyntax::Repeated, vec![Sourced::from_source(value, origin)])
}

fn is_repeated_build_key(kind: EntryKind) -> bool {
    matches!(
        kind,
        EntryKind::Build(
            BuildKey::ImageTag
                | BuildKey::Label
                | BuildKey::BuildArg
                | BuildKey::Secret
                | BuildKey::PodmanArgs
                | BuildKey::GroupAdd
                | BuildKey::DNS
                | BuildKey::DNSOption
                | BuildKey::DNSSearch
                | BuildKey::Annotation
                | BuildKey::Environment
                | BuildKey::ContainersConfModule
                | BuildKey::GlobalArgs
                | BuildKey::Volume
        )
    )
}

fn artifact_assignment(value: &str, sensitive: bool) -> ImageArtifactAssignment {
    let (name, value) = value
        .split_once('=')
        .map_or((value, None), |(name, value)| (name, Some(value)));
    let protect = |value: &str| {
        if sensitive {
            ProtectedString::sensitive(value)
        } else {
            ProtectedString::plain(value)
        }
    };
    ImageArtifactAssignment::new(protect(name), value.map(protect))
}

fn parse_quadlet_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn parse_canonical_network_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn is_safe_network_scalar(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.trim() == value
        && !value.bytes().any(|byte| {
            matches!(byte, b'\0' | b'\n' | b'\r' | b'%' | b'\'' | b'"' | b'\\') || byte.is_ascii_whitespace()
        })
}

fn parse_network_assignment(value: &str) -> Option<(&str, &str)> {
    let (name, value) = value.split_once('=')?;
    (!name.is_empty()
        && !name.contains('=')
        && is_safe_network_scalar(name, false)
        && is_safe_network_scalar(value, true))
    .then_some((name, value))
}

fn parse_network_label(value: &str) -> Option<(&str, &str)> {
    let (name, value) = parse_network_assignment(value)?;
    (is_label_name(name)).then_some((name, value))
}
