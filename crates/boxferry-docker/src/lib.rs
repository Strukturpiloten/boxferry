//! Loss-aware decoding and explicit acquisition of Docker Engine inspection responses.
//!
//! Decoding is pure: callers supply exact-version JSON document arrays. Optional acquisition uses
//! a replaceable executor, an explicit daemon endpoint, and a forced Engine API version.

mod acquire;
mod decode;
mod native;
mod source;
mod version;

pub use acquire::{
    DockerAcquisitionError, DockerCommandExecutor, DockerCommandOutput, DockerExpansionPolicy, DockerInspectCommand,
    DockerInspector, DockerResourceKind, DockerResourceSelection, DockerSelectionError, ProcessDockerCommandExecutor,
};
pub use decode::{DockerImporter, DockerSnapshotResult};
pub use source::{DockerInspectDocuments, DockerInspectSource};
pub use version::{DockerApiVersion, ParseDockerApiVersionError};

/// Inclusive oldest Docker Engine API version covered by this adapter.
pub const MINIMUM_DOCKER_API_VERSION: DockerApiVersion = DockerApiVersion::new(1, 40);

/// Inclusive newest Docker Engine API version covered by fixtures and source review.
pub const MAXIMUM_DOCKER_API_VERSION: DockerApiVersion = DockerApiVersion::new(1, 55);
