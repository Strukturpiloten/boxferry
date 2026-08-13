//! Podman inspect decoding and runtime reconstruction composition.

use std::collections::{BTreeMap, BTreeSet};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, PlatformVersion, RuleId, Severity,
};
use boxferry_model::{
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, Identifier, ImageReference, ModelError, Mount,
    MountSource, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, RestartPolicy, SelinuxRelabel,
    SourceId,
};
use boxferry_runtime::{
    ContainerObservation, CreationEvidence, EffectiveCommand, ImageObservation, NetworkObservation,
    OverrideReconstruction, PodObservation, RuntimeEnvironmentVariable, RuntimeHealthcheck, RuntimeImplementation,
    RuntimeImporter, RuntimeMetadataLabel, RuntimeResolutions, RuntimeSnapshot, RuntimeSnapshotError,
    VolumeObservation,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    MAXIMUM_PODMAN_VERSION, MINIMUM_PODMAN_VERSION,
    native::{
        ContainerInspect, HealthConfig, ImageInspect, InspectMount, NetworkInspect, PodInspect,
        RestartPolicy as NativeRestartPolicy, VolumeInspect,
    },
    source::PodmanInspectSource,
};

/// Recoverable output of decoding native Podman inspection responses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodmanSnapshotResult {
    snapshot: Option<RuntimeSnapshot>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl PodmanSnapshotResult {
    /// Returns the decoded snapshot when the supplied native data is structurally usable.
    #[must_use]
    pub const fn snapshot(&self) -> Option<&RuntimeSnapshot> {
        self.snapshot.as_ref()
    }

    /// Returns Podman-native decoding decisions.
    #[must_use]
    pub fn outcomes(&self) -> &[ConversionOutcome] {
        &self.outcomes
    }

    /// Returns structured Podman-native diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Decomposes the decoding result for composition with another importer.
    #[must_use]
    pub fn into_parts(self) -> (Option<RuntimeSnapshot>, Vec<ConversionOutcome>, Vec<Diagnostic>) {
        (self.snapshot, self.outcomes, self.diagnostics)
    }
}

/// Pure importer from caller-supplied Podman inspect JSON into the neutral application model.
#[derive(Clone, Debug)]
pub struct PodmanImporter {
    runtime: RuntimeImporter,
    codes: Codes,
}

impl PodmanImporter {
    /// Creates an importer with an explicit effective-value reconstruction policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only if a diagnostic code embedded in `BoxFerry` is invalid.
    pub fn new(override_reconstruction: OverrideReconstruction) -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            runtime: RuntimeImporter::new(override_reconstruction)?,
            codes: Codes {
                invalid_data: RuleId::PodmanDataInvalid.definition().diagnostic_code()?,
                unmodeled_configuration: RuleId::PodmanConfigurationUnsupported.definition().diagnostic_code()?,
                unsupported_version: RuleId::PodmanVersionUnsupported.definition().diagnostic_code()?,
                missing_relationship: RuleId::PodmanRelationshipMissing.definition().diagnostic_code()?,
            },
        })
    }

    /// Applies finite caller-owned lifecycle resolutions during shared reconstruction.
    #[must_use]
    pub fn with_resolutions(mut self, resolutions: RuntimeResolutions) -> Self {
        self.runtime = self.runtime.with_resolutions(resolutions);
        self
    }

    /// Returns the exact lifecycle resolutions forwarded to shared reconstruction.
    #[must_use]
    pub const fn resolutions(&self) -> &RuntimeResolutions {
        self.runtime.resolutions()
    }

    /// Decodes native documents without reconstructing a neutral application.
    #[must_use]
    pub fn decode(&self, source: &PodmanInspectSource) -> PodmanSnapshotResult {
        Decoder::new(&self.codes, source).decode()
    }
}

impl ImportAdapter for PodmanImporter {
    type Source = PodmanInspectSource;

    fn import(&self, source: &Self::Source) -> ImportResult {
        let (snapshot, mut outcomes, mut diagnostics) = self.decode(source).into_parts();
        let Some(snapshot) = snapshot else {
            return ImportResult::new(None, outcomes, diagnostics);
        };
        let (application, runtime_outcomes, runtime_diagnostics) = self.runtime.import(&snapshot).into_parts();
        outcomes.extend(runtime_outcomes);
        diagnostics.extend(runtime_diagnostics);
        ImportResult::new(application, outcomes, diagnostics)
    }
}

#[derive(Clone, Debug)]
struct Codes {
    invalid_data: DiagnosticCode,
    unmodeled_configuration: DiagnosticCode,
    unsupported_version: DiagnosticCode,
    missing_relationship: DiagnosticCode,
}

struct Decoder<'a> {
    codes: &'a Codes,
    source: &'a PodmanInspectSource,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
    invalid: bool,
}

impl<'a> Decoder<'a> {
    const fn new(codes: &'a Codes, source: &'a PodmanInspectSource) -> Self {
        Self {
            codes,
            source,
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
            invalid: false,
        }
    }

    fn decode(mut self) -> PodmanSnapshotResult {
        if !supported_version(self.source.version()) {
            self.loss(
                self.codes.unsupported_version.clone(),
                "runtime.podman.version".to_owned(),
                ConversionKind::Invalid,
                "Podman inspect version is outside the reviewed decoder range",
                vec![
                    ("version", self.source.version().to_string()),
                    ("minimum", MINIMUM_PODMAN_VERSION.to_string()),
                    ("maximum", MAXIMUM_PODMAN_VERSION.to_string()),
                    (
                        "action",
                        "supply data from a covered Podman version or add reviewed fixture evidence".to_owned(),
                    ),
                ],
                None,
            );
            self.invalid = true;
            return self.finish(None);
        }

        let containers = self.parse_document::<ContainerInspect>("containers", self.source.documents().containers());
        let images = self.parse_document::<ImageInspect>("images", self.source.documents().images());
        let networks = self.parse_document::<NetworkInspect>("networks", self.source.documents().networks());
        let volumes = self.parse_document::<VolumeInspect>("volumes", self.source.documents().volumes());
        let pods = self.parse_document::<PodInspect>("pods", self.source.documents().pods());
        let (Some(containers), Some(images), Some(networks), Some(volumes), Some(pods)) =
            (containers, images, networks, volumes, pods)
        else {
            return self.finish(None);
        };

        let mut snapshot = RuntimeSnapshot::new(self.source.application_name().clone(), RuntimeImplementation::Podman);
        let container_sources = self.container_sources(&containers);
        let pod_sources = self.pod_sources(&pods);
        let image_sources = self.map_images(&mut snapshot, images);
        self.map_networks(&mut snapshot, networks);
        self.map_volumes(&mut snapshot, volumes);
        self.map_pods(&mut snapshot, pods, &pod_sources, &container_sources);
        self.map_containers(
            &mut snapshot,
            containers,
            &container_sources,
            &pod_sources,
            &image_sources,
        );

        if self.invalid {
            self.finish(None)
        } else {
            self.finish(Some(snapshot))
        }
    }

    fn finish(self, snapshot: Option<RuntimeSnapshot>) -> PodmanSnapshotResult {
        PodmanSnapshotResult {
            snapshot,
            outcomes: self.outcomes,
            diagnostics: self.diagnostics,
        }
    }

    fn parse_document<T: DeserializeOwned>(&mut self, name: &'static str, document: &str) -> Option<Vec<T>> {
        match serde_json::from_str(document) {
            Ok(values) => Some(values),
            Err(error) => {
                self.loss(
                    self.codes.invalid_data.clone(),
                    format!("runtime.podman.documents.{name}"),
                    ConversionKind::Invalid,
                    "Podman inspect document is not a valid expected JSON array",
                    vec![
                        ("document", name.to_owned()),
                        ("line", error.line().to_string()),
                        ("column", error.column().to_string()),
                        (
                            "action",
                            "capture the complete JSON array from the matching Podman inspect command".to_owned(),
                        ),
                    ],
                    document_origin(name),
                );
                self.invalid = true;
                None
            }
        }
    }

    fn container_sources(&mut self, containers: &[ContainerInspect]) -> BTreeMap<String, SourceId> {
        let mut sources = BTreeMap::new();
        for container in containers {
            let Ok(source_id) = named_source("container", &container.name) else {
                self.invalid_resource("container", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            if container.is_infra {
                self.report_fields(
                    format!("runtime.podman.containers.{}", container.name),
                    vec!["IsInfra".to_owned()],
                    Some(&source_id),
                );
                continue;
            }
            if sources.insert(container.id.clone(), source_id).is_some() {
                self.invalid_resource(
                    "container",
                    "duplicate runtime identity was present",
                    Some(&container.name),
                );
            }
        }
        sources
    }

    fn pod_sources(&mut self, pods: &[PodInspect]) -> BTreeMap<String, SourceId> {
        let mut sources = BTreeMap::new();
        for pod in pods {
            let Ok(source_id) = named_source("pod", &pod.name) else {
                self.invalid_resource("pod", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            if sources.insert(pod.id.clone(), source_id).is_some() {
                self.invalid_resource("pod", "duplicate runtime identity was present", Some(&pod.name));
            }
        }
        sources
    }

    fn map_images(&mut self, snapshot: &mut RuntimeSnapshot, images: Vec<ImageInspect>) -> BTreeMap<String, SourceId> {
        let mut sources = BTreeMap::new();
        for (index, image) in images.into_iter().enumerate() {
            let label = index.to_string();
            let Ok(source_id) = named_source("image", &label) else {
                self.invalid_resource("image", "could not construct a stable source identity", None);
                continue;
            };
            if sources.insert(image.id, source_id.clone()).is_some() {
                self.invalid_resource("image", "duplicate runtime identity was present", Some(&label));
                continue;
            }
            let mut observation = ImageObservation::new(source_id.clone());
            if let Some(user) = image.user.filter(|value| !value.is_empty()) {
                observation.set_user(user);
            }
            self.report_fields(
                format!("runtime.podman.images.{label}"),
                meaningful_fields(
                    &image.other,
                    &[
                        "Digest",
                        "RepoTags",
                        "RepoDigests",
                        "Parent",
                        "Comment",
                        "Created",
                        "Version",
                        "Author",
                        "Architecture",
                        "Os",
                        "Size",
                        "VirtualSize",
                        "GraphDriver",
                        "RootFS",
                        "NamesHistory",
                        "ManifestType",
                    ],
                ),
                Some(&source_id),
            );
            if let Some(config) = image.config {
                if let Some(command) = config.cmd {
                    observation.set_command(effective_command(command));
                }
                if let Some(environment) = config.env {
                    if let Some(environment) = self.environment("image", &label, environment, Some(&source_id)) {
                        observation.set_environment(environment);
                    }
                }
                if let Some(labels) = self.metadata_labels("image", &label, config.labels.unwrap_or_default()) {
                    observation.set_labels(labels);
                }
                if let Some(user) = config.user.filter(|value| !value.is_empty()) {
                    observation.set_user(user);
                }
                if let Some(working_directory) = config.working_dir.filter(|value| !value.is_empty()) {
                    observation.set_working_directory(working_directory);
                }
                if let Some(healthcheck) = self.healthcheck("image", &label, &source_id, config.healthcheck) {
                    observation.set_healthcheck(healthcheck);
                }
                self.report_fields(
                    format!("runtime.podman.images.{label}.Config"),
                    meaningful_fields(&config.other, &[]),
                    Some(&source_id),
                );
            }
            self.add_image(snapshot, observation, &label);
        }
        sources
    }

    fn map_networks(&mut self, snapshot: &mut RuntimeSnapshot, networks: Vec<NetworkInspect>) {
        for network in networks {
            let Ok(name) = Identifier::new(network.name.clone()) else {
                self.invalid_resource("network", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            let Ok(source_id) = named_source("network", &network.name) else {
                self.invalid_resource(
                    "network",
                    "could not construct a stable source identity",
                    Some(&network.name),
                );
                continue;
            };
            self.report_fields(
                format!("runtime.podman.networks.{}", network.name),
                meaningful_fields(&network.other, &["id", "created"]),
                Some(&source_id),
            );
            self.add_network(snapshot, NetworkObservation::new(source_id, name), &network.name);
        }
    }

    fn map_volumes(&mut self, snapshot: &mut RuntimeSnapshot, volumes: Vec<VolumeInspect>) {
        for volume in volumes {
            let Ok(name) = Identifier::new(volume.name.clone()) else {
                self.invalid_resource("volume", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            let Ok(source_id) = named_source("volume", &volume.name) else {
                self.invalid_resource(
                    "volume",
                    "could not construct a stable source identity",
                    Some(&volume.name),
                );
                continue;
            };
            self.report_fields(
                format!("runtime.podman.volumes.{}", volume.name),
                meaningful_fields(&volume.other, &["Mountpoint", "CreatedAt"]),
                Some(&source_id),
            );
            self.add_volume(snapshot, VolumeObservation::new(source_id, name), &volume.name);
        }
    }

    fn map_pods(
        &mut self,
        snapshot: &mut RuntimeSnapshot,
        pods: Vec<PodInspect>,
        pod_sources: &BTreeMap<String, SourceId>,
        container_sources: &BTreeMap<String, SourceId>,
    ) {
        for pod in pods {
            let Some(source_id) = pod_sources.get(&pod.id).cloned() else {
                continue;
            };
            let Ok(name) = Identifier::new(pod.name.clone()) else {
                continue;
            };
            let mut observation = PodObservation::new(source_id.clone(), name);
            for member in pod.containers {
                if let Some(member_source) = container_sources.get(&member.id) {
                    observation.add_member(member_source.clone());
                } else {
                    self.missing_relationship("pod member", &pod.name, Some(&source_id));
                }
            }
            if let Some(arguments) = pod.create_command.filter(|arguments| !arguments.is_empty()) {
                if let Ok(evidence_source) = named_source("create:pod", &pod.name) {
                    observation.set_creation_evidence(CreationEvidence::new(evidence_source, arguments));
                }
            }
            self.report_fields(
                format!("runtime.podman.pods.{}", pod.name),
                meaningful_fields(
                    &pod.other,
                    &[
                        "Created",
                        "State",
                        "CgroupPath",
                        "NumContainers",
                        "Namespace",
                        "LockNumber",
                    ],
                ),
                Some(&source_id),
            );
            self.add_pod(snapshot, observation, &pod.name);
        }
    }

    fn map_containers(
        &mut self,
        snapshot: &mut RuntimeSnapshot,
        containers: Vec<ContainerInspect>,
        container_sources: &BTreeMap<String, SourceId>,
        pod_sources: &BTreeMap<String, SourceId>,
        image_sources: &BTreeMap<String, SourceId>,
    ) {
        for mut container in containers {
            let Some(source_id) = container_sources.get(&container.id).cloned() else {
                continue;
            };
            let Ok(name) = Identifier::new(container.name.clone()) else {
                continue;
            };
            let mut observation = ContainerObservation::new(source_id.clone(), name);
            let config = container.config;
            let image_name = container.image_name.filter(|value| !value.is_empty()).or_else(|| {
                config
                    .as_ref()
                    .and_then(|value| value.image.clone())
                    .filter(|value| !value.is_empty())
            });
            if let Some(image_name) = image_name {
                match ImageReference::parse(image_name) {
                    Ok(reference) => observation.set_image(reference, image_sources.get(&container.image).cloned()),
                    Err(_) => self.invalid_resource(
                        "container image",
                        "image reference is structurally invalid",
                        Some(&container.name),
                    ),
                }
            }
            if let Some(config) = config {
                self.map_container_config(&mut observation, &container.name, &source_id, config);
            }
            self.map_host_config(
                &mut observation,
                &container.name,
                &source_id,
                &mut container.host_config,
            );
            let mut container_fields = Vec::new();
            if container.is_service {
                container_fields.push("IsService".to_owned());
            }
            if !container.dependencies.is_empty() {
                container_fields.push("Dependencies".to_owned());
            }
            self.report_fields(
                format!("runtime.podman.containers.{}", container.name),
                container_fields,
                Some(&source_id),
            );
            for (index, mount) in container.mounts.into_iter().enumerate() {
                if let Some(mapped) = self.mount(&container.name, index, mount, &source_id) {
                    observation.add_mount(mapped);
                }
            }
            if let Some(settings) = container.network_settings {
                self.ports(&container.name, &mut observation, settings.ports);
                for (network_name, network) in settings.networks {
                    self.report_fields(
                        format!("runtime.podman.containers.{}.Networks.{network_name}", container.name),
                        meaningful_fields(&network.other, &["EndpointID", "NetworkID"]),
                        Some(&source_id),
                    );
                    match Identifier::new(network_name.clone()) {
                        Ok(network_name) => {
                            observation.add_network(NetworkAttachment::new(network_name, network.aliases));
                        }
                        Err(_) => self.invalid_resource(
                            "container network",
                            "network name is empty or contains a NUL byte",
                            Some(&container.name),
                        ),
                    }
                }
            }
            if let Some(pod_id) = container.pod.filter(|value| !value.is_empty()) {
                if let Some(pod_source) = pod_sources.get(&pod_id) {
                    observation.set_pod_source_id(pod_source.clone());
                } else {
                    self.missing_relationship("container pod", &container.name, Some(&source_id));
                }
            }
            self.add_container(snapshot, observation, &container.name);
        }
    }

    fn map_container_config(
        &mut self,
        observation: &mut ContainerObservation,
        container_name: &str,
        source_id: &SourceId,
        config: crate::native::ContainerConfig,
    ) {
        if let Some(command) = config.cmd {
            observation.set_command(effective_command(command));
        }
        if let Some(environment) = config.env {
            if let Some(environment) = self.environment("container", container_name, environment, Some(source_id)) {
                observation.set_environment(environment);
            }
        }
        if let Some(labels) = self.metadata_labels("container", container_name, config.labels.unwrap_or_default()) {
            observation.set_labels(labels);
        }
        if let Some(user) = config.user.filter(|value| !value.is_empty()) {
            observation.set_user(user);
        }
        if let Some(working_directory) = config.working_dir.filter(|value| !value.is_empty()) {
            observation.set_working_directory(working_directory);
        }
        if let Some(healthcheck) = self.healthcheck("container", container_name, source_id, config.healthcheck) {
            observation.set_healthcheck(healthcheck);
        }
        if let Some(arguments) = config.create_command.filter(|arguments| !arguments.is_empty()) {
            if let Ok(evidence_source) = named_source("create:container", container_name) {
                observation.set_creation_evidence(CreationEvidence::new(evidence_source, arguments));
            }
        }
        self.report_fields(
            format!("runtime.podman.containers.{container_name}.Config"),
            meaningful_fields(&config.other, &[]),
            Some(source_id),
        );
    }

    fn map_host_config(
        &mut self,
        observation: &mut ContainerObservation,
        container_name: &str,
        source_id: &SourceId,
        host_config: &mut BTreeMap<String, Value>,
    ) {
        match host_config.remove("ReadonlyRootfs") {
            Some(Value::Bool(read_only)) => observation.set_read_only_root_filesystem(read_only),
            Some(Value::Null) | None => {}
            Some(_) => self.invalid_resource(
                "container host configuration",
                "ReadonlyRootfs is not a boolean",
                Some(container_name),
            ),
        }
        match host_config.remove("RestartPolicy") {
            Some(Value::Null) | None => {}
            Some(value) => match serde_json::from_value::<NativeRestartPolicy>(value)
                .map_err(|_| "RestartPolicy is not an object with the expected fields")
                .and_then(|native| decode_restart_policy(&native, true))
            {
                Ok(policy) => observation.set_restart_policy(policy),
                Err(reason) => self.invalid_resource("container restart policy", reason, Some(container_name)),
            },
        }
        self.report_fields(
            format!("runtime.podman.containers.{container_name}.HostConfig"),
            meaningful_fields(host_config, &[]),
            Some(source_id),
        );
    }

    fn environment(
        &mut self,
        kind: &'static str,
        label: &str,
        values: Vec<String>,
        source_id: Option<&SourceId>,
    ) -> Option<Vec<RuntimeEnvironmentVariable>> {
        let mut environment = Vec::with_capacity(values.len());
        for value in values {
            let Some((name, value)) = value.split_once('=') else {
                self.invalid_resource(kind, "environment entry has no equals separator", Some(label));
                return None;
            };
            let Ok(name) = Identifier::new(name) else {
                self.invalid_resource(kind, "environment name is empty or contains a NUL byte", Some(label));
                return None;
            };
            environment.push(RuntimeEnvironmentVariable::new(name, value));
        }
        if source_id.is_none() {
            self.invalid_resource(kind, "environment has no stable source identity", Some(label));
            None
        } else {
            Some(environment)
        }
    }

    fn metadata_labels(
        &mut self,
        kind: &'static str,
        resource: &str,
        values: BTreeMap<String, String>,
    ) -> Option<Vec<RuntimeMetadataLabel>> {
        let mut labels = Vec::with_capacity(values.len());
        for (name, value) in values {
            let Ok(name) = Identifier::new(name) else {
                self.invalid_resource(
                    kind,
                    "metadata-label name is empty or contains a NUL byte",
                    Some(resource),
                );
                return None;
            };
            labels.push(RuntimeMetadataLabel::new(name, value));
        }
        Some(labels)
    }

    fn healthcheck(
        &mut self,
        kind: &'static str,
        label: &str,
        source_id: &SourceId,
        native: Option<HealthConfig>,
    ) -> Option<RuntimeHealthcheck> {
        let Some(native) = native else {
            return Some(RuntimeHealthcheck::new());
        };
        self.report_fields(
            format!("runtime.podman.{kind}s.{label}.Config.Healthcheck"),
            meaningful_fields(&native.other, &[]),
            Some(source_id),
        );
        match decode_healthcheck(native) {
            Ok(healthcheck) => Some(healthcheck),
            Err(reason) => {
                self.invalid_resource("health check", reason, Some(label));
                None
            }
        }
    }

    fn mount(&mut self, container: &str, index: usize, native: InspectMount, source_id: &SourceId) -> Option<Mount> {
        let source = match native.kind.as_str() {
            "volume" if !native.name.is_empty() => {
                let Ok(name) = Identifier::new(native.name) else {
                    self.invalid_resource("container mount", "volume name is invalid", Some(container));
                    return None;
                };
                MountSource::Volume(name)
            }
            "bind" if !native.source.is_empty() => MountSource::HostPath(native.source),
            "volume" | "bind" => {
                self.invalid_resource("container mount", "mount source is empty", Some(container));
                return None;
            }
            _ => {
                self.loss(
                    self.codes.unmodeled_configuration.clone(),
                    format!("runtime.podman.containers.{container}.Mounts[{index}]"),
                    ConversionKind::Unsupported,
                    "Podman mount type has no runtime-neutral representation",
                    vec![
                        ("resource", container.to_owned()),
                        ("field", "Type".to_owned()),
                        (
                            "action",
                            "recreate this mount manually in the generated definition".to_owned(),
                        ),
                    ],
                    Some(source_id.clone()),
                );
                return None;
            }
        };
        let Ok(mut mount) = Mount::new(source, native.destination, !native.rw) else {
            self.invalid_resource("container mount", "mount destination is invalid", Some(container));
            return None;
        };
        let options = native
            .mode
            .split(',')
            .chain(native.options.iter().map(String::as_str))
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        let shared = options.contains("z");
        let private = options.contains("Z");
        match (shared, private) {
            (true, false) => mount.set_selinux_relabel(SelinuxRelabel::Shared),
            (false, true) => mount.set_selinux_relabel(SelinuxRelabel::Private),
            (true, true) => {
                self.invalid_resource(
                    "container mount",
                    "mount requests both SELinux relabel modes",
                    Some(container),
                );
                return None;
            }
            (false, false) => {}
        }
        let unknown = options
            .into_iter()
            .filter(|option| !matches!(*option, "rw" | "ro" | "z" | "Z"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut fields = unknown;
        if !native.propagation.is_empty() {
            fields.push("Propagation".to_owned());
        }
        if !native.driver.is_empty() {
            fields.push("Driver".to_owned());
        }
        if !native.sub_path.is_empty() {
            fields.push("SubPath".to_owned());
        }
        self.report_fields(
            format!("runtime.podman.containers.{container}.Mounts[{index}]"),
            fields,
            Some(source_id),
        );
        Some(mount)
    }

    fn ports(
        &mut self,
        container: &str,
        observation: &mut ContainerObservation,
        ports: BTreeMap<String, Option<Vec<crate::native::PortBinding>>>,
    ) {
        for (key, bindings) in ports {
            let Some((container_port, protocol)) = key.split_once('/') else {
                self.invalid_resource("container port", "port key has no protocol separator", Some(container));
                continue;
            };
            let Ok(container_port) = container_port.parse::<u16>() else {
                self.invalid_resource(
                    "container port",
                    "container port is not a non-zero 16-bit integer",
                    Some(container),
                );
                continue;
            };
            let protocol = protocol_value(protocol);
            let bindings = bindings.unwrap_or_default();
            if bindings.is_empty() {
                self.add_port(observation, container, container_port, None, None, protocol.clone());
            }
            for binding in bindings {
                let Ok(host_port) = binding.host_port.parse::<u16>() else {
                    self.invalid_resource(
                        "container port",
                        "host port is not a non-zero 16-bit integer",
                        Some(container),
                    );
                    continue;
                };
                let host_address = (!binding.host_ip.is_empty()).then_some(binding.host_ip);
                self.add_port(
                    observation,
                    container,
                    container_port,
                    Some(host_port),
                    host_address,
                    protocol.clone(),
                );
            }
        }
    }

    fn add_port(
        &mut self,
        observation: &mut ContainerObservation,
        container: &str,
        container_port: u16,
        host_port: Option<u16>,
        host_address: Option<String>,
        protocol: Protocol,
    ) {
        match Port::new(container_port, host_port, host_address, protocol) {
            Ok(port) => observation.add_port(port),
            Err(_) => self.invalid_resource("container port", "container port is zero", Some(container)),
        }
    }

    fn report_fields(&mut self, subject: String, fields: Vec<String>, source_id: Option<&SourceId>) {
        if fields.is_empty() {
            return;
        }
        let mut field_names = String::new();
        for field in fields {
            if !field_names.is_empty() {
                field_names.push(',');
            }
            field_names.push_str(&field);
        }
        self.loss(
            self.codes.unmodeled_configuration.clone(),
            subject,
            ConversionKind::Unsupported,
            "Podman inspection contains reusable configuration not represented by this adapter",
            vec![
                ("fields", field_names),
                ("action", "review and recreate these named fields manually".to_owned()),
            ],
            source_id.cloned(),
        );
    }

    fn missing_relationship(&mut self, kind: &'static str, resource: &str, source_id: Option<&SourceId>) {
        self.loss(
            self.codes.missing_relationship.clone(),
            format!("runtime.podman.relationships.{resource}"),
            ConversionKind::Unsupported,
            "Podman inspection references a resource absent from the supplied document set",
            vec![
                ("relationship", kind.to_owned()),
                ("resource", resource.to_owned()),
                (
                    "action",
                    "supply a complete related inspect set or recreate the relationship manually".to_owned(),
                ),
            ],
            source_id.cloned(),
        );
    }

    fn invalid_resource(&mut self, kind: &'static str, reason: &'static str, label: Option<&str>) {
        let mut fields = vec![
            ("resource_kind", kind.to_owned()),
            ("reason", reason.to_owned()),
            ("action", "correct or recapture the Podman inspect document".to_owned()),
        ];
        if let Some(label) = label {
            fields.push(("resource", label.to_owned()));
        }
        self.loss(
            self.codes.invalid_data.clone(),
            format!("runtime.podman.{kind}"),
            ConversionKind::Invalid,
            "Podman inspection contains an invalid reusable value",
            fields,
            None,
        );
        self.invalid = true;
    }

    fn add_container(&mut self, snapshot: &mut RuntimeSnapshot, value: ContainerObservation, label: &str) {
        if let Err(error) = snapshot.add_container(value) {
            self.snapshot_error("container", label, &error);
        }
    }

    fn add_image(&mut self, snapshot: &mut RuntimeSnapshot, value: ImageObservation, label: &str) {
        if let Err(error) = snapshot.add_image(value) {
            self.snapshot_error("image", label, &error);
        }
    }

    fn add_network(&mut self, snapshot: &mut RuntimeSnapshot, value: NetworkObservation, label: &str) {
        if let Err(error) = snapshot.add_network(value) {
            self.snapshot_error("network", label, &error);
        }
    }

    fn add_volume(&mut self, snapshot: &mut RuntimeSnapshot, value: VolumeObservation, label: &str) {
        if let Err(error) = snapshot.add_volume(value) {
            self.snapshot_error("volume", label, &error);
        }
    }

    fn add_pod(&mut self, snapshot: &mut RuntimeSnapshot, value: PodObservation, label: &str) {
        if let Err(error) = snapshot.add_pod(value) {
            self.snapshot_error("pod", label, &error);
        }
    }

    fn snapshot_error(&mut self, kind: &'static str, label: &str, error: &RuntimeSnapshotError) {
        let reason = match error {
            RuntimeSnapshotError::DuplicateSourceIdentity { .. } => {
                "stable source identity collides with another resource"
            }
            RuntimeSnapshotError::DuplicateResource { .. } => "resource name is duplicated",
            _ => "runtime snapshot rejected the resource",
        };
        self.invalid_resource(kind, reason, Some(label));
    }

    fn loss(
        &mut self,
        code: DiagnosticCode,
        subject: String,
        kind: ConversionKind,
        summary: &'static str,
        fields: Vec<(&'static str, String)>,
        source_id: Option<SourceId>,
    ) {
        let mut diagnostic = Diagnostic::new(code.clone(), severity(kind), summary);
        for (name, value) in fields {
            diagnostic = diagnostic.with_field(DiagnosticField::new(name, DiagnosticValue::plain(value)));
        }
        if let Ok(mut outcome) = ConversionOutcome::loss(subject, kind, code) {
            if let Some(source_id) = source_id {
                outcome = outcome.with_origin(Provenance::runtime_observation(source_id));
            }
            self.outcomes.push(outcome);
            self.diagnostics.push(diagnostic);
        }
    }
}

fn supported_version(version: PlatformVersion) -> bool {
    version >= MINIMUM_PODMAN_VERSION && version <= MAXIMUM_PODMAN_VERSION
}

fn effective_command(arguments: Vec<String>) -> EffectiveCommand {
    if arguments.is_empty() {
        EffectiveCommand::Empty
    } else {
        EffectiveCommand::exec(arguments)
    }
}

fn decode_restart_policy(
    native: &NativeRestartPolicy,
    allow_podman_extensions: bool,
) -> Result<RestartPolicy, &'static str> {
    let maximum_retry_count =
        u64::try_from(native.maximum_retry_count).map_err(|_| "restart-policy maximum retry count is negative")?;
    let maximum_retries = std::num::NonZeroU64::new(maximum_retry_count);
    match native.name.as_str() {
        "no" if maximum_retries.is_none() => Ok(RestartPolicy::Never),
        "" | "never" if allow_podman_extensions && maximum_retries.is_none() => Ok(RestartPolicy::Never),
        "always" if maximum_retries.is_none() => Ok(RestartPolicy::Always),
        "unless-stopped" if maximum_retries.is_none() => Ok(RestartPolicy::UnlessStopped),
        "on-failure" => Ok(RestartPolicy::on_failure(maximum_retries)),
        "no" | "always" | "unless-stopped" => Err("restart-policy maximum retries are only valid with on-failure"),
        "" | "never" if allow_podman_extensions => Err("restart-policy maximum retries are only valid with on-failure"),
        _ => Err("restart-policy name is unknown"),
    }
}

fn decode_healthcheck(native: HealthConfig) -> Result<RuntimeHealthcheck, &'static str> {
    let scalars_are_configured = health_scalars_are_configured(&native);
    let mut healthcheck = RuntimeHealthcheck::new();
    match native.test {
        None => {
            if scalars_are_configured {
                return Err("health-check timing is present without a command");
            }
        }
        Some(test) if test.is_empty() => return Err("health-check command array is empty"),
        Some(test) => {
            let mut values = test.into_iter();
            match values.next().as_deref() {
                Some("NONE") if values.next().is_none() => healthcheck.set_disabled(true),
                Some("CMD") => {
                    let arguments = values.map(ProtectedString::sensitive).collect::<Vec<ProtectedString>>();
                    if arguments.is_empty() {
                        return Err("health-check exec command has no arguments");
                    }
                    healthcheck.set_disabled(false);
                    healthcheck.set_command(HealthcheckCommand::Exec(arguments));
                }
                Some("CMD-SHELL") => {
                    let Some(command) = values.next() else {
                        return Err("health-check shell command is absent");
                    };
                    if command.is_empty() || values.next().is_some() {
                        return Err("health-check shell command must contain exactly one non-empty value");
                    }
                    healthcheck.set_disabled(false);
                    healthcheck.set_command(HealthcheckCommand::Shell(ProtectedString::sensitive(command)));
                }
                Some("NONE") => return Err("disabled health check contains unexpected arguments"),
                Some(_) => return Err("health-check command kind is unknown"),
                None => return Err("health-check command array is empty"),
            }
        }
    }

    set_duration(&mut healthcheck, native.interval, RuntimeHealthcheck::set_interval)?;
    set_duration(&mut healthcheck, native.timeout, RuntimeHealthcheck::set_timeout)?;
    set_retries(&mut healthcheck, native.retries)?;
    set_duration(
        &mut healthcheck,
        native.start_period,
        RuntimeHealthcheck::set_start_period,
    )?;
    Ok(healthcheck)
}

fn health_scalars_are_configured(native: &HealthConfig) -> bool {
    [native.interval, native.timeout, native.retries, native.start_period]
        .into_iter()
        .flatten()
        .any(|value| value != 0)
}

fn set_duration(
    healthcheck: &mut RuntimeHealthcheck,
    value: Option<i64>,
    set: fn(&mut RuntimeHealthcheck, HealthcheckDuration),
) -> Result<(), &'static str> {
    let Some(value) = value else {
        return Ok(());
    };
    if value < 0 {
        return Err("health-check duration is negative");
    }
    if value > 0 {
        let duration = HealthcheckDuration::new(canonical_duration(value))
            .map_err(|_| "health-check duration could not enter the neutral model")?;
        set(healthcheck, duration);
    }
    Ok(())
}

fn set_retries(healthcheck: &mut RuntimeHealthcheck, value: Option<i64>) -> Result<(), &'static str> {
    let Some(value) = value else {
        return Ok(());
    };
    if value < 0 {
        return Err("health-check retry count is negative");
    }
    if value > 0 {
        let retries = HealthcheckRetries::new(value.to_string())
            .map_err(|_| "health-check retry count could not enter the neutral model")?;
        healthcheck.set_retries(retries);
    }
    Ok(())
}

fn canonical_duration(nanoseconds: i64) -> String {
    for (unit_nanoseconds, suffix) in [
        (3_600_000_000_000, "h"),
        (60_000_000_000, "m"),
        (1_000_000_000, "s"),
        (1_000_000, "ms"),
        (1_000, "us"),
    ] {
        if nanoseconds % unit_nanoseconds == 0 {
            return format!("{}{suffix}", nanoseconds / unit_nanoseconds);
        }
    }
    format!("{nanoseconds}ns")
}

fn protocol_value(protocol: &str) -> Protocol {
    match protocol {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        "sctp" => Protocol::Sctp,
        other => Protocol::Other(other.to_owned()),
    }
}

fn named_source(kind: &str, label: &str) -> Result<SourceId, ModelError> {
    SourceId::new(format!("runtime:podman:{kind}:{label}"))
}

fn document_origin(name: &str) -> Option<SourceId> {
    named_source("document", name).ok()
}

fn meaningful_fields(values: &BTreeMap<String, Value>, ignored: &[&str]) -> Vec<String> {
    values
        .iter()
        .filter(|(name, value)| !ignored.contains(&name.as_str()) && meaningful(value))
        .map(|(name, _)| name.clone())
        .collect()
}

fn meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_i64() != Some(0) && value.as_u64() != Some(0),
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => values.iter().any(meaningful),
        Value::Object(values) => values.values().any(meaningful),
    }
}

const fn severity(kind: ConversionKind) -> Severity {
    match kind {
        ConversionKind::Invalid => Severity::Error,
        ConversionKind::Exact => Severity::Note,
        _ => Severity::Warning,
    }
}
