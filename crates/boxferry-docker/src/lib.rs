//! Format-neutral intent mapped to inert, reviewed `DockerLens` target artifacts.
//!
//! This target-only prerelease integration never acquires a source, opens a socket,
//! executes requests, or writes output. Callers select an exact native target profile
//! and authorize structured conversion outcomes with the ordinary loss policy.
//! Docker input and CLI routes are not implemented by this checkpoint.

mod export;

pub use export::{DOCKER_TARGET, DockerDependencyDecision, DockerEnvironmentPolicy, DockerExporter, DockerOutput};

/// The native Docker library consumed by this target adapter.
pub use docker_lens;
