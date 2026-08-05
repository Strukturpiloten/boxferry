//! Docker inspect decoding and runtime reconstruction composition.

use std::collections::{BTreeMap, BTreeSet};

use boxferry_engine::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, ImportAdapter,
    ImportResult, InvalidDiagnosticCode, Severity,
};
use boxferry_model::{
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, Identifier, ImageReference, ModelError, Mount,
    MountSource, NetworkAttachment, Port, ProtectedString, Protocol, Provenance, RestartPolicy, SelinuxRelabel,
    SourceId,
};
use boxferry_runtime::{
    ContainerObservation, EffectiveCommand, ImageObservation, NetworkObservation, OverrideReconstruction,
    RuntimeEnvironmentVariable, RuntimeHealthcheck, RuntimeImplementation, RuntimeImporter, RuntimeMetadataLabel,
    RuntimeResolutions, RuntimeSnapshot, RuntimeSnapshotError, VolumeObservation,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    DockerInspectSource, MAXIMUM_DOCKER_API_VERSION, MINIMUM_DOCKER_API_VERSION,
    native::{
        ContainerInspect, HealthConfig, ImageConfig, ImageInspect, InspectMount, NetworkInspect,
        RestartPolicy as NativeRestartPolicy, VolumeInspect,
    },
};

/// Recoverable output of decoding native Docker inspection responses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerSnapshotResult {
    snapshot: Option<RuntimeSnapshot>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl DockerSnapshotResult {
    /// Returns the decoded snapshot when the supplied native data is structurally usable.
    #[must_use]
    pub const fn snapshot(&self) -> Option<&RuntimeSnapshot> {
        self.snapshot.as_ref()
    }

    /// Returns Docker-native decoding decisions.
    #[must_use]
    pub fn outcomes(&self) -> &[ConversionOutcome] {
        &self.outcomes
    }

    /// Returns structured Docker-native diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Decomposes the result for composition with another importer.
    #[must_use]
    pub fn into_parts(self) -> (Option<RuntimeSnapshot>, Vec<ConversionOutcome>, Vec<Diagnostic>) {
        (self.snapshot, self.outcomes, self.diagnostics)
    }
}

/// Pure importer from caller-supplied Docker inspect JSON into the neutral application model.
#[derive(Clone, Debug)]
pub struct DockerImporter {
    runtime: RuntimeImporter,
    codes: Codes,
}

impl DockerImporter {
    /// Creates an importer with an explicit effective-value reconstruction policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] only if an embedded `BoxFerry` code is invalid.
    pub fn new(override_reconstruction: OverrideReconstruction) -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            runtime: RuntimeImporter::new(override_reconstruction)?,
            codes: Codes {
                invalid_data: DiagnosticCode::new("BFD0001")?,
                unmodeled_configuration: DiagnosticCode::new("BFD0002")?,
                unsupported_version: DiagnosticCode::new("BFD0003")?,
                missing_relationship: DiagnosticCode::new("BFD0004")?,
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
    pub fn decode(&self, source: &DockerInspectSource) -> DockerSnapshotResult {
        Decoder::new(&self.codes, source).decode()
    }
}

impl ImportAdapter for DockerImporter {
    type Source = DockerInspectSource;

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

struct Relationships<'a> {
    image_sources: &'a BTreeMap<String, SourceId>,
    network_names: &'a BTreeSet<String>,
    volume_names: &'a BTreeSet<String>,
}

struct Decoder<'a> {
    codes: &'a Codes,
    source: &'a DockerInspectSource,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
    invalid: bool,
}

impl<'a> Decoder<'a> {
    const fn new(codes: &'a Codes, source: &'a DockerInspectSource) -> Self {
        Self {
            codes,
            source,
            outcomes: Vec::new(),
            diagnostics: Vec::new(),
            invalid: false,
        }
    }

    fn decode(mut self) -> DockerSnapshotResult {
        if self.source.api_version() < MINIMUM_DOCKER_API_VERSION
            || self.source.api_version() > MAXIMUM_DOCKER_API_VERSION
        {
            self.loss(
                self.codes.unsupported_version.clone(),
                "runtime.docker.api-version".to_owned(),
                ConversionKind::Invalid,
                "Docker Engine API version is outside the reviewed decoder range",
                vec![
                    ("version", self.source.api_version().to_string()),
                    ("minimum", MINIMUM_DOCKER_API_VERSION.to_string()),
                    ("maximum", MAXIMUM_DOCKER_API_VERSION.to_string()),
                    (
                        "action",
                        "supply data from a covered Engine API version or add reviewed fixture evidence".to_owned(),
                    ),
                ],
                None,
            );
            return self.finish(None);
        }

        let containers = self.parse_document::<ContainerInspect>("containers", self.source.documents().containers());
        let images = self.parse_document::<ImageInspect>("images", self.source.documents().images());
        let networks = self.parse_document::<NetworkInspect>("networks", self.source.documents().networks());
        let volumes = self.parse_document::<VolumeInspect>("volumes", self.source.documents().volumes());
        let (Some(containers), Some(images), Some(networks), Some(volumes)) = (containers, images, networks, volumes)
        else {
            return self.finish(None);
        };

        let mut snapshot = RuntimeSnapshot::new(self.source.application_name().clone(), RuntimeImplementation::Docker);
        let container_sources = self.container_sources(&containers);
        let image_sources = self.map_images(&mut snapshot, images);
        let network_names = self.map_networks(&mut snapshot, networks);
        let volume_names = self.map_volumes(&mut snapshot, volumes);
        self.map_containers(
            &mut snapshot,
            containers,
            &container_sources,
            &image_sources,
            &network_names,
            &volume_names,
        );

        if self.invalid {
            self.finish(None)
        } else {
            self.finish(Some(snapshot))
        }
    }

    fn finish(self, snapshot: Option<RuntimeSnapshot>) -> DockerSnapshotResult {
        DockerSnapshotResult {
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
                    format!("runtime.docker.documents.{name}"),
                    ConversionKind::Invalid,
                    "Docker inspect document is not a valid expected JSON array",
                    vec![
                        ("document", name.to_owned()),
                        ("line", error.line().to_string()),
                        ("column", error.column().to_string()),
                        (
                            "action",
                            "capture the complete JSON array from the matching Docker inspect command".to_owned(),
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
            let Some(name) = container_name(&container.name) else {
                self.invalid_resource("container", "resource name is empty after Docker normalization", None);
                continue;
            };
            let Ok(source_id) = named_source("container", name) else {
                self.invalid_resource("container", "resource name contains a NUL byte", None);
                continue;
            };
            if container.id.is_empty() {
                self.invalid_resource("container", "runtime identity is empty", Some(name));
                continue;
            }
            if sources.contains_key(&container.id) {
                self.invalid_resource("container", "duplicate runtime identity was present", Some(name));
                continue;
            }
            sources.insert(container.id.clone(), source_id);
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
            if image.id.is_empty() {
                self.invalid_resource("image", "runtime identity is empty", Some(&label));
                continue;
            }
            if sources.contains_key(&image.id) {
                self.invalid_resource("image", "duplicate runtime identity was present", Some(&label));
                continue;
            }
            sources.insert(image.id, source_id.clone());
            let mut observation = ImageObservation::new(source_id.clone());
            self.report_fields(
                format!("runtime.docker.images.{label}"),
                meaningful_fields(
                    &image.other,
                    &[
                        "Id",
                        "RepoTags",
                        "RepoDigests",
                        "Parent",
                        "Comment",
                        "Created",
                        "DockerVersion",
                        "Author",
                        "Architecture",
                        "Variant",
                        "Os",
                        "OsVersion",
                        "Size",
                        "VirtualSize",
                        "GraphDriver",
                        "RootFS",
                        "Metadata",
                        "Descriptor",
                        "Identity",
                    ],
                ),
                Some(&source_id),
            );
            if let Some(config) = image.config {
                if let Some(command) = combined_image_command(&config) {
                    observation.set_command(command);
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
                let mut fields = meaningful_fields(&config.other, &[]);
                if config.entrypoint.as_ref().is_some_and(|values| !values.is_empty()) {
                    fields.push("EntrypointBoundary".to_owned());
                }
                if config.cmd.as_ref().is_some_and(|values| !values.is_empty()) {
                    fields.push("CommandBoundary".to_owned());
                }
                self.report_fields(
                    format!("runtime.docker.images.{label}.Config"),
                    fields,
                    Some(&source_id),
                );
            }
            self.add_image(snapshot, observation, &label);
        }
        sources
    }

    fn map_networks(&mut self, snapshot: &mut RuntimeSnapshot, networks: Vec<NetworkInspect>) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for network in networks {
            let Ok(name) = Identifier::new(network.name.clone()) else {
                self.invalid_resource("network", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            if !names.insert(network.name.clone()) {
                self.invalid_resource("network", "resource name is duplicated", Some(&network.name));
                continue;
            }
            let Ok(source_id) = named_source("network", &network.name) else {
                self.invalid_resource(
                    "network",
                    "could not construct a stable source identity",
                    Some(&network.name),
                );
                continue;
            };
            self.report_fields(
                format!("runtime.docker.networks.{}", network.name),
                meaningful_fields(
                    &network.other,
                    &[
                        "Id",
                        "Created",
                        "Scope",
                        "EnableIPv4",
                        "EnableIPv6",
                        "IPAM",
                        "Internal",
                        "Attachable",
                        "Ingress",
                        "ConfigFrom",
                        "ConfigOnly",
                        "Containers",
                        "Status",
                        "Peers",
                    ],
                ),
                Some(&source_id),
            );
            self.add_network(snapshot, NetworkObservation::new(source_id, name), &network.name);
        }
        names
    }

    fn map_volumes(&mut self, snapshot: &mut RuntimeSnapshot, volumes: Vec<VolumeInspect>) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for volume in volumes {
            let Ok(name) = Identifier::new(volume.name.clone()) else {
                self.invalid_resource("volume", "resource name is empty or contains a NUL byte", None);
                continue;
            };
            if !names.insert(volume.name.clone()) {
                self.invalid_resource("volume", "resource name is duplicated", Some(&volume.name));
                continue;
            }
            let Ok(source_id) = named_source("volume", &volume.name) else {
                self.invalid_resource(
                    "volume",
                    "could not construct a stable source identity",
                    Some(&volume.name),
                );
                continue;
            };
            self.report_fields(
                format!("runtime.docker.volumes.{}", volume.name),
                meaningful_fields(&volume.other, &["Mountpoint", "CreatedAt", "Status", "UsageData"]),
                Some(&source_id),
            );
            self.add_volume(snapshot, VolumeObservation::new(source_id, name), &volume.name);
        }
        names
    }

    fn map_containers(
        &mut self,
        snapshot: &mut RuntimeSnapshot,
        containers: Vec<ContainerInspect>,
        container_sources: &BTreeMap<String, SourceId>,
        image_sources: &BTreeMap<String, SourceId>,
        network_names: &BTreeSet<String>,
        volume_names: &BTreeSet<String>,
    ) {
        let relationships = Relationships {
            image_sources,
            network_names,
            volume_names,
        };
        for container in containers {
            let Some(source_id) = container_sources.get(&container.id).cloned() else {
                continue;
            };
            let Some(container_name) = container_name(&container.name).map(str::to_owned) else {
                continue;
            };
            self.map_container(snapshot, container, &container_name, &source_id, &relationships);
        }
    }

    fn map_container(
        &mut self,
        snapshot: &mut RuntimeSnapshot,
        mut container: ContainerInspect,
        container_name: &str,
        source_id: &SourceId,
        relationships: &Relationships<'_>,
    ) {
        let Ok(name) = Identifier::new(container_name) else {
            return;
        };
        let mut observation = ContainerObservation::new(source_id.clone(), name);
        self.configure_container(
            &mut observation,
            container_name,
            source_id,
            &mut container,
            relationships.image_sources,
        );
        self.map_host_config(&mut observation, container_name, source_id, &mut container.host_config);
        self.report_container_fields(container_name, source_id, &container);
        for (index, mount) in container.mounts.into_iter().enumerate() {
            if mount.kind == "volume" && !mount.name.is_empty() && !relationships.volume_names.contains(&mount.name) {
                self.missing_relationship("volume", &mount.name, Some(source_id));
            }
            if let Some(mapped) = self.mount(container_name, index, mount, source_id) {
                observation.add_mount(mapped);
            }
        }
        if let Some(settings) = container.network_settings {
            self.map_network_settings(
                container_name,
                &container.id,
                source_id,
                &mut observation,
                settings,
                relationships.network_names,
            );
        }
        self.add_container(snapshot, observation, container_name);
    }

    fn configure_container(
        &mut self,
        observation: &mut ContainerObservation,
        container_name: &str,
        source_id: &SourceId,
        container: &mut ContainerInspect,
        image_sources: &BTreeMap<String, SourceId>,
    ) {
        let Some(config) = container.config.take() else {
            self.invalid_resource("container image", "container Config is absent", Some(container_name));
            return;
        };
        let Some(image_name) = config.image.clone().filter(|value| !value.is_empty()) else {
            self.invalid_resource(
                "container image",
                "container Config.Image is absent",
                Some(container_name),
            );
            return;
        };
        match ImageReference::parse(image_name.clone()) {
            Ok(reference) => {
                let image_source = image_sources.get(&container.image).cloned();
                if image_source.is_none() && !container.image.is_empty() {
                    self.missing_relationship("image", &image_name, Some(source_id));
                }
                observation.set_image(reference, image_source);
            }
            Err(_) => self.invalid_resource(
                "container image",
                "image reference is structurally invalid",
                Some(container_name),
            ),
        }
        if let Some(command) = combined_container_command(container.path.take(), container.args.take()) {
            observation.set_command(command);
        }
        self.map_container_config(observation, container_name, source_id, config);
    }

    fn map_container_config(
        &mut self,
        observation: &mut ContainerObservation,
        container_name: &str,
        source_id: &SourceId,
        config: crate::native::ContainerConfig,
    ) {
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
        let mut fields = meaningful_fields(&config.other, &[]);
        if config.entrypoint.as_ref().is_some_and(|values| !values.is_empty()) {
            fields.push("EntrypointBoundary".to_owned());
        }
        if config.cmd.as_ref().is_some_and(|values| !values.is_empty()) {
            fields.push("CommandBoundary".to_owned());
        }
        self.report_fields(
            format!("runtime.docker.containers.{container_name}.Config"),
            fields,
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
                .and_then(|native| decode_restart_policy(&native, false))
            {
                Ok(policy) => observation.set_restart_policy(policy),
                Err(reason) => self.invalid_resource("container restart policy", reason, Some(container_name)),
            },
        }
        self.report_fields(
            format!("runtime.docker.containers.{container_name}.HostConfig"),
            meaningful_fields(host_config, &[]),
            Some(source_id),
        );
    }

    fn report_container_fields(&mut self, container_name: &str, source_id: &SourceId, container: &ContainerInspect) {
        self.report_fields(
            format!("runtime.docker.containers.{container_name}"),
            meaningful_fields(
                &container.other,
                &[
                    "Created",
                    "State",
                    "ResolvConfPath",
                    "HostnamePath",
                    "HostsPath",
                    "LogPath",
                    "RestartCount",
                    "Driver",
                    "Platform",
                    "MountLabel",
                    "ProcessLabel",
                    "AppArmorProfile",
                    "ExecIDs",
                    "GraphDriver",
                    "SizeRw",
                    "SizeRootFs",
                ],
            ),
            Some(source_id),
        );
    }

    fn map_network_settings(
        &mut self,
        container_name: &str,
        container_id: &str,
        source_id: &SourceId,
        observation: &mut ContainerObservation,
        settings: crate::native::NetworkSettings,
        network_names: &BTreeSet<String>,
    ) {
        self.ports(container_name, observation, settings.ports);
        for (network_name, network) in settings.networks {
            self.report_fields(
                format!("runtime.docker.containers.{container_name}.Networks.{network_name}"),
                meaningful_fields(
                    &network.other,
                    &[
                        "EndpointID",
                        "Gateway",
                        "IPAddress",
                        "IPPrefixLen",
                        "IPv6Gateway",
                        "GlobalIPv6Address",
                        "GlobalIPv6PrefixLen",
                        "MacAddress",
                        "NetworkID",
                    ],
                ),
                Some(source_id),
            );
            if !network_names.contains(&network_name) {
                self.missing_relationship("network", &network_name, Some(source_id));
            }
            let aliases = self.configured_network_aliases(container_id, network.aliases.unwrap_or_default());
            match Identifier::new(network_name.clone()) {
                Ok(network_name) => observation.add_network(NetworkAttachment::new(network_name, aliases)),
                Err(_) => self.invalid_resource(
                    "container network",
                    "network name is empty or contains a NUL byte",
                    Some(container_name),
                ),
            }
        }
    }

    fn configured_network_aliases(&self, container_id: &str, mut aliases: Vec<String>) -> Vec<String> {
        if self.source.api_version() < crate::DockerApiVersion::new(1, 45) {
            let short_id = container_id.get(..12);
            aliases.retain(|alias| short_id != Some(alias.as_str()));
        }
        aliases
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
            format!("runtime.docker.{kind}s.{label}.Config.Healthcheck"),
            meaningful_fields(&native.other, &[]),
            Some(source_id),
        );
        match decode_healthcheck(native, self.source.api_version() >= crate::DockerApiVersion::new(1, 44)) {
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
                    format!("runtime.docker.containers.{container}.Mounts[{index}]"),
                    ConversionKind::Unsupported,
                    "Docker mount type has no runtime-neutral representation",
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
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        match (options.contains("z"), options.contains("Z")) {
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
        let mut fields = options
            .into_iter()
            .filter(|option| !matches!(*option, "rw" | "ro" | "z" | "Z"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
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
            format!("runtime.docker.containers.{container}.Mounts[{index}]"),
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

    fn report_fields(
        &mut self,
        subject: String,
        fields: impl IntoIterator<Item = String>,
        source_id: Option<&SourceId>,
    ) {
        let fields = fields.into_iter().collect::<Vec<_>>();
        if fields.is_empty() {
            return;
        }
        let field_names = fields.join(",");
        self.loss(
            self.codes.unmodeled_configuration.clone(),
            subject,
            ConversionKind::Unsupported,
            "Docker inspection contains reusable configuration not represented by this adapter",
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
            format!("runtime.docker.relationships.{resource}"),
            ConversionKind::Unsupported,
            "Docker inspection references a resource absent from the supplied document set",
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
            ("action", "correct or recapture the Docker inspect document".to_owned()),
        ];
        if let Some(label) = label {
            fields.push(("resource", label.to_owned()));
        }
        self.loss(
            self.codes.invalid_data.clone(),
            format!("runtime.docker.{kind}"),
            ConversionKind::Invalid,
            "Docker inspection contains an invalid reusable value",
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

fn combined_container_command(path: Option<String>, args: Option<Vec<String>>) -> Option<EffectiveCommand> {
    if path.is_none() && args.is_none() {
        return None;
    }
    let mut values = Vec::new();
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        values.push(path);
    }
    values.extend(args.unwrap_or_default());
    Some(effective_command(values))
}

fn combined_image_command(config: &ImageConfig) -> Option<EffectiveCommand> {
    if config.entrypoint.is_none() && config.cmd.is_none() {
        return None;
    }
    let mut values = config.entrypoint.clone().unwrap_or_default();
    values.extend(config.cmd.clone().unwrap_or_default());
    Some(effective_command(values))
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

fn decode_healthcheck(native: HealthConfig, allow_start_interval: bool) -> Result<RuntimeHealthcheck, &'static str> {
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
    if !allow_start_interval && native.start_interval.is_some_and(|value| value != 0) {
        return Err("health-check start interval is not available in the selected Docker API version");
    }
    if allow_start_interval {
        set_duration(
            &mut healthcheck,
            native.start_interval,
            RuntimeHealthcheck::set_start_interval,
        )?;
    }
    Ok(healthcheck)
}

fn health_scalars_are_configured(native: &HealthConfig) -> bool {
    [
        native.interval,
        native.timeout,
        native.retries,
        native.start_period,
        native.start_interval,
    ]
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

fn container_name(value: &str) -> Option<&str> {
    let value = value.strip_prefix('/').unwrap_or(value);
    (!value.is_empty()).then_some(value)
}

fn named_source(kind: &str, label: &str) -> Result<SourceId, ModelError> {
    SourceId::new(format!("runtime:docker:{kind}:{label}"))
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
