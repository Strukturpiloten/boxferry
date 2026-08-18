//! Loss-aware decoding of caller-supplied Podman inspection responses.
//!
//! This crate never starts Podman or reads ambient machine state. Callers explicitly supply
//! sanitized or live JSON documents and the Podman version that produced them. Decoding targets
//! [`boxferry_runtime::RuntimeSnapshot`], keeping command execution replaceable and separately
//! testable.

mod acquire;
mod decode;
mod native;
mod source;

pub use acquire::{
    PodmanAcquisitionError, PodmanCommandExecutor, PodmanCommandOutput, PodmanExpansionPolicy, PodmanInspectCommand,
    PodmanInspector, PodmanResourceKind, PodmanResourceSelection, PodmanSelectionError, ProcessPodmanCommandExecutor,
};
pub use decode::{PodmanImporter, PodmanSnapshotResult};
pub use source::{PodmanInspectDocuments, PodmanInspectSource};

/// Inclusive oldest Podman version whose inspect shape is covered by this adapter.
pub const MINIMUM_PODMAN_VERSION: boxferry_engine::PlatformVersion = boxferry_engine::PlatformVersion::new(5, 4, 0);

/// Inclusive newest Podman version whose inspect shape is covered by fixtures and source review.
pub const MAXIMUM_PODMAN_VERSION: boxferry_engine::PlatformVersion = boxferry_engine::PlatformVersion::new(6, 1, 0);
