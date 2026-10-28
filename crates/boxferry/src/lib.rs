//! Reusable high-level APIs for `BoxFerry` conversions.
//!
//! This facade exposes the same neutral model and conversion engine used by the `boxferry`
//! command-line application. Compose, Docker, Podman, and Quadlet adapters are available through
//! additive features and are selected by the default CLI. Embedded callers can disable defaults
//! and select individual adapters. Every conversion uses the importer → neutral model → exporter
//! pipeline, including same-format routes.
//!
//! Planning does not execute generated commands or mutate a runtime. Docker and Podman acquisition
//! require explicit caller-supplied read-only transports. Docker output is an inert request plan;
//! Podman output is a reviewable deployment plan and command script.
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

/// Create-new artifact output shared by conversion and transport extraction.
pub mod output;
/// Bounded single-file Quadlet transport and explicit extraction.
#[cfg(feature = "quadlet")]
pub mod quadlet_bundle;
/// Versioned, presentation-independent conversion reports.
pub mod report;

/// Compose native-model adapter APIs, enabled by the additive `compose` feature.
#[cfg(feature = "compose")]
pub use boxferry_compose as compose;
/// Docker source and inert target adapter APIs, enabled by the additive `docker` feature.
#[cfg(feature = "docker")]
pub use boxferry_docker as docker;
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
    ImageBuild, ImageBuildSetting, ImageReference, KernelParameter, Logging, LoggingOption, METADATA_MAX_BYTES,
    METADATA_MAX_DEPTH, MetadataEntry, MetadataError, MetadataLabel, MetadataNode, MetadataScope, MetadataValue,
    ModelError, Mount, MountSource, Network, NetworkAttachment, NetworkDriverOption, NetworkIpamConfig, Port,
    ProtectedString, Protocol, Provenance, ProvenanceKind, PullPolicy, ReloadAction, ResourceGrant,
    ResourceGrantSyntax, ResourceLimit, ResourceOwnership, RestartPolicy, RetainedNativeEvidence,
    RetainedNativeEvidenceEvent, RetainedNativeEvidenceSubject, Secret, SecretMaterial, SecurityOption, SelinuxRelabel,
    Service, ServiceDependency, ServiceDependencyCondition, ServiceGroup, ServiceGroupRuntime, SourceBuildSecret,
    SourceBuildSetting, SourceId, SourceSpan, Sourced, StartupNotification, StopTimeout, VendorMetadata, Volume,
    VolumeImageSource,
};

#[cfg(feature = "compose")]
pub use boxferry_compose::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeFindingStage,
    ComposeImporter, ComposeRuntime, ComposeSource, DOCKER_COMPOSE_TARGET, PODMAN_COMPOSE_TARGET,
    QuadletDeferredOptions,
};
#[cfg(feature = "docker")]
pub use boxferry_docker::{
    DOCKER_TARGET, DockerDependencyDecision, DockerEnvironmentPolicy, DockerExporter, DockerImporter, DockerOutput,
    DockerPromotionPolicy, DockerPublicLabelDecision, DockerPublicLabelDecisionError, DockerPublicLabelDecisions,
    DockerPublicLabelResourceKind, DockerResourceAction, DockerResourceDecision, DockerResourceDecisionError,
    DockerResourceDecisions, DockerResourceKind, DockerSelection, DockerSource, DockerSourceError,
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

/// Acquires and decodes one explicitly selected, read-only Docker source.
///
/// The caller supplies the Unix endpoint, `DockerLens` selector, limits, and cancellation flag.
/// This function performs no ambient connection lookup and sends no mutating request. The
/// decoded source retains observed Engine/API/root-mode evidence separately from any target.
/// Source promotion remains conservative; acquisition grants no artifact or loss authorization.
///
/// # Errors
///
/// Returns an acquisition or decoding error without exposing endpoint paths or native values.
#[cfg(feature = "docker")]
pub fn acquire_docker_source(
    application_name: Identifier,
    endpoint: &docker::docker_lens::acquisition::Endpoint,
    selector: docker::docker_lens::acquisition::Selector,
    limits: docker::docker_lens::acquisition::Limits,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<DockerSource, DockerSourceError> {
    let selection = if matches!(&selector, docker::docker_lens::acquisition::Selector::Discovery) {
        DockerSelection::Discovery
    } else {
        DockerSelection::Explicit
    };
    let capture = docker::docker_lens::acquisition::acquire(endpoint, selector, limits, cancelled)?;
    let inventory = docker::docker_lens::decoder::decode_capture(&capture)?;
    Ok(DockerSource::new(application_name, inventory).with_selection(selection))
}

#[cfg(feature = "quadlet")]
pub use boxferry_quadlet::{
    QuadletDocumentInput, QuadletExporter, QuadletExporterError, QuadletFile, QuadletGroupingPolicy, QuadletImporter,
    QuadletOutput, QuadletParseDiagnostic, QuadletParseDiagnosticLabel, QuadletParseDiagnosticOrigin,
    QuadletParseDiagnosticSeverity, QuadletParseError, QuadletParseFailure, QuadletParseFailureStage,
    QuadletParseResult, QuadletSource,
};

pub use boxferry_model::{
    DEFERRED_MAX_BYTES, DEFERRED_MAX_SEGMENTS, DeferredExpression, DeferredSegment, ServiceEnvironmentAssignment,
    ServiceEnvironmentFile, ServiceEnvironmentName, ServiceExecutionEnvironment,
};
