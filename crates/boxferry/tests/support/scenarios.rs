//! Authored expectations and run-derived observations for migration scenarios.
#![allow(dead_code)] // Shared by separately compiled integration-test entry points.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use boxferry::LossPolicy;
use boxferry::{
    Application, Command, ConversionKind, ConversionOutcome, Diagnostic, Entrypoint, EnvironmentValue,
    HealthcheckCommand, MountSource, PlatformVersion, ResourceGrantSyntax, ResourceOwnership, RestartPolicy,
    SelinuxRelabel, ServiceDependencyCondition, TargetProfile,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ScenarioManifest {
    pub schema: u32,
    pub id: String,
    pub application: ApplicationVersion,
    pub components: Vec<Component>,
    pub native_inputs: Vec<NativeInput>,
    pub provenance: Provenance,
    pub deployment: Deployment,
    pub source_capabilities: Vec<String>,
    pub target_capabilities: Vec<String>,
    #[serde(default)]
    pub protected_values: Vec<String>,
    pub semantics: Semantics,
    pub evidence: Vec<RouteExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplicationVersion {
    pub name: String,
    pub version: Option<String>,
    pub revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Component {
    pub name: String,
    pub version: Option<String>,
    pub image: Option<String>,
    pub digest: Option<String>,
    #[serde(rename = "configured-image")]
    pub configured_image: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct NativeInput {
    pub id: String,
    pub importer: String,
    pub files: Vec<String>,
    pub format_version: String,
    #[serde(default = "default_input_outcome")]
    pub outcome: Outcome,
    pub reason: Option<String>,
    #[serde(default)]
    pub interpolate: bool,
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub environment_files: Vec<String>,
    #[serde(default)]
    pub all_profiles: bool,
    pub import_diagnostics: Option<Vec<String>>,
    pub import_diagnostics_file: Option<String>,
    pub podman: Option<PodmanInput>,
}

const fn default_input_outcome() -> Outcome {
    Outcome::MigrationSuccess
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct PodmanInput {
    pub application: String,
    #[serde(default)]
    pub all: bool,
    pub cassette_scenario_id: Option<String>,
    pub root_mode: Option<String>,
    pub include_environment_values: bool,
    pub selectors: Vec<PodmanSelector>,
    pub promotion_policy: PodmanPromotionPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct PodmanSelector {
    pub kind: String,
    pub exact: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the scenario contract mirrors four independent user-facing Podman promotion switches"
)]
pub(crate) struct PodmanPromotionPolicy {
    pub effective_bind_mounts: bool,
    pub effective_named_volume_mounts: bool,
    pub effective_named_networks: bool,
    pub portable_effective_settings: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Provenance {
    pub source: String,
    pub license: String,
    pub privacy_reviewed: bool,
    pub test_data: String,
    pub image_evidence: String,
    pub corpus_project: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Deployment {
    pub origin: String,
    pub root_mode: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Outcome {
    MigrationSuccess,
    ExpectedRejection,
    UnsupportedEnvironment,
    KnownMigrationGap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Dimension {
    pub state: DimensionState,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DimensionState {
    Passed,
    NotApplicable,
    UnsupportedEnvironment,
    KnownMigrationGap,
}

impl Dimension {
    pub(crate) const fn passed() -> Self {
        Self {
            state: DimensionState::Passed,
            reason: None,
        }
    }

    pub(crate) fn not_applicable(reason: &str) -> Self {
        Self {
            state: DimensionState::NotApplicable,
            reason: Some(reason.to_owned()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct RouteExpectation {
    pub input: String,
    pub exporter: String,
    pub target_implementation: String,
    pub loss_policy: String,
    pub target_minimum: String,
    pub target_maximum: String,
    pub outcome: Outcome,
    pub reason: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    pub artifacts_file: Option<String>,
    pub native_validation: Dimension,
    pub runtime_probe: Dimension,
    pub reimport: Dimension,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub diagnostics_file: Option<String>,
    #[serde(default)]
    pub reimport_diagnostics: Vec<String>,
    pub reimport_diagnostics_file: Option<String>,
    #[serde(default)]
    pub reimport_allowed_losses: Vec<LossTuple>,
    pub reimport_allowed_losses_file: Option<String>,
    pub reimport_version_scope: Option<String>,
    #[serde(default)]
    pub allowed_losses: Vec<LossTuple>,
    pub allowed_losses_file: Option<String>,
    #[serde(default)]
    pub semantic_gaps: Vec<String>,
    #[serde(default)]
    pub reimport_semantic_gaps: Vec<String>,
    #[serde(default)]
    pub environment_order: Vec<String>,
    #[serde(default)]
    pub unavailable_prerequisites: Vec<String>,
    pub grouping: Option<String>,
    pub pod_name: Option<String>,
    pub podman_plan: Option<PodmanPlanExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct PodmanPlanExpectation {
    pub creatable_resources: Vec<String>,
    pub external_preconditions: Vec<String>,
    pub image_operations: Vec<ImageOperationExpectation>,
    pub start_order: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ImageOperationExpectation {
    pub resource: String,
    pub reference: String,
    pub policy: String,
}

impl RouteExpectation {
    pub(crate) fn policy(&self) -> Result<LossPolicy, String> {
        match self.loss_policy.as_str() {
            "exact" => Ok(LossPolicy::ExactOnly),
            "approximate" => Ok(LossPolicy::AllowApproximate),
            "partial" => Ok(LossPolicy::AllowPartial),
            _ => Err("unknown scenario loss policy".into()),
        }
    }

    pub(crate) fn target(&self) -> Result<TargetProfile, String> {
        TargetProfile::new(
            &self.target_implementation,
            self.target_minimum.parse().map_err(|error| format!("{error}"))?,
            Some(self.target_maximum.parse().map_err(|error| format!("{error}"))?),
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn version_scope(&self) -> String {
        format!(
            "{}@{}..{}",
            self.target_implementation, self.target_minimum, self.target_maximum
        )
    }
}

/// Actual results are constructed only after running their respective checks.
#[derive(Debug)]
pub(crate) struct RouteObservation {
    pub blocked: bool,
    pub semantics_match: bool,
    pub native_validation: Dimension,
    pub runtime_probe: Dimension,
    pub reimport: Dimension,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct LossTuple {
    pub rule: String,
    pub subject: String,
    pub decision: String,
    pub version_scope: String,
    #[serde(default = "default_loss_count")]
    pub count: usize,
}

const fn default_loss_count() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Semantics {
    pub selected_resources: Vec<String>,
    #[serde(default)]
    pub excluded_resources: Vec<String>,
    #[serde(default)]
    pub ownership_boundaries: Vec<String>,
    #[serde(default)]
    pub shared_boundaries: Vec<String>,
    #[serde(default)]
    pub required_mounts: Vec<String>,
    #[serde(default)]
    pub required_tmpfs: Vec<String>,
    #[serde(default)]
    pub required_bind_mounts: Vec<BindMountExpectation>,
    #[serde(default)]
    pub required_environment: Vec<String>,
    #[serde(default)]
    pub required_environment_order: Vec<String>,
    #[serde(default)]
    pub required_dependencies: Vec<DependencyExpectation>,
    #[serde(default)]
    pub required_healthchecks: Vec<HealthcheckExpectation>,
    #[serde(default)]
    pub required_ports: Vec<String>,
    #[serde(default)]
    pub required_restart: Vec<String>,
    #[serde(default)]
    pub required_runtime_names: Vec<String>,
    #[serde(default)]
    pub unpublished_services: Vec<String>,
    #[serde(default)]
    pub external_prerequisites: Vec<String>,
    #[serde(default)]
    pub expected_diagnostics: Vec<String>,
    #[serde(default)]
    pub required_networks: Vec<NetworkExpectation>,
    #[serde(default)]
    pub exact_network_memberships: Vec<String>,
    #[serde(default)]
    pub required_groups: Vec<GroupExpectation>,
    #[serde(default)]
    pub required_commands: Vec<ProcessExpectation>,
    #[serde(default)]
    pub required_entrypoints: Vec<ProcessExpectation>,
    #[serde(default)]
    pub required_config_grants: Vec<GrantExpectation>,
    #[serde(default)]
    pub required_secret_grants: Vec<GrantExpectation>,
    #[serde(default)]
    pub required_image_acquisitions: Vec<String>,
    #[serde(default)]
    pub required_image_builds: Vec<String>,
    #[serde(default)]
    pub required_image_sources: Vec<ImageSourceExpectation>,
    #[serde(default)]
    pub required_port_bindings: Vec<PortExpectation>,
    #[serde(default)]
    pub required_volume_mounts: Vec<VolumeMountExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct DependencyExpectation {
    pub service: String,
    pub dependency: String,
    pub condition: String,
    pub required: Option<bool>,
    pub restart: Option<bool>,
    pub assert_options: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct HealthcheckExpectation {
    pub service: String,
    pub command_kind: String,
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct BindMountExpectation {
    pub service: String,
    pub source: String,
    pub target: String,
    pub read_only: bool,
    pub selinux_relabel: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct NetworkExpectation {
    pub name: String,
    pub ownership: String,
    pub internal: Option<bool>,
    pub ipv6: Option<bool>,
    pub ipam: Option<Vec<NetworkIpamExpectation>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct NetworkIpamExpectation {
    pub subnet: String,
    pub gateway: Option<String>,
    pub ip_range: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct GroupExpectation {
    pub name: String,
    pub ownership: String,
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ProcessExpectation {
    pub service: String,
    pub kind: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct GrantExpectation {
    pub service: String,
    pub source: String,
    pub syntax: String,
    pub target: Option<String>,
    pub uid: Option<String>,
    pub gid: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ImageSourceExpectation {
    pub service: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct PortExpectation {
    pub service: String,
    pub container: u16,
    pub published: Option<u16>,
    pub host_address: Option<String>,
    pub protocol: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct VolumeMountExpectation {
    pub service: String,
    pub source: String,
    pub target: String,
    pub read_only: bool,
    pub selinux_relabel: Option<String>,
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps manifest schema invariants in one review transaction"
)]
pub(crate) fn validate_manifest(manifest: &ScenarioManifest, location: &Path) -> Result<(), String> {
    if manifest.schema != 1 {
        return Err("scenario schema must be exactly 1".into());
    }
    required("scenario id", &manifest.id)?;
    let directory = if location.is_file() {
        location.parent().ok_or("scenario manifest has no fixture directory")?
    } else {
        location
    };
    let location_id = if location.is_file() {
        match location.file_name().and_then(|name| name.to_str()) {
            Some("scenario.toml") => directory.file_name().and_then(|name| name.to_str()),
            Some(name) => name.strip_suffix(".scenario.toml"),
            None => None,
        }
    } else {
        directory.file_name().and_then(|name| name.to_str())
    };
    if location_id != Some(&manifest.id) {
        return Err("scenario id must match its manifest location".into());
    }
    required("application name", &manifest.application.name)?;
    match (&manifest.application.version, &manifest.application.revision) {
        (Some(version), None) => pinned_version(version)?,
        (None, Some(revision))
            if revision.len() == 40
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) => {}
        _ => return Err("application requires exactly one numeric version or lowercase Git revision".into()),
    }
    unique(
        "components",
        manifest.components.iter().map(|entry| entry.name.as_str()),
    )?;
    if manifest.native_inputs.is_empty() || manifest.evidence.is_empty() {
        return Err("scenario requires inputs and route expectations".into());
    }
    let all_inputs_rejected = manifest
        .native_inputs
        .iter()
        .all(|input| input.outcome == Outcome::ExpectedRejection);
    if manifest.components.is_empty() && !all_inputs_rejected {
        return Err("accepted scenario requires components".into());
    }
    for component in &manifest.components {
        required("component name", &component.name)?;
        if let Some(version) = &component.version {
            pinned_version(version)?;
        }
        if let Some(configured) = &component.configured_image {
            required("component configured image", configured)?;
        }
        match (&component.image, &component.digest) {
            (Some(image), Some(digest)) => {
                required("component image", image)?;
                if digest.len() != 71
                    || !digest.starts_with("sha256:")
                    || !digest[7..]
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                {
                    return Err(format!(
                        "component {} requires an exact lowercase SHA-256 digest",
                        component.name
                    ));
                }
            }
            (None, None) => {}
            _ => return Err("component image and digest must be declared together".into()),
        }
        if component.configured_image.is_none() && component.image.is_none() {
            return Err("component requires configured image or digest identity".into());
        }
    }
    unique(
        "input ids",
        manifest.native_inputs.iter().map(|input| input.id.as_str()),
    )?;
    for input in &manifest.native_inputs {
        required("input id", &input.id)?;
        required("input importer", &input.importer)?;
        required("input format version", &input.format_version)?;
        if input.files.is_empty() {
            return Err("native input needs at least one file".into());
        }
        unique("input files", input.files.iter().map(String::as_str))?;
        unique("input environment", input.environment.iter().map(String::as_str))?;
        unique(
            "input environment files",
            input.environment_files.iter().map(String::as_str),
        )?;
        load_string_expectations(
            input.import_diagnostics.as_deref().unwrap_or(&[]),
            input.import_diagnostics_file.as_deref(),
            directory,
            "input diagnostics",
        )?;
        if (!input.environment.is_empty()
            || !input.environment_files.is_empty()
            || input.interpolate
            || input.all_profiles)
            && input.importer != "compose"
        {
            return Err("interpolation belongs only to Compose scenario inputs".into());
        }
        if !input.interpolate && (!input.environment.is_empty() || !input.environment_files.is_empty()) {
            return Err("scenario environment assignments require interpolation".into());
        }
        if input.outcome == Outcome::ExpectedRejection {
            required("rejected input reason", input.reason.as_deref().unwrap_or(""))?;
        } else if input.outcome != Outcome::MigrationSuccess || input.reason.is_some() {
            return Err("scenario input outcome must be migration-success or expected-rejection".into());
        }
        for file in input.files.iter().chain(&input.environment_files) {
            contained_file(directory, file)?;
        }
        validate_native_input(input, &manifest.application.name)?;
    }
    for value in [
        &manifest.provenance.source,
        &manifest.provenance.license,
        &manifest.provenance.test_data,
        &manifest.provenance.image_evidence,
    ] {
        required("reviewed provenance", value)?;
    }
    if !manifest.provenance.privacy_reviewed {
        return Err("privacy review is required".into());
    }
    let scenario_project = manifest.id.strip_prefix("real-world-compose-");
    match (manifest.provenance.corpus_project.as_deref(), scenario_project) {
        (Some(project), Some(identity)) => {
            required("provenance corpus project", project)?;
            if manifest.provenance.source != "external"
                || project != identity
                || manifest.application.name != project
                || manifest.application.revision.is_none()
            {
                return Err("real-world corpus provenance does not match the scenario identity".into());
            }
        }
        (None, None) => {}
        _ => {
            return Err("real-world corpus provenance does not match the scenario identity".into());
        }
    }
    if !matches!(
        manifest.deployment.origin.as_str(),
        "authored" | "native-cli" | "compose-provider" | "quadlet" | "external"
    ) || !matches!(
        manifest.deployment.root_mode.as_str(),
        "rootful" | "rootless" | "not-applicable"
    ) {
        return Err("unknown deployment origin or root mode".into());
    }
    unique(
        "source capabilities",
        manifest.source_capabilities.iter().map(String::as_str),
    )?;
    unique(
        "target capabilities",
        manifest.target_capabilities.iter().map(String::as_str),
    )?;
    unique("protected values", manifest.protected_values.iter().map(String::as_str))?;
    for value in &manifest.protected_values {
        required("protected value", value)?;
    }
    if all_inputs_rejected {
        if !semantics_is_empty(&manifest.semantics) {
            return Err("importer-rejection scenario cannot claim neutral semantics".into());
        }
    } else {
        validate_semantics(&manifest.semantics)?;
    }
    validate_routes(manifest, directory)
}

fn validate_podman_plan(plan: &PodmanPlanExpectation) -> Result<(), String> {
    unique(
        "Podman creatable resources",
        plan.creatable_resources.iter().map(String::as_str),
    )?;
    unique(
        "Podman external preconditions",
        plan.external_preconditions.iter().map(String::as_str),
    )?;
    unique("Podman start order", plan.start_order.iter().map(String::as_str))?;
    let mut images = BTreeSet::new();
    for image in &plan.image_operations {
        required("Podman image operation resource", &image.resource)?;
        required("Podman image operation reference", &image.reference)?;
        if image.policy != "missing" {
            return Err("unreviewed Podman image operation policy".into());
        }
        if !images.insert(image.resource.as_str()) {
            return Err("duplicate Podman image operation resource".into());
        }
    }
    for resource in plan.creatable_resources.iter().chain(&plan.external_preconditions) {
        let Some((kind, name)) = resource.split_once(':') else {
            return Err("Podman plan resource requires kind:name".into());
        };
        if !matches!(kind, "container" | "network" | "pod" | "secret" | "volume") || name.is_empty() {
            return Err("unreviewed Podman plan resource".into());
        }
    }
    let creatable = plan
        .creatable_resources
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let external = plan
        .external_preconditions
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !creatable.is_disjoint(&external) {
        return Err("Podman creatable resources and external preconditions must be disjoint".into());
    }
    let creatable_containers = creatable
        .iter()
        .filter_map(|resource| resource.strip_prefix("container:"))
        .collect::<BTreeSet<_>>();
    let started_containers = plan.start_order.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if creatable_containers != started_containers {
        return Err("Podman start order must cover exactly the creatable containers".into());
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps route evidence invariants in one review transaction"
)]
fn validate_routes(manifest: &ScenarioManifest, directory: &Path) -> Result<(), String> {
    let mut routes = BTreeSet::new();
    for route in &manifest.evidence {
        if !routes.insert((&route.input, &route.exporter)) {
            return Err("duplicate input/exporter expectation".into());
        }
        let input = manifest
            .native_inputs
            .iter()
            .find(|input| input.id == route.input)
            .ok_or("route references an unknown scenario input")?;
        if input.outcome == Outcome::ExpectedRejection && route.outcome != Outcome::ExpectedRejection {
            return Err("rejected scenario input requires rejection from every exporter route".into());
        }
        route.target()?;
        route.policy()?;
        if route.exporter == "quadlet" {
            if route
                .grouping
                .as_deref()
                .is_some_and(|value| !matches!(value, "separate" | "pod" | "preserve"))
            {
                return Err("unknown Quadlet grouping expectation".into());
            }
            if route.pod_name.is_some() && route.grouping.as_deref() != Some("pod") {
                return Err("Quadlet pod name requires pod grouping".into());
            }
        } else if route.grouping.is_some() || route.pod_name.is_some() {
            return Err("Quadlet grouping metadata belongs only Quadlet routes".into());
        }
        match (route.exporter.as_str(), route.podman_plan.as_ref()) {
            ("podman", Some(plan)) => validate_podman_plan(plan)?,
            (_, None) => {}
            (_, Some(_)) => return Err("Podman plan metadata belongs only to Podman routes".into()),
        }

        let label = format!("{} -> {}", route.input, route.exporter);
        let artifacts = load_string_expectations(
            &route.artifacts,
            route.artifacts_file.as_deref(),
            directory,
            &format!("{label} artifacts"),
        )?;
        let diagnostics = load_string_expectations(
            &route.diagnostics,
            route.diagnostics_file.as_deref(),
            directory,
            &format!("{label} diagnostics"),
        )?;
        let reimport_diagnostics = load_string_expectations(
            &route.reimport_diagnostics,
            route.reimport_diagnostics_file.as_deref(),
            directory,
            &format!("{label} reimport diagnostics"),
        )?;
        let losses = load_loss_expectations(
            &route.allowed_losses,
            route.allowed_losses_file.as_deref(),
            directory,
            &format!("{label} losses"),
        )?;
        let reimport_losses = load_loss_expectations(
            &route.reimport_allowed_losses,
            route.reimport_allowed_losses_file.as_deref(),
            directory,
            &format!("{label} reimport losses"),
        )?;

        if route.outcome == Outcome::ExpectedRejection {
            if !artifacts.is_empty()
                || !route.semantic_gaps.is_empty()
                || !route.reimport_semantic_gaps.is_empty()
                || !reimport_diagnostics.is_empty()
                || !route.environment_order.is_empty()
            {
                return Err("expected rejection cannot declare emitted or reimport evidence".into());
            }
            if diagnostics.is_empty() {
                return Err("expected rejection requires exact declared losses and diagnostics".into());
            }
            if input.outcome == Outcome::ExpectedRejection && !losses.is_empty() {
                return Err("importer rejection cannot declare exporter losses".into());
            }
            if input.outcome != Outcome::ExpectedRejection && losses.is_empty() {
                return Err("policy rejection requires exact declared losses".into());
            }
        }
        unique(
            "unavailable prerequisites",
            route.unavailable_prerequisites.iter().map(String::as_str),
        )?;
        if route
            .unavailable_prerequisites
            .iter()
            .any(|entry| !manifest.semantics.external_prerequisites.contains(entry))
        {
            return Err("unavailable prerequisite was not declared by the scenario".into());
        }
        if !route.unavailable_prerequisites.is_empty() && route.outcome != Outcome::UnsupportedEnvironment {
            return Err("unavailable prerequisites require an unsupported-environment outcome".into());
        }
        if route.outcome != Outcome::KnownMigrationGap && !route.semantic_gaps.is_empty() {
            return Err("only a known-gap route may expect missing native semantics".into());
        }
        if route.outcome != Outcome::MigrationSuccess {
            required("non-success reason", route.reason.as_deref().unwrap_or(""))?;
        }
        for dimension in [&route.native_validation, &route.runtime_probe, &route.reimport] {
            if dimension.state != DimensionState::Passed {
                required("non-passing evidence reason", dimension.reason.as_deref().unwrap_or(""))?;
            }
            if route.outcome == Outcome::MigrationSuccess
                && matches!(
                    dimension.state,
                    DimensionState::KnownMigrationGap | DimensionState::UnsupportedEnvironment
                )
            {
                return Err("a gap or unsupported environment cannot be migration success".into());
            }
        }
        unique("artifacts", artifacts.iter().map(String::as_str))?;
        unique(
            "route environment order",
            route.environment_order.iter().map(String::as_str),
        )?;
        if artifacts.iter().any(|file| !safe_path(file)) {
            return Err("unsafe artifact path".into());
        }
        let mut seen_losses = BTreeSet::new();
        for loss in &losses {
            required("loss rule", &loss.rule)?;
            required("loss subject", &loss.subject)?;
            if !matches!(loss.decision.as_str(), "approximate" | "unsupported" | "invalid")
                || loss.version_scope != route.version_scope()
                || loss.count == 0
                || !seen_losses.insert((
                    loss.rule.as_str(),
                    loss.subject.as_str(),
                    loss.decision.as_str(),
                    loss.version_scope.as_str(),
                ))
            {
                return Err(
                    "loss tuples must have a positive count, be unique, non-exact and scoped to this route target"
                        .into(),
                );
            }
        }
        let reimport_scope = route
            .reimport_version_scope
            .clone()
            .unwrap_or_else(|| route.version_scope());
        let mut seen_reimport_losses = BTreeSet::new();
        for loss in &reimport_losses {
            required("reimport loss rule", &loss.rule)?;
            required("reimport loss subject", &loss.subject)?;
            if !matches!(loss.decision.as_str(), "approximate" | "unsupported" | "invalid")
                || loss.version_scope != reimport_scope
                || loss.count == 0
                || !seen_reimport_losses.insert((
                    loss.rule.as_str(),
                    loss.subject.as_str(),
                    loss.decision.as_str(),
                    loss.version_scope.as_str(),
                ))
            {
                return Err("reimport loss tuples must be positive, unique, non-exact and target-scoped".into());
            }
        }
    }
    Ok(())
}

/// Coverage is per input case, not only per distinct importer name.
pub(crate) fn validate_exporter_coverage(
    manifest: &ScenarioManifest,
    registered: &[(String, String)],
) -> Result<(), String> {
    let mut expected = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    for input in &manifest.native_inputs {
        let applicable = registered
            .iter()
            .filter(|(importer, _)| importer == &input.importer)
            .collect::<Vec<_>>();
        if applicable.is_empty() {
            return Err(format!("unregistered importer {}", input.importer));
        }
        for (_, output) in applicable {
            expected.insert((input.id.as_str(), output.as_str()));
            outputs.insert(output.as_str());
        }
    }
    let actual = manifest
        .evidence
        .iter()
        .map(|route| (route.input.as_str(), route.exporter.as_str()))
        .collect::<BTreeSet<_>>();
    if expected != actual {
        return Err(format!(
            "every input must exercise every exporter: expected {expected:?}, found {actual:?}"
        ));
    }
    let sources = manifest
        .native_inputs
        .iter()
        .map(|input| input.importer.as_str())
        .collect::<BTreeSet<_>>();
    if sources != manifest.source_capabilities.iter().map(String::as_str).collect()
        || outputs != manifest.target_capabilities.iter().map(String::as_str).collect()
    {
        return Err("declared route capabilities disagree with the executable registry".into());
    }
    Ok(())
}

pub(crate) fn validate_evidence(expected: &RouteExpectation, actual: &RouteObservation) -> Result<(), String> {
    for (label, wanted, observed) in [
        (
            "native validation",
            &expected.native_validation,
            &actual.native_validation,
        ),
        ("runtime probe", &expected.runtime_probe, &actual.runtime_probe),
        ("reimport", &expected.reimport, &actual.reimport),
    ] {
        if wanted != observed {
            return Err(format!("{label} differs: expected {wanted:?}, observed {observed:?}"));
        }
    }
    let observed_outcome = if actual.native_validation.state == DimensionState::UnsupportedEnvironment
        || actual.runtime_probe.state == DimensionState::UnsupportedEnvironment
        || actual.reimport.state == DimensionState::UnsupportedEnvironment
    {
        Outcome::UnsupportedEnvironment
    } else if actual.blocked {
        Outcome::ExpectedRejection
    } else if !actual.semantics_match
        || actual.reimport.state == DimensionState::KnownMigrationGap
        || actual.native_validation.state == DimensionState::KnownMigrationGap
        || actual.runtime_probe.state == DimensionState::KnownMigrationGap
    {
        Outcome::KnownMigrationGap
    } else {
        Outcome::MigrationSuccess
    };
    if expected.outcome != observed_outcome {
        return Err(format!(
            "outcome differs: expected {:?}, observed {observed_outcome:?}",
            expected.outcome
        ));
    }
    Ok(())
}

pub(crate) fn validate_losses(expected: &[LossTuple], actual: &[ConversionOutcome], scope: &str) -> Result<(), String> {
    let mut observed_counts = BTreeMap::new();
    for outcome in actual.iter().filter(|outcome| outcome.kind() != ConversionKind::Exact) {
        let rule = outcome
            .diagnostic()
            .ok_or("non-exact outcome lacks a rule")?
            .as_str()
            .to_owned();
        let subject = outcome.subject().to_owned();
        let decision = match outcome.kind() {
            ConversionKind::Approximate => "approximate",
            ConversionKind::Unsupported => "unsupported",
            ConversionKind::Invalid => "invalid",
            _ => return Err("unreviewed conversion kind".into()),
        }
        .to_owned();
        *observed_counts.entry((rule, subject, decision)).or_insert(0) += 1;
    }
    let mut observed = observed_counts
        .into_iter()
        .map(|((rule, subject, decision), count)| LossTuple {
            rule,
            subject,
            decision,
            version_scope: scope.into(),
            count,
        })
        .collect::<Vec<_>>();
    let mut wanted = expected.to_vec();
    wanted.sort();
    observed.sort();
    if wanted != observed {
        let missing = wanted
            .iter()
            .filter(|expected| !observed.contains(expected))
            .collect::<Vec<_>>();
        let unexpected = observed
            .iter()
            .filter(|actual| !wanted.contains(actual))
            .collect::<Vec<_>>();
        return Err(format!(
            "loss tuples differ: missing {missing:?}; unexpected {unexpected:?}"
        ));
    }
    Ok(())
}

pub(crate) fn diagnostic_facts(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let subject = diagnostic
                .fields()
                .iter()
                .find(|field| field.name() == "subject")
                .map_or("<global>", |field| field.value().expose());
            format!("{}|{subject}", diagnostic.code().as_str())
        })
        .collect()
}

pub(crate) fn load_string_expectations(
    inline: &[String],
    file: Option<&str>,
    directory: &Path,
    label: &str,
) -> Result<Vec<String>, String> {
    let Some(file) = file else {
        return Ok(inline.to_vec());
    };
    if !inline.is_empty() {
        return Err(format!("{label} cannot mix inline and sidecar expectations"));
    }
    contained_file(directory, file)?;
    let values = fs::read_to_string(directory.join(file))
        .map_err(|error| format!("{label} sidecar {file}: {error}"))?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(format!("{label} sidecar cannot be empty"));
    }
    Ok(values)
}

pub(crate) fn load_loss_expectations(
    inline: &[LossTuple],
    file: Option<&str>,
    directory: &Path,
    label: &str,
) -> Result<Vec<LossTuple>, String> {
    let rows = load_string_expectations(&[], file, directory, label)?;
    if file.is_none() {
        return Ok(inline.to_vec());
    }
    if !inline.is_empty() {
        return Err(format!("{label} cannot mix inline and sidecar expectations"));
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let fields = row.split('\t').collect::<Vec<_>>();
            if fields.len() != 5 {
                return Err(format!(
                    "{label} sidecar row {} must have five tab-separated fields",
                    index + 1
                ));
            }
            let count = fields[4]
                .parse::<usize>()
                .map_err(|_| format!("{label} sidecar row {} has invalid count", index + 1))?;
            if count == 0 {
                return Err(format!("{label} sidecar row {} has zero count", index + 1));
            }
            Ok(LossTuple {
                rule: fields[0].to_owned(),
                subject: fields[1].to_owned(),
                decision: fields[2].to_owned(),
                version_scope: fields[3].to_owned(),
                count,
            })
        })
        .collect()
}

pub(crate) fn validate_diagnostics(expected: &[String], actual: &[String]) -> Result<(), String> {
    let mut expected = expected.to_vec();
    let mut actual = actual.to_vec();
    expected.sort();
    actual.sort();
    if expected != actual {
        return Err(format!(
            "diagnostics differ: expected {expected:?}, observed {actual:?}"
        ));
    }
    Ok(())
}

pub(crate) fn validate_neutral_application(
    manifest: &ScenarioManifest,
    application: &Application,
    environment_order: &[String],
    expected_gaps: &[String],
) -> Result<(), String> {
    let gaps = validate_neutral_application_collecting_gaps(manifest, application, environment_order)?;
    validate_diagnostics(expected_gaps, &gaps)
}

fn validate_neutral_application_collecting_gaps(
    manifest: &ScenarioManifest,
    application: &Application,
    environment_order: &[String],
) -> Result<Vec<String>, String> {
    let mut gaps = validate_resources(manifest, application)?;
    validate_images(manifest, application)?;
    validate_volume_boundaries(manifest, application)?;
    gaps.extend(validate_service_settings(manifest, application, environment_order)?);
    Ok(gaps)
}

fn validate_resources(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut actual = BTreeSet::new();
    for service in application.services() {
        actual.insert(format!("service:{}", service.value().name().as_str()));
    }
    for volume in application.volumes() {
        actual.insert(format!("volume:{}", volume.value().name().as_str()));
    }
    for network in application.networks() {
        actual.insert(format!("network:{}", network.value().name().as_str()));
    }
    for config in application.configs() {
        actual.insert(format!("config:{}", config.value().name().as_str()));
    }
    for secret in application.secrets() {
        actual.insert(format!("secret:{}", secret.value().name().as_str()));
    }
    for group in application.service_groups() {
        actual.insert(format!("group:{}", group.value().name().as_str()));
    }
    for acquisition in application.image_acquisitions() {
        actual.insert(format!("image-acquisition:{}", acquisition.value().name().as_str()));
    }
    for build in application.image_builds() {
        actual.insert(format!("image-build:{}", build.value().name().as_str()));
    }
    let expected = manifest
        .semantics
        .selected_resources
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut gaps = actual
        .difference(&expected)
        .map(|resource| format!("unexpected-resource:{resource}"))
        .collect::<Vec<_>>();
    gaps.extend(
        expected
            .difference(&actual)
            .map(|resource| format!("resource:{resource}")),
    );
    if manifest
        .semantics
        .excluded_resources
        .iter()
        .any(|resource| actual.contains(resource))
    {
        return Err("an excluded resource was selected".into());
    }
    gaps.extend(validate_groups(manifest, application));
    gaps.extend(validate_image_graph(manifest, application)?);
    Ok(gaps)
}

fn validate_groups(manifest: &ScenarioManifest, application: &Application) -> Vec<String> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_groups {
        let Some(actual) = application
            .service_groups()
            .iter()
            .find(|group| group.value().name().as_str() == expected.name)
            .map(boxferry::Sourced::value)
        else {
            continue;
        };
        if ownership_name(actual.ownership()) != expected.ownership {
            gaps.push(format!("group-ownership:{}", expected.name));
        }
        let members = actual
            .members()
            .iter()
            .map(|member| member.value().as_str())
            .collect::<Vec<_>>();
        if members != expected.members.iter().map(String::as_str).collect::<Vec<_>>() {
            gaps.push(format!("group-members:{}", expected.name));
        }
    }
    gaps
}

fn validate_image_graph(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    let actual_acquisitions = application
        .image_acquisitions()
        .iter()
        .map(|entry| entry.value().name().as_str())
        .collect::<BTreeSet<_>>();
    let expected_acquisitions = manifest
        .semantics
        .required_image_acquisitions
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !actual_acquisitions.is_subset(&expected_acquisitions) {
        return Err("unexpected image-acquisition graph node".into());
    }
    let actual_builds = application
        .image_builds()
        .iter()
        .map(|entry| entry.value().name().as_str())
        .collect::<BTreeSet<_>>();
    let expected_builds = manifest
        .semantics
        .required_image_builds
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if !actual_builds.is_subset(&expected_builds) {
        return Err("unexpected image-build graph node".into());
    }
    for expected in &manifest.semantics.required_image_sources {
        let service = service(application, &expected.service)?;
        let matches = match expected.kind.as_str() {
            "literal" => service
                .image()
                .is_some_and(|value| value.value().as_str() == expected.value),
            "acquisition" => service
                .image_acquisition()
                .is_some_and(|value| value.value().as_str() == expected.value),
            "build" => service
                .image_build()
                .is_some_and(|value| value.value().as_str() == expected.value),
            "rootfs" => service
                .rootfs()
                .is_some_and(|value| value.value().expose() == expected.value),
            _ => return Err("unvalidated service image-source kind".into()),
        };
        if !matches {
            gaps.push(format!("image-source:{}", expected.service));
        }
    }
    Ok(gaps)
}

fn ownership_name(ownership: ResourceOwnership) -> &'static str {
    match ownership {
        ResourceOwnership::Application => "application",
        ResourceOwnership::External => "external",
        ResourceOwnership::Implicit => "implicit",
        ResourceOwnership::Uncertain => "uncertain",
        _ => "unreviewed",
    }
}

fn validate_images(manifest: &ScenarioManifest, application: &Application) -> Result<(), String> {
    let selected = application
        .services()
        .iter()
        .map(|service| service.value().name().as_str())
        .collect::<BTreeSet<_>>();
    let components = manifest
        .components
        .iter()
        .map(|component| component.name.as_str())
        .collect::<BTreeSet<_>>();
    if !components.is_subset(&selected) {
        return Err("component inventory references a missing service".into());
    }
    for service in application.services() {
        let name = service.value().name().as_str();
        let Some(component) = manifest.components.iter().find(|component| component.name == name) else {
            continue;
        };
        let image = service
            .value()
            .image()
            .ok_or_else(|| format!("service {name} has no image"))?;
        let digest_reference = component
            .image
            .as_ref()
            .zip(component.digest.as_ref())
            .map(|(image, digest)| format!("{image}\u{40}{digest}"));
        let expected = component
            .configured_image
            .as_deref()
            .or(digest_reference.as_deref())
            .ok_or("component image expectation")?;
        if image.value().as_str() != expected {
            return Err(format!("service {name} image/digest changed"));
        }
    }
    Ok(())
}

fn validate_volume_boundaries(manifest: &ScenarioManifest, application: &Application) -> Result<(), String> {
    for boundary in &manifest.semantics.ownership_boundaries {
        let (kind, name, ownership) = split_three(boundary)?;
        if kind != "volume" {
            return Err("schema 1 currently supports volume ownership assertions".into());
        }
        let volume = application
            .volumes()
            .iter()
            .find(|volume| volume.value().name().as_str() == name)
            .ok_or_else(|| format!("owned volume {name} is absent"))?;
        let expected = match ownership {
            "application" => ResourceOwnership::Application,
            "external" => ResourceOwnership::External,
            "implicit" => ResourceOwnership::Implicit,
            "uncertain" => ResourceOwnership::Uncertain,
            _ => return Err("unknown ownership boundary".into()),
        };
        if volume.value().ownership() != expected {
            return Err(format!("volume {name} ownership changed"));
        }
    }
    for boundary in &manifest.semantics.shared_boundaries {
        let (kind, name, consumers) = split_three(boundary)?;
        if kind != "volume" {
            return Err("schema 1 currently supports shared-volume consumers".into());
        }
        let consumers = consumers
            .strip_prefix("consumers=")
            .ok_or("missing consumer declaration")?;
        let expected = if consumers == "none" {
            BTreeSet::new()
        } else {
            consumers.split(',').collect()
        };
        let actual = application
            .services()
            .iter()
            .filter(|service| {
                service.value().mounts().iter().any(
                    |mount| matches!(mount.value().source(), MountSource::Volume(volume) if volume.as_str() == name),
                )
            })
            .map(|service| service.value().name().as_str())
            .collect::<BTreeSet<_>>();
        if expected != actual {
            return Err(format!("volume {name} shared consumer boundary changed"));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines, reason = "keeps cross-field service assertions together")]
fn validate_service_settings(
    manifest: &ScenarioManifest,
    application: &Application,
    environment_order: &[String],
) -> Result<Vec<String>, String> {
    let mut semantic_gaps = Vec::new();
    for requirement in &manifest.semantics.required_mounts {
        let (service_name, volume_name, target) = split_three(requirement)?;
        let service = service(application, service_name)?;
        if !service.mounts().iter().any(|mount| {
            matches!(mount.value().source(), MountSource::Volume(volume)
            if volume.as_str() == volume_name)
                && mount.value().target() == target
        }) {
            return Err(format!("required mount {requirement} is absent"));
        }
    }
    for requirement in &manifest.semantics.required_tmpfs {
        let (service_name, target) = requirement.split_once(':').ok_or("invalid tmpfs assertion")?;
        let actual = service(application, service_name)?
            .tmpfs()
            .ok_or_else(|| format!("required tmpfs {requirement} absent"))?;
        if !actual.iter().any(|entry| entry.value().expose() == target) {
            return Err(format!("required tmpfs {requirement} absent"));
        }
    }
    for expected in &manifest.semantics.required_bind_mounts {
        let service = service(application, &expected.service)?;
        let relabel = match expected.selinux_relabel.as_str() {
            "shared" => SelinuxRelabel::Shared,
            "private" => SelinuxRelabel::Private,
            _ => return Err("unvalidated bind-mount relabel".into()),
        };
        let matches = service.mounts().iter().any(|mount| {
            matches!(mount.value().source(), MountSource::HostPath(path) if path == &expected.source)
                && mount.value().target() == expected.target
                && mount.value().read_only() == expected.read_only
                && mount.value().selinux_relabel() == Some(relabel)
        });
        if !matches {
            semantic_gaps.push(format!("bind-mount:{}:{}", expected.service, expected.target));
        }
    }
    for requirement in &manifest.semantics.required_environment {
        let (service_name, assignment) = requirement.split_once(':').ok_or("invalid environment assertion")?;
        let (name, expected) = assignment.split_once('=').ok_or("invalid environment assignment")?;
        let service = service(application, service_name)?;
        if !service.environment().iter().any(|environment| {
            environment.value().name().as_str() == name
                && matches!(environment.value().value(), EnvironmentValue::Literal(value) if value.expose() == expected)
        }) {
            return Err(format!("required environment {service_name}:{name} changed"));
        }
    }
    let ordered_services = environment_order
        .iter()
        .map(|requirement| requirement.split_once(':').map(|(owner, _)| owner))
        .collect::<Option<BTreeSet<_>>>()
        .ok_or("invalid ordered environment assertion")?;
    for owner in ordered_services {
        let expected = environment_order
            .iter()
            .filter_map(|requirement| requirement.strip_prefix(&format!("{owner}:")))
            .collect::<Vec<_>>();
        let actual = service(application, owner)?
            .environment()
            .iter()
            .map(|environment| match environment.value().value() {
                EnvironmentValue::Literal(value) => {
                    Ok(format!("{}={}", environment.value().name().as_str(), value.expose()))
                }
                _ => Err("ordered environment must be literal"),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if actual != expected {
            return Err(format!("service {owner} environment order changed"));
        }
    }
    for requirement in &manifest.semantics.required_ports {
        let (name, host, container_protocol) = split_three(requirement)?;
        let (container, protocol) = container_protocol.split_once('/').ok_or("invalid port protocol")?;
        if !service(application, name)?.ports().iter().any(|port| {
            port.value()
                .published()
                .is_some_and(|published| published.to_string() == host)
                && port.value().container().to_string() == container
                && format!("{:?}", port.value().protocol()).eq_ignore_ascii_case(protocol)
        }) {
            semantic_gaps.push(format!("publication:{requirement}"));
        }
    }
    for requirement in &manifest.semantics.required_runtime_names {
        let (service_name, expected) = requirement.split_once(':').ok_or("invalid runtime-name assertion")?;
        let actual = service(application, service_name)?
            .runtime_name()
            .map(|name| name.value().expose());
        if actual != Some(expected) {
            return Err(format!("service {service_name} runtime name changed"));
        }
    }
    for requirement in &manifest.semantics.required_restart {
        let (service_name, expected) = requirement.split_once(':').ok_or("invalid restart assertion")?;
        let actual = service(application, service_name)?
            .restart_policy()
            .map(boxferry::Sourced::value);
        let matches = match (expected, actual) {
            ("absent", None)
            | ("never", Some(RestartPolicy::Never))
            | ("always", Some(RestartPolicy::Always))
            | ("unless-stopped", Some(RestartPolicy::UnlessStopped))
            | ("on-failure", Some(RestartPolicy::OnFailure { maximum_retries: None })) => true,
            (
                expected,
                Some(RestartPolicy::OnFailure {
                    maximum_retries: Some(actual),
                }),
            ) => expected
                .strip_prefix("on-failure:")
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|value| value == actual.get()),
            _ => false,
        };
        if !matches {
            semantic_gaps.push(format!("restart:{service_name}"));
        }
    }
    for expected in &manifest.semantics.required_dependencies {
        let actual = service(application, &expected.service)?
            .dependencies()
            .iter()
            .find(|dependency| dependency.value().service().as_str() == expected.dependency)
            .map(boxferry::Sourced::value);
        let matches = actual.is_some_and(|dependency| {
            let condition = match dependency.condition().map(boxferry::Sourced::value) {
                None => "unspecified",
                Some(ServiceDependencyCondition::Started) => "started",
                Some(ServiceDependencyCondition::Healthy) => "healthy",
                Some(ServiceDependencyCondition::CompletedSuccessfully) => "completed-successfully",
                Some(ServiceDependencyCondition::Other(value)) => value.expose(),
                Some(_) => "unreviewed",
            };
            expected.assert_options == Some(false)
                || (condition == expected.condition
                    && dependency.required().map(|value| *value.value()) == expected.required
                    && dependency.restart().map(|value| *value.value()) == expected.restart)
        });
        if !matches {
            semantic_gaps.push(format!("dependency:{}:{}", expected.service, expected.dependency));
        }
    }
    for expected in &manifest.semantics.required_healthchecks {
        let actual = service(application, &expected.service)?
            .healthcheck()
            .and_then(|healthcheck| healthcheck.value().command())
            .map(|command| match command.value() {
                HealthcheckCommand::Exec(arguments) => (
                    "exec",
                    arguments
                        .iter()
                        .map(|value| value.expose().to_owned())
                        .collect::<Vec<_>>(),
                ),
                HealthcheckCommand::Shell(value) => ("shell", vec![value.expose().to_owned()]),
                _ => ("unreviewed", Vec::new()),
            });
        if actual
            .as_ref()
            .is_none_or(|(kind, command)| *kind != expected.command_kind || command != &expected.command)
        {
            semantic_gaps.push(format!("healthcheck:{}", expected.service));
        }
    }
    for name in &manifest.semantics.unpublished_services {
        if service(application, name)?
            .ports()
            .iter()
            .any(|port| port.value().published().is_some())
        {
            return Err(format!("service {name} must not publish a database/internal port"));
        }
    }
    for requirement in &manifest.semantics.exact_network_memberships {
        let (service_name, network_names) = requirement
            .split_once(':')
            .ok_or("invalid exact-network-memberships assertion")?;
        let expected = network_names.split(',').collect::<BTreeSet<_>>();
        let actual = service(application, service_name)?
            .networks()
            .iter()
            .map(|network| network.value().network().as_str())
            .collect::<BTreeSet<_>>();
        if actual != expected {
            semantic_gaps.push(format!(
                "network-memberships:{service_name}:expected={}:actual={}",
                expected.into_iter().collect::<Vec<_>>().join(","),
                actual.into_iter().collect::<Vec<_>>().join(",")
            ));
        }
    }
    semantic_gaps.extend(validate_process_settings(manifest, application)?);
    semantic_gaps.extend(validate_grants(manifest, application)?);
    semantic_gaps.extend(validate_port_bindings(manifest, application)?);
    semantic_gaps.extend(validate_volume_mounts(manifest, application)?);
    semantic_gaps.extend(validate_network_settings(manifest, application)?);
    Ok(semantic_gaps)
}

fn validate_process_settings(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_commands {
        let actual = service(application, &expected.service)?
            .command()
            .map(boxferry::Sourced::value);
        let matches = match (expected.kind.as_str(), actual) {
            ("exec", Some(Command::Exec(values))) => values
                .iter()
                .map(boxferry::ProtectedString::expose)
                .eq(expected.values.iter().map(String::as_str)),
            ("shell", Some(Command::Shell(value))) => expected.values.as_slice() == [value.expose()],
            ("empty", Some(Command::Empty)) | ("absent", None) => expected.values.is_empty(),
            _ => false,
        };
        if !matches {
            gaps.push(format!("command:{}", expected.service));
        }
    }
    for expected in &manifest.semantics.required_entrypoints {
        let actual = service(application, &expected.service)?
            .entrypoint()
            .map(boxferry::Sourced::value);
        let matches = match (expected.kind.as_str(), actual) {
            ("exec", Some(Entrypoint::Exec(values))) => values
                .iter()
                .map(boxferry::ProtectedString::expose)
                .eq(expected.values.iter().map(String::as_str)),
            ("shell", Some(Entrypoint::Shell(value))) => expected.values.as_slice() == [value.expose()],
            ("empty", Some(Entrypoint::Empty)) | ("absent", None) => expected.values.is_empty(),
            _ => false,
        };
        if !matches {
            gaps.push(format!("entrypoint:{}", expected.service));
        }
    }
    Ok(gaps)
}

fn validate_grants(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    for (kind, expectations) in [
        ("config", &manifest.semantics.required_config_grants),
        ("secret", &manifest.semantics.required_secret_grants),
    ] {
        for expected in expectations {
            let owner = service(application, &expected.service)?;
            let grants = if kind == "config" {
                owner.config_grants()
            } else {
                owner.secret_grants()
            };
            let matches = grants.iter().any(|grant| {
                let grant = grant.value();
                let syntax = match grant.syntax() {
                    ResourceGrantSyntax::Short => "short",
                    ResourceGrantSyntax::Long => "long",
                    _ => "unreviewed",
                };
                grant.source().expose() == expected.source
                    && syntax == expected.syntax
                    && grant.target().map(|value| value.value().expose()) == expected.target.as_deref()
                    && grant.uid().map(|value| value.value().expose()) == expected.uid.as_deref()
                    && grant.gid().map(|value| value.value().expose()) == expected.gid.as_deref()
                    && grant.mode().map(|value| value.value().expose()) == expected.mode.as_deref()
            });
            if !matches {
                gaps.push(format!("{kind}-grant:{}:{}", expected.service, expected.source));
            }
        }
    }
    Ok(gaps)
}

fn validate_port_bindings(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_port_bindings {
        let matches = service(application, &expected.service)?.ports().iter().any(|port| {
            let port = port.value();
            let protocol = format!("{:?}", port.protocol()).to_ascii_lowercase();
            port.container() == expected.container
                && port.published() == expected.published
                && port.host_address() == expected.host_address.as_deref()
                && protocol == expected.protocol
        });
        if !matches {
            gaps.push(format!(
                "port:{}:{}:{:?}",
                expected.service, expected.container, expected.published
            ));
        }
    }
    Ok(gaps)
}

fn validate_volume_mounts(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_volume_mounts {
        let relabel = match expected.selinux_relabel.as_deref() {
            None => None,
            Some("shared") => Some(SelinuxRelabel::Shared),
            Some("private") => Some(SelinuxRelabel::Private),
            Some(_) => return Err("unvalidated volume-mount SELinux relabel".into()),
        };
        let matches = service(application, &expected.service)?.mounts().iter().any(|mount| {
            let mount = mount.value();
            matches!(mount.source(), MountSource::Volume(volume) if volume.as_str() == expected.source)
                && mount.target() == expected.target
                && mount.read_only() == expected.read_only
                && mount.selinux_relabel() == relabel
        });
        if !matches {
            gaps.push(format!("volume-mount:{}:{}", expected.service, expected.target));
        }
    }
    Ok(gaps)
}

fn validate_network_settings(manifest: &ScenarioManifest, application: &Application) -> Result<Vec<String>, String> {
    let mut gaps = Vec::new();
    for expected in &manifest.semantics.required_networks {
        let network = application
            .networks()
            .iter()
            .find(|network| network.value().name().as_str() == expected.name)
            .map(boxferry::Sourced::value)
            .ok_or_else(|| format!("required network {} absent", expected.name))?;
        let ownership = match expected.ownership.as_str() {
            "application" => ResourceOwnership::Application,
            "external" => ResourceOwnership::External,
            "implicit" => ResourceOwnership::Implicit,
            "uncertain" => ResourceOwnership::Uncertain,
            _ => return Err("unvalidated network ownership".into()),
        };
        if network.ownership() != ownership {
            return Err(format!("network {} settings changed", expected.name));
        }
        if expected.internal.is_some() && network.internal().map(|value| *value.value()) != expected.internal {
            gaps.push(format!("network-internal:{}", expected.name));
        }
        if expected.ipv6.is_some() && network.ipv6().map(|value| *value.value()) != expected.ipv6 {
            gaps.push(format!("network-ipv6:{}", expected.name));
        }
        let rows = network.ipam_configs();
        let Some(expected_rows) = expected.ipam.as_deref() else {
            continue;
        };
        let Some(rows) = rows else {
            gaps.push(format!("network-ipam:{}", expected.name));
            continue;
        };
        if rows.len() != expected_rows.len() {
            return Err(format!("network {} IPAM row count changed", expected.name));
        }
        for (actual, expected) in rows.iter().zip(expected_rows) {
            let actual = actual.value();
            if actual.subnet().value().expose() != expected.subnet
                || actual.gateway().map(|value| value.value().expose()) != expected.gateway.as_deref()
                || actual.ip_range().map(|value| value.value().expose()) != expected.ip_range.as_deref()
            {
                return Err(format!("network {} IPAM changed", network.name().as_str()));
            }
        }
    }
    Ok(gaps)
}

pub(crate) fn validate_prerequisites(manifest: &ScenarioManifest, observed: &[String]) -> Result<(), String> {
    validate_diagnostics(&manifest.semantics.external_prerequisites, observed)
}

fn validate_native_input(input: &NativeInput, application: &str) -> Result<(), String> {
    match (input.importer.as_str(), input.podman.as_ref()) {
        ("podman", Some(podman)) => {
            if input.files.len() != 1 {
                return Err("Podman scenario input needs exactly one cassette".into());
            }
            pinned_version(&input.format_version)?;
            required("Podman application", &podman.application)?;
            if podman.application != application {
                return Err("Podman input application must match scenario application".into());
            }
            if podman.all != podman.selectors.is_empty() {
                return Err("Podman input must select exactly one of all resources or exact selectors".into());
            }
            let mut selectors = BTreeSet::new();
            for selector in &podman.selectors {
                if !matches!(
                    selector.kind.as_str(),
                    "container" | "pod" | "network" | "volume" | "image" | "secret"
                ) {
                    return Err("unsupported Podman scenario selector kind".into());
                }
                required("Podman exact selector", &selector.exact)?;
                if !selectors.insert((&selector.kind, &selector.exact)) {
                    return Err("duplicate Podman exact selector".into());
                }
            }
        }
        ("podman", None) => return Err("Podman scenario input lacks acquisition metadata".into()),
        (_, Some(_)) => return Err("Podman metadata belongs only to Podman inputs".into()),
        _ => {}
    }
    Ok(())
}

/// Read-only preflight for explicitly declared, fixture-relative file prerequisites.
pub(crate) fn observe_prerequisites(manifest: &ScenarioManifest, directory: &Path) -> Result<Vec<String>, String> {
    let mut missing = Vec::new();
    for prerequisite in &manifest.semantics.external_prerequisites {
        let path = prerequisite
            .strip_prefix("file:")
            .ok_or("unsupported prerequisite kind")?;
        if !safe_path(path) {
            return Err("unsafe prerequisite path".into());
        }
        if !directory.join(path).is_file() {
            missing.push(prerequisite.clone());
        }
    }
    Ok(missing)
}

fn service<'a>(application: &'a Application, name: &str) -> Result<&'a boxferry::Service, String> {
    application
        .services()
        .iter()
        .find(|service| service.value().name().as_str() == name)
        .map(boxferry::Sourced::value)
        .ok_or_else(|| format!("required service {name} is absent"))
}

fn semantics_is_empty(semantics: &Semantics) -> bool {
    semantics.selected_resources.is_empty()
        && semantics.excluded_resources.is_empty()
        && semantics.ownership_boundaries.is_empty()
        && semantics.shared_boundaries.is_empty()
        && semantics.required_mounts.is_empty()
        && semantics.required_tmpfs.is_empty()
        && semantics.required_bind_mounts.is_empty()
        && semantics.required_environment.is_empty()
        && semantics.required_environment_order.is_empty()
        && semantics.required_dependencies.is_empty()
        && semantics.required_healthchecks.is_empty()
        && semantics.required_ports.is_empty()
        && semantics.required_restart.is_empty()
        && semantics.required_runtime_names.is_empty()
        && semantics.unpublished_services.is_empty()
        && semantics.external_prerequisites.is_empty()
        && semantics.expected_diagnostics.is_empty()
        && semantics.required_networks.is_empty()
        && semantics.exact_network_memberships.is_empty()
        && semantics.required_groups.is_empty()
        && semantics.required_commands.is_empty()
        && semantics.required_entrypoints.is_empty()
        && semantics.required_config_grants.is_empty()
        && semantics.required_secret_grants.is_empty()
        && semantics.required_image_acquisitions.is_empty()
        && semantics.required_image_builds.is_empty()
        && semantics.required_image_sources.is_empty()
        && semantics.required_port_bindings.is_empty()
        && semantics.required_volume_mounts.is_empty()
}

fn validate_semantics(semantics: &Semantics) -> Result<(), String> {
    if semantics.selected_resources.is_empty() {
        return Err("scenario must select resources".into());
    }
    validate_selected_volume_coverage(semantics)?;
    validate_typed_semantics(semantics)?;
    for prerequisite in &semantics.external_prerequisites {
        let path = prerequisite
            .strip_prefix("file:")
            .ok_or("unsupported prerequisite kind")?;
        if !safe_path(path) {
            return Err("unsafe prerequisite path".into());
        }
    }
    for values in [
        &semantics.selected_resources,
        &semantics.excluded_resources,
        &semantics.ownership_boundaries,
        &semantics.shared_boundaries,
        &semantics.required_mounts,
        &semantics.required_tmpfs,
        &semantics.required_environment,
        &semantics.required_environment_order,
        &semantics.required_ports,
        &semantics.required_restart,
        &semantics.required_runtime_names,
        &semantics.unpublished_services,
        &semantics.external_prerequisites,
        &semantics.exact_network_memberships,
        &semantics.required_image_acquisitions,
        &semantics.required_image_builds,
    ] {
        unique("semantic assertions", values.iter().map(String::as_str))?;
    }
    unique(
        "ordered environment assertions",
        semantics.required_environment_order.iter().map(String::as_str),
    )?;
    let mut binds = BTreeSet::new();
    for bind in &semantics.required_bind_mounts {
        for value in [&bind.service, &bind.source, &bind.target] {
            required("bind-mount assertion", value)?;
        }
        if !bind.source.starts_with('/') || !bind.target.starts_with('/') {
            return Err("bind-mount assertions require absolute source and target".into());
        }
        if !matches!(bind.selinux_relabel.as_str(), "shared" | "private") {
            return Err("bind-mount SELinux relabel must be shared or private".into());
        }
        if !binds.insert((&bind.service, &bind.source, &bind.target)) {
            return Err("duplicate bind-mount assertion".into());
        }
    }
    let mut networks = BTreeSet::new();
    for network in &semantics.required_networks {
        required("network assertion", &network.name)?;
        if !matches!(
            network.ownership.as_str(),
            "application" | "external" | "implicit" | "uncertain"
        ) {
            return Err("unknown network ownership assertion".into());
        }
        if !networks.insert(&network.name) {
            return Err("network assertions require unique names".into());
        }
        let mut subnets = BTreeSet::new();
        for row in network.ipam.iter().flatten() {
            required("network IPAM subnet", &row.subnet)?;
            if !subnets.insert(&row.subnet) {
                return Err("duplicate network IPAM subnet".into());
            }
            for value in [row.gateway.as_deref(), row.ip_range.as_deref()].into_iter().flatten() {
                required("network IPAM value", value)?;
            }
        }
    }
    for resource in semantics.selected_resources.iter().chain(&semantics.excluded_resources) {
        let (kind, name) = resource
            .split_once(':')
            .ok_or("resource assertion requires kind:name")?;
        if !matches!(
            kind,
            "service" | "volume" | "network" | "config" | "secret" | "group" | "image-acquisition" | "image-build"
        ) || name.is_empty()
        {
            return Err(format!("unknown resource assertion {resource}"));
        }
    }
    if semantics
        .selected_resources
        .iter()
        .any(|resource| semantics.excluded_resources.contains(resource))
    {
        return Err("resource cannot be both selected and excluded".into());
    }
    Ok(())
}

#[allow(clippy::too_many_lines, reason = "keeps typed semantic schema coverage together")]
fn validate_typed_semantics(semantics: &Semantics) -> Result<(), String> {
    let selected_services = semantics
        .selected_resources
        .iter()
        .filter_map(|resource| resource.strip_prefix("service:"))
        .collect::<BTreeSet<_>>();
    let selected_groups = semantics
        .selected_resources
        .iter()
        .filter_map(|resource| resource.strip_prefix("group:"))
        .collect::<BTreeSet<_>>();
    let selected_networks = semantics
        .selected_resources
        .iter()
        .filter_map(|resource| resource.strip_prefix("network:"))
        .collect::<BTreeSet<_>>();
    let mut exact_membership_services = BTreeSet::new();
    for membership in &semantics.exact_network_memberships {
        let (service, networks) = membership
            .split_once(':')
            .ok_or("exact network memberships require service:network[,network]")?;
        if !exact_membership_services.insert(service) {
            return Err("exact network memberships require one assertion per service".into());
        }
        let networks = networks.split(',').collect::<Vec<_>>();
        if networks.is_empty()
            || networks.iter().any(|network| network.is_empty())
            || networks.iter().collect::<BTreeSet<_>>().len() != networks.len()
        {
            return Err("exact network memberships require distinct non-empty networks".into());
        }
        if !selected_services.contains(service) || networks.iter().any(|network| !selected_networks.contains(network)) {
            return Err("exact network memberships must reference selected resources".into());
        }
    }
    let expected_groups = semantics
        .required_groups
        .iter()
        .map(|group| group.name.as_str())
        .collect::<BTreeSet<_>>();
    if selected_groups != expected_groups {
        return Err("selected service-group coverage differs from group assertions".into());
    }
    for group in &semantics.required_groups {
        if !matches!(
            group.ownership.as_str(),
            "application" | "external" | "implicit" | "uncertain"
        ) || group.members.is_empty()
        {
            return Err("service-group assertion requires reviewed ownership and members".into());
        }
        let members = group.members.iter().map(String::as_str).collect::<BTreeSet<_>>();
        if members.len() != group.members.len() || !members.is_subset(&selected_services) {
            return Err("service-group membership must be unique and selected".into());
        }
    }

    for (prefix, expected) in [
        ("image-acquisition:", &semantics.required_image_acquisitions),
        ("image-build:", &semantics.required_image_builds),
    ] {
        let selected = semantics
            .selected_resources
            .iter()
            .filter_map(|resource| resource.strip_prefix(prefix))
            .collect::<BTreeSet<_>>();
        let expected = expected.iter().map(String::as_str).collect::<BTreeSet<_>>();
        if selected != expected {
            return Err("selected image graph coverage differs from graph assertions".into());
        }
    }
    let mut image_sources = BTreeSet::new();
    for source in &semantics.required_image_sources {
        if !selected_services.contains(source.service.as_str())
            || !matches!(source.kind.as_str(), "literal" | "acquisition" | "build" | "rootfs")
            || source.value.is_empty()
            || !image_sources.insert(source.service.as_str())
        {
            return Err("invalid or duplicate service image-source assertion".into());
        }
    }

    for (label, expected) in [
        ("command", &semantics.required_commands),
        ("entrypoint", &semantics.required_entrypoints),
    ] {
        let mut services = BTreeSet::new();
        for process in expected {
            if !selected_services.contains(process.service.as_str())
                || !matches!(process.kind.as_str(), "exec" | "shell" | "empty" | "absent")
                || matches!(process.kind.as_str(), "shell") && process.values.len() != 1
                || matches!(process.kind.as_str(), "empty" | "absent") && !process.values.is_empty()
                || !services.insert(process.service.as_str())
            {
                return Err(format!("invalid or duplicate {label} assertion"));
            }
        }
    }

    for (label, grants) in [
        ("config", &semantics.required_config_grants),
        ("secret", &semantics.required_secret_grants),
    ] {
        let mut identities = BTreeSet::new();
        for grant in grants {
            if !selected_services.contains(grant.service.as_str())
                || !matches!(grant.syntax.as_str(), "short" | "long")
                || grant.source.is_empty()
                || !identities.insert((grant.service.as_str(), grant.source.as_str()))
            {
                return Err(format!("invalid or duplicate {label}-grant assertion"));
            }
        }
    }

    let mut ports = BTreeSet::new();
    for port in &semantics.required_port_bindings {
        if !selected_services.contains(port.service.as_str())
            || port.container == 0
            || !matches!(port.protocol.as_str(), "tcp" | "udp" | "sctp")
            || !ports.insert((
                port.service.as_str(),
                port.container,
                port.published,
                port.host_address.as_deref(),
                port.protocol.as_str(),
            ))
        {
            return Err("invalid or duplicate port-binding assertion".into());
        }
    }
    let mut mounts = BTreeSet::new();
    for mount in &semantics.required_volume_mounts {
        if !selected_services.contains(mount.service.as_str())
            || mount.source.is_empty()
            || mount.target.is_empty()
            || mount
                .selinux_relabel
                .as_deref()
                .is_some_and(|value| !matches!(value, "shared" | "private"))
            || !mounts.insert((mount.service.as_str(), mount.source.as_str(), mount.target.as_str()))
        {
            return Err("invalid or duplicate volume-mount assertion".into());
        }
    }
    Ok(())
}

fn validate_selected_volume_coverage(semantics: &Semantics) -> Result<(), String> {
    let selected_volumes = semantics
        .selected_resources
        .iter()
        .filter_map(|resource| resource.strip_prefix("volume:"))
        .collect::<BTreeSet<_>>();
    let selected_services = semantics
        .selected_resources
        .iter()
        .filter_map(|resource| resource.strip_prefix("service:"))
        .collect::<BTreeSet<_>>();

    let mut ownership = BTreeSet::new();
    for boundary in &semantics.ownership_boundaries {
        let (kind, name, expected) = split_three(boundary)?;
        if kind != "volume" || !matches!(expected, "application" | "external" | "implicit" | "uncertain") {
            return Err("schema 1 volume ownership assertion is invalid".into());
        }
        if !ownership.insert(name) {
            return Err(format!("volume {name} has duplicate ownership assertions"));
        }
    }
    if ownership != selected_volumes {
        return Err(format!(
            "selected volume ownership coverage differs: selected {selected_volumes:?}, asserted {ownership:?}"
        ));
    }

    let mut relationships = BTreeSet::new();
    for boundary in &semantics.shared_boundaries {
        let (kind, name, consumers) = split_three(boundary)?;
        if kind != "volume" {
            return Err("schema 1 volume relationship assertion is invalid".into());
        }
        if !relationships.insert(name) {
            return Err(format!("volume {name} has duplicate relationship assertions"));
        }
        let consumers = consumers
            .strip_prefix("consumers=")
            .ok_or("missing consumer declaration")?;
        if consumers == "none" {
            continue;
        }
        let mut declared = BTreeSet::new();
        for consumer in consumers.split(',') {
            if consumer.is_empty() || !declared.insert(consumer) {
                return Err(format!("volume {name} has invalid consumer assertions"));
            }
            if !selected_services.contains(consumer) {
                return Err(format!("volume {name} consumer {consumer} is not a selected service"));
            }
        }
    }
    if relationships != selected_volumes {
        return Err(format!(
            "selected volume relationship coverage differs: selected {selected_volumes:?}, asserted {relationships:?}"
        ));
    }
    Ok(())
}

fn required(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(format!("{name} must be non-empty and NUL-free"))
    } else {
        Ok(())
    }
}

fn pinned_version(value: &str) -> Result<(), String> {
    // Scenario 1 deliberately uses exact numeric versions; new version grammars need schema review.
    value
        .parse::<PlatformVersion>()
        .map(|_| ())
        .map_err(|error| format!("unversioned scenario: {error}"))
}

fn unique<'a>(label: &str, values: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        required(label, value)?;
        if !seen.insert(value) {
            return Err(format!("duplicate {label}"));
        }
    }
    Ok(())
}

fn split_three(value: &str) -> Result<(&str, &str, &str), String> {
    let (first, rest) = value
        .split_once(':')
        .ok_or_else(|| format!("invalid assertion {value}"))?;
    let (second, third) = rest
        .split_once(':')
        .ok_or_else(|| format!("invalid assertion {value}"))?;
    if first.is_empty() || second.is_empty() || third.is_empty() {
        return Err("empty assertion component".into());
    }
    Ok((first, second, third))
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
}

fn contained_file(directory: &Path, path: &str) -> Result<(), String> {
    if !safe_path(path) {
        return Err("unsafe scenario source path".into());
    }
    let root = directory.canonicalize().map_err(|error| error.to_string())?;
    let file = directory.join(path).canonicalize().map_err(|error| error.to_string())?;
    if !file.starts_with(root) || !file.is_file() {
        return Err("scenario source escapes fixture directory".into());
    }
    Ok(())
}
