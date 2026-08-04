//! Format-independent application model for `BoxFerry`.
//!
//! The model describes container application intent without exposing Compose,
//! Quadlet, Podman, Docker, or Kubernetes types. Ordered collections and source
//! provenance remain available to conversion adapters.

mod application;
mod image;
mod provenance;
mod value;

pub use application::{
    Application, Command, Config, ConfigMaterial, EnvironmentValue, EnvironmentVariable, Healthcheck,
    HealthcheckCommand, HealthcheckDuration, HealthcheckRetries, HostAddress, HostAddressKind, HostMapping, Identifier,
    ModelError, Mount, MountSource, Network, NetworkAttachment, Port, Protocol, ResourceGrant, ResourceGrantSyntax,
    ResourceOwnership, Secret, SecretMaterial, SelinuxRelabel, Service, ServiceDependency, ServiceDependencyCondition,
    Volume,
};
pub use image::ImageReference;
pub use provenance::{Provenance, ProvenanceKind, SourceId, SourceSpan, Sourced};
pub use value::ProtectedString;
