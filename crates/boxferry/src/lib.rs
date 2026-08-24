//! Reusable high-level APIs for `BoxFerry` conversions.
//!
//! This facade exposes the same neutral model and conversion engine used by the `boxferry`
//! command-line application. Compose, Podman, and Quadlet adapters are available through additive
//! features. Every conversion uses the importer → neutral model → exporter pipeline.
//!
//! Planning does not execute generated commands or mutate a runtime. Podman acquisition requires
//! an explicit caller-supplied read-only transport, and Podman output is an inert deployment plan.
//!
//! # Example
//!
//! ```
//! use boxferry::{
//!     Application, Identifier, InMemoryAdapter, LossPolicy, PlatformVersion, TargetProfile,
//!     convert,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let application = Application::new(Identifier::new("example")?);
//! let adapter = InMemoryAdapter::exact("target document".to_owned());
//! let target = TargetProfile::new("podman", PlatformVersion::new(5, 4, 0), None)?;
//! let result = convert(
//!     &adapter,
//!     &application,
//!     &adapter,
//!     &target,
//!     LossPolicy::ExactOnly,
//! )?;
//!
//! assert_eq!(result.output().map(String::as_str), Some("target document"));
//! # Ok(())
//! # }
//! ```

/// Versioned, presentation-independent conversion reports.
pub mod report;

/// Compose native-model adapter APIs, enabled by the additive `compose` feature.
#[cfg(feature = "compose")]
pub use boxferry_compose as compose;
/// Conversion planning, policy, capability, and diagnostic APIs.
pub use boxferry_engine as engine;
/// Format-independent container application model APIs.
pub use boxferry_model as model;
/// Podman semantic adapter and `PodmanLens` APIs, enabled by the additive `podman` feature.
#[cfg(feature = "podman")]
pub use boxferry_podman as podman;
/// Quadlet native-model adapter APIs, enabled by the additive `quadlet` feature.
#[cfg(feature = "quadlet")]
pub use boxferry_quadlet as quadlet;

pub use boxferry_engine::{
    ConversionError, ConversionKind, ConversionOutcome, ConversionPlan, ConversionResult, Diagnostic, DiagnosticCode,
    DiagnosticField, DiagnosticRule, DiagnosticValue, ExportAdapter, ImportAdapter, ImportResult, InMemoryAdapter,
    LossPolicy, NativeFinding, NativeFindingLabel, NativeFindingLabelKind, ParsePlatformVersionError, PlatformVersion,
    RULES, RuleId, Severity, TargetProfile, VersionRange, convert, convert_imported, find_rule,
};
pub use boxferry_model::{
    Annotation, Application, ArtifactDependency, ArtifactDependencyNode, BuildAttestation, BuildContext,
    BuildSettingValues, BuildSourceDeclaration, BuildSyntax, Command, Config, ConfigMaterial, Device, Entrypoint,
    EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable, ExposedPort,
    GroupExitPolicy, Healthcheck, HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress,
    HostAddressKind, HostMapping, Identifier, ImageAcquisition, ImageAcquisitionSetting, ImageArtifactAssignment,
    ImageBuild, ImageBuildSetting, ImageReference, KernelParameter, Logging, LoggingOption, MetadataLabel, ModelError,
    Mount, MountSource, Network, NetworkAttachment, NetworkDriverOption, NetworkIpamConfig, Port, ProtectedString,
    Protocol, Provenance, ProvenanceKind, PullPolicy, ReloadAction, ResourceGrant, ResourceGrantSyntax, ResourceLimit,
    ResourceOwnership, RestartPolicy, Secret, SecretMaterial, SecurityOption, SelinuxRelabel, Service,
    ServiceDependency, ServiceDependencyCondition, ServiceGroup, ServiceGroupRuntime, SourceBuildSecret,
    SourceBuildSetting, SourceId, SourceSpan, Sourced, StartupNotification, StopTimeout, Volume, VolumeImageSource,
};

#[cfg(feature = "compose")]
pub use boxferry_compose::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeFindingStage,
    ComposeImporter, ComposeRuntime, ComposeSource, DOCKER_COMPOSE_TARGET, PODMAN_COMPOSE_TARGET,
};
#[cfg(feature = "podman")]
pub use boxferry_podman::{
    PODMAN_TARGET, PodmanExporter, PodmanImporter, PodmanOutput, PodmanPromotionPolicy, PodmanSource,
    PodmanTargetError, ResolvedPodmanTarget, resolve_podman_target, reviewed_podman_versions,
};

/// Acquires and discovers one explicit read-only Podman source.
///
/// The caller owns the transport and discovery request. This helper performs no ambient
/// connection lookup, starts no process, and sends only requests accepted by the supplied
/// [`podman::podman_lens::LibpodTransport`]. The CLI uses this same public boundary with
/// `PodmanLens`'s read-only Unix transport.
///
/// # Errors
///
/// Returns a `PodmanLens` diagnostic when acquisition or discovery cannot complete.
#[cfg(feature = "podman")]
pub async fn acquire_podman_source(
    application_name: Identifier,
    transport: &dyn podman::podman_lens::LibpodTransport,
    acquisition: podman::podman_lens::AcquisitionOptions,
    discovery: &podman::podman_lens::DiscoveryRequest,
    promotion: PodmanPromotionPolicy,
) -> Result<PodmanSource, podman::podman_lens::Diagnostic> {
    let inventory = podman::podman_lens::acquire_inventory(transport, acquisition).await?;
    let graph = podman::podman_lens::discover(&inventory, discovery)?;
    Ok(PodmanSource::new(application_name, inventory, graph).with_promotion_policy(promotion))
}

#[cfg(feature = "quadlet")]
pub use boxferry_quadlet::{
    QuadletDocumentInput, QuadletExporter, QuadletExporterError, QuadletFile, QuadletGroupingPolicy, QuadletImporter,
    QuadletOutput, QuadletParseDiagnostic, QuadletParseDiagnosticLabel, QuadletParseDiagnosticOrigin,
    QuadletParseDiagnosticSeverity, QuadletParseError, QuadletParseFailure, QuadletParseFailureStage,
    QuadletParseResult, QuadletSource,
};
