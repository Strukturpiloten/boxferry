//! Maps explicit `ComposeLens` native models into `BoxFerry`'s neutral application model.
//!
//! The adapter does not read files, process environment variables, or guess active Compose
//! profiles. Callers parse and process Compose input with `ComposeLens`, provide identities for
//! each contributing source, and attach a [`compose_lens::profiles::ProfileSelection`] whenever a
//! merged project contains profiled services.

mod export;
mod import;
mod source;

pub use export::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeRuntime,
    DOCKER_COMPOSE_TARGET, PODMAN_COMPOSE_TARGET,
};
pub use import::ComposeImporter;
pub use source::ComposeSource;

/// The native Compose library consumed by this adapter.
pub use compose_lens;
