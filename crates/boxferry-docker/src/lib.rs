//! Typed `DockerLens` source mapping and inert, reviewed Docker target artifacts.
//!
//! This adapter never opens sockets, parses Engine JSON, executes requests, or writes output.
//! The importer consumes already-decoded observations with conservative, separately selected
//! promotion and resource decisions. The exporter requires an exact reviewed native target
//! profile and ordinary loss-policy authorization. The facade owns explicit acquisition and CLI
//! orchestration; this adapter introduces neither an executor nor a native parser.

mod export;
mod import;
mod public_labels;
mod resource_decisions;
mod source;

pub use export::{DOCKER_TARGET, DockerDependencyDecision, DockerEnvironmentPolicy, DockerExporter, DockerOutput};
pub use import::DockerImporter;
pub use public_labels::{
    DockerPublicLabelDecision, DockerPublicLabelDecisionError, DockerPublicLabelDecisions,
    DockerPublicLabelResourceKind,
};
pub use resource_decisions::{
    DockerResourceAction, DockerResourceDecision, DockerResourceDecisionError, DockerResourceDecisions,
    DockerResourceKind,
};
pub use source::{DockerPromotionPolicy, DockerSelection, DockerSource, DockerSourceError};

/// The native Docker library consumed by this adapter.
pub use docker_lens;
