//! Semantic `PodmanLens` adapter for `BoxFerry`'s neutral application model.
//!
//! Read-only acquisition remains a caller boundary. This crate consumes an already acquired
//! [`podman_lens::ResourceInventory`] and [`podman_lens::ResourceGraph`], and produces only inert
//! deployment JSON and a review shell script. It never opens a connection or executes Podman.

mod export;
mod import;
mod output;
mod source;

pub use export::{
    PODMAN_TARGET, PodmanExporter, PodmanTargetError, ResolvedPodmanTarget, resolve_podman_target,
    reviewed_podman_versions,
};
pub use import::PodmanImporter;
pub use output::PodmanOutput;
pub use source::{PodmanPromotionPolicy, PodmanSource};

/// The native Podman library consumed by this adapter.
pub use podman_lens;
