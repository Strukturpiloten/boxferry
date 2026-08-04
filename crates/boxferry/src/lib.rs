//! Reusable high-level APIs for `BoxFerry` conversions.
//!
//! This facade exposes the same model and conversion-engine crates used by the
//! `boxferry` command-line application. Format adapters will be re-exported here behind
//! documented additive features when their public contracts are implemented.
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

/// Compose native-model adapter APIs, enabled by the additive `compose` feature.
#[cfg(feature = "compose")]
pub use boxferry_compose as compose;
/// Conversion planning, policy, capability, and diagnostic APIs.
pub use boxferry_engine as engine;
/// Format-independent container application model APIs.
pub use boxferry_model as model;
/// Quadlet native-model adapter APIs, enabled by the additive `quadlet` feature.
#[cfg(feature = "quadlet")]
pub use boxferry_quadlet as quadlet;

pub use boxferry_engine::{
    ConversionError, ConversionKind, ConversionOutcome, ConversionPlan, ConversionResult, Diagnostic, DiagnosticCode,
    DiagnosticField, DiagnosticValue, ExportAdapter, ImportAdapter, ImportResult, InMemoryAdapter, LossPolicy,
    ParsePlatformVersionError, PlatformVersion, Severity, TargetProfile, VersionRange, convert,
};
pub use boxferry_model::{
    Application, Command, Config, ConfigMaterial, EnvironmentValue, EnvironmentVariable, Healthcheck,
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier,
    ImageReference, ModelError, Mount, MountSource, Network, NetworkAttachment, Port, ProtectedString, Protocol,
    Provenance, ProvenanceKind, ResourceGrant, ResourceGrantSyntax, ResourceOwnership, Secret, SecretMaterial,
    SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition, SourceId, SourceSpan, Sourced, Volume,
};

#[cfg(feature = "compose")]
pub use boxferry_compose::{ComposeImporter, ComposeSource};
#[cfg(feature = "quadlet")]
pub use boxferry_quadlet::{QuadletExporter, QuadletExporterError, QuadletFile, QuadletGroupingPolicy, QuadletOutput};
