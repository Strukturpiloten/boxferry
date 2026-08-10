//! Format-independent application model for `BoxFerry`.
//!
//! The model describes container application intent without exposing Compose,
//! Quadlet, Podman, Docker, or Kubernetes types. Ordered collections and source
//! provenance remain available to conversion adapters.

mod application;
mod image;
mod image_artifact;
mod provenance;
mod value;

pub use application::{
    Application, Command, Config, ConfigMaterial, Device, EnvironmentFile, EnvironmentFileFormat,
    EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable, Healthcheck, HealthcheckCommand, HealthcheckDuration,
    HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier, KernelParameter, MetadataLabel,
    ModelError, Mount, MountSource, Network, NetworkAttachment, Port, Protocol, ResourceGrant, ResourceGrantSyntax,
    ResourceLimit, ResourceOwnership, RestartPolicy, Secret, SecretMaterial, SecurityOption, SelinuxRelabel, Service,
    ServiceDependency, ServiceDependencyCondition, ServiceGroup, Volume,
};
pub use image::ImageReference;
pub use image_artifact::{
    BuildAttestation, BuildContext, BuildSettingValues, BuildSourceDeclaration, BuildSyntax, ImageAcquisition,
    ImageAcquisitionSetting, ImageArtifactAssignment, ImageBuild, ImageBuildSetting, SourceBuildSecret,
    SourceBuildSetting,
};
pub use provenance::{Provenance, ProvenanceKind, SourceId, SourceSpan, Sourced};
pub use value::ProtectedString;
