//! Runtime-neutral observations and loss-aware application reconstruction.
//!
//! Docker and Podman response parsing belongs in separate runtime-specific adapters. Those
//! adapters construct a [`RuntimeSnapshot`] from effective inspection data and pass it to the
//! pure [`RuntimeImporter`]. No daemon, command, file, or process environment is accessed here.

mod observation;
mod reconstruct;
mod resolution;

pub use observation::{
    ContainerObservation, CreationEvidence, EffectiveCommand, ImageObservation, NetworkObservation, PodObservation,
    RuntimeEnvironmentVariable, RuntimeImplementation, RuntimeSnapshot, RuntimeSnapshotError, VolumeObservation,
};
pub use reconstruct::{OverrideReconstruction, RuntimeImporter};
pub use resolution::{RuntimeResolutionError, RuntimeResolutions, RuntimeResourceKind};
