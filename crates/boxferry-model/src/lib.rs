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
    Application, Command, EnvironmentValue, EnvironmentVariable, Identifier, ModelError, Mount, MountSource, Network,
    NetworkAttachment, Port, Protocol, ResourceOwnership, SelinuxRelabel, Service, Volume,
};
pub use image::ImageReference;
pub use provenance::{Provenance, SourceId, SourceSpan, Sourced};
pub use value::ProtectedString;
