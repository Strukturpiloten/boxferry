//! Authored expectations and run-derived observations for migration scenarios.
#![allow(dead_code)] // Shared by separately compiled integration-test entry points.

use std::{collections::BTreeSet, path::Path};

use boxferry::LossPolicy;
use boxferry::{
    Application, ConversionKind, ConversionOutcome, Diagnostic, EnvironmentValue, MountSource, PlatformVersion,
    ResourceOwnership, TargetProfile,
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
    pub semantics: Semantics,
    pub evidence: Vec<RouteExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplicationVersion {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Component {
    pub name: String,
    pub version: String,
    pub image: String,
    pub digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct NativeInput {
    pub id: String,
    pub importer: String,
    pub files: Vec<String>,
    pub format_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Provenance {
    pub source: String,
    pub license: String,
    pub privacy_reviewed: bool,
    pub test_data: String,
    pub image_evidence: String,
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
    pub artifacts: Vec<String>,
    pub native_validation: Dimension,
    pub runtime_probe: Dimension,
    pub reimport: Dimension,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub allowed_losses: Vec<LossTuple>,
    #[serde(default)]
    pub semantic_gaps: Vec<String>,
    #[serde(default)]
    pub unavailable_prerequisites: Vec<String>,
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
    pub required_environment: Vec<String>,
    #[serde(default)]
    pub required_ports: Vec<String>,
    #[serde(default)]
    pub unpublished_services: Vec<String>,
    #[serde(default)]
    pub external_prerequisites: Vec<String>,
    #[serde(default)]
    pub expected_diagnostics: Vec<String>,
}

pub(crate) fn validate_manifest(manifest: &ScenarioManifest, directory: &Path) -> Result<(), String> {
    if manifest.schema != 1 {
        return Err("scenario schema must be exactly 1".into());
    }
    required("scenario id", &manifest.id)?;
    if directory.file_name().and_then(|name| name.to_str()) != Some(&manifest.id) {
        return Err("scenario id must match its directory".into());
    }
    required("application name", &manifest.application.name)?;
    pinned_version(&manifest.application.version)?;
    unique(
        "components",
        manifest.components.iter().map(|entry| entry.name.as_str()),
    )?;
    if manifest.components.is_empty() || manifest.native_inputs.is_empty() || manifest.evidence.is_empty() {
        return Err("scenario requires components, inputs, and route expectations".into());
    }
    for component in &manifest.components {
        required("component name", &component.name)?;
        pinned_version(&component.version)?;
        required("component image", &component.image)?;
        if component.digest.len() != 71
            || !component.digest.starts_with("sha256:")
            || !component.digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(format!(
                "component {} requires an exact lowercase SHA-256 digest",
                component.name
            ));
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
        for file in &input.files {
            contained_file(directory, file)?;
        }
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
    validate_semantics(&manifest.semantics)?;
    validate_routes(manifest)
}

fn validate_routes(manifest: &ScenarioManifest) -> Result<(), String> {
    let mut routes = BTreeSet::new();
    for route in &manifest.evidence {
        if !routes.insert((&route.input, &route.exporter)) {
            return Err("duplicate input/exporter expectation".into());
        }
        route.target()?;
        route.policy()?;
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
        unique("artifacts", route.artifacts.iter().map(String::as_str))?;
        if route.artifacts.iter().any(|file| !safe_path(file)) {
            return Err("unsafe artifact path".into());
        }
        let mut losses = BTreeSet::new();
        for loss in &route.allowed_losses {
            required("loss rule", &loss.rule)?;
            required("loss subject", &loss.subject)?;
            if !matches!(loss.decision.as_str(), "approximate" | "unsupported" | "invalid")
                || loss.version_scope != route.version_scope()
                || !losses.insert(loss)
            {
                return Err("loss tuples must be unique, non-exact and scoped to this route target".into());
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
    let mut observed = Vec::new();
    for outcome in actual.iter().filter(|outcome| outcome.kind() != ConversionKind::Exact) {
        observed.push(LossTuple {
            rule: outcome
                .diagnostic()
                .ok_or("non-exact outcome lacks a rule")?
                .as_str()
                .to_owned(),
            subject: outcome.subject().to_owned(),
            decision: match outcome.kind() {
                ConversionKind::Approximate => "approximate",
                ConversionKind::Unsupported => "unsupported",
                ConversionKind::Invalid => "invalid",
                _ => return Err("unreviewed conversion kind".into()),
            }
            .into(),
            version_scope: scope.into(),
        });
    }
    let mut wanted = expected.to_vec();
    wanted.sort();
    observed.sort();
    if wanted != observed {
        return Err(format!(
            "loss tuples differ: expected {wanted:?}, observed {observed:?}"
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
) -> Result<(), String> {
    validate_resources(manifest, application)?;
    validate_images(manifest, application)?;
    validate_volume_boundaries(manifest, application)?;
    validate_service_settings(manifest, application)
}

fn validate_resources(manifest: &ScenarioManifest, application: &Application) -> Result<(), String> {
    if !application.service_groups().is_empty()
        || !application.image_acquisitions().is_empty()
        || !application.image_builds().is_empty()
    {
        return Err(
            "schema 1 needs explicit assertions before accepting groups or image build/acquisition graphs".into(),
        );
    }
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
    let expected = manifest
        .semantics
        .selected_resources
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "selected resources differ: expected {expected:?}, observed {actual:?}"
        ));
    }
    if manifest
        .semantics
        .excluded_resources
        .iter()
        .any(|resource| actual.contains(resource))
    {
        return Err("an excluded resource was selected".into());
    }
    Ok(())
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
    if selected != components {
        return Err("component inventory differs from selected services".into());
    }
    for service in application.services() {
        let name = service.value().name().as_str();
        let component = manifest
            .components
            .iter()
            .find(|component| component.name == name)
            .ok_or_else(|| format!("selected service {name} lacks an independent component version"))?;
        let image = service
            .value()
            .image()
            .ok_or_else(|| format!("service {name} has no image"))?;
        let expected = format!("{}@{}", component.image, component.digest);
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
        let expected = consumers
            .strip_prefix("consumers=")
            .ok_or("missing consumer declaration")?
            .split(',')
            .collect::<BTreeSet<_>>();
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

fn validate_service_settings(manifest: &ScenarioManifest, application: &Application) -> Result<(), String> {
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
            return Err(format!("required port {requirement} changed"));
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
    Ok(())
}

pub(crate) fn validate_prerequisites(manifest: &ScenarioManifest, observed: &[String]) -> Result<(), String> {
    validate_diagnostics(&manifest.semantics.external_prerequisites, observed)
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

fn validate_semantics(semantics: &Semantics) -> Result<(), String> {
    if semantics.selected_resources.is_empty() {
        return Err("scenario must select resources".into());
    }
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
        &semantics.required_environment,
        &semantics.required_ports,
        &semantics.unpublished_services,
        &semantics.external_prerequisites,
    ] {
        unique("semantic assertions", values.iter().map(String::as_str))?;
    }
    for resource in semantics.selected_resources.iter().chain(&semantics.excluded_resources) {
        let (kind, name) = resource
            .split_once(':')
            .ok_or("resource assertion requires kind:name")?;
        if !matches!(kind, "service" | "volume" | "network" | "config" | "secret") || name.is_empty() {
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
