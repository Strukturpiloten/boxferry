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
    Annotation, Application, ArtifactDependency, ArtifactDependencyNode, Command, Config, ConfigMaterial, Device,
    Entrypoint, EnvironmentFile, EnvironmentFileFormat, EnvironmentFileSyntax, EnvironmentValue, EnvironmentVariable,
    ExposedPort, GroupExitPolicy, Healthcheck, HealthcheckCommand, HealthcheckDuration, HealthcheckRetries,
    HostAddress, HostAddressKind, HostMapping, Identifier, KernelParameter, Logging, LoggingOption, MetadataLabel,
    ModelError, Mount, MountSource, Network, NetworkAttachment, NetworkDriverOption, NetworkIpamConfig, Port, Protocol,
    PullPolicy, ReloadAction, ResourceGrant, ResourceGrantSyntax, ResourceLimit, ResourceOwnership, RestartPolicy,
    Secret, SecretMaterial, SecurityOption, SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition,
    ServiceGroup, ServiceGroupRuntime, StartupNotification, StopTimeout, Volume, VolumeImageSource,
};
pub use image::ImageReference;
pub use image_artifact::{
    BuildAttestation, BuildContext, BuildSettingValues, BuildSourceDeclaration, BuildSyntax, ImageAcquisition,
    ImageAcquisitionSetting, ImageArtifactAssignment, ImageBuild, ImageBuildSetting, SourceBuildSecret,
    SourceBuildSetting,
};
pub use provenance::{Provenance, ProvenanceKind, SourceId, SourceSpan, Sourced};
pub use value::ProtectedString;
