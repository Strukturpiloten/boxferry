//! Explicit, sensitive-by-default Podman inspect inputs.

use std::fmt;

use boxferry_engine::PlatformVersion;
use boxferry_model::Identifier;

/// Complete set of JSON arrays used for one Podman runtime snapshot.
///
/// Each document must contain the unmodified JSON array emitted for one corresponding inspect
/// resource family. Empty families are represented by `[]`, not by an omitted ambient lookup.
#[derive(Clone, Eq, PartialEq)]
pub struct PodmanInspectDocuments {
    containers: String,
    images: String,
    networks: String,
    volumes: String,
    pods: String,
}

impl PodmanInspectDocuments {
    /// Creates an explicit document set without parsing or accessing Podman.
    #[must_use]
    pub fn new(
        containers: impl Into<String>,
        images: impl Into<String>,
        networks: impl Into<String>,
        volumes: impl Into<String>,
        pods: impl Into<String>,
    ) -> Self {
        Self {
            containers: containers.into(),
            images: images.into(),
            networks: networks.into(),
            volumes: volumes.into(),
            pods: pods.into(),
        }
    }

    /// Returns the container-inspect JSON array.
    #[must_use]
    pub fn containers(&self) -> &str {
        &self.containers
    }

    /// Returns the image-inspect JSON array.
    #[must_use]
    pub fn images(&self) -> &str {
        &self.images
    }

    /// Returns the network-inspect JSON array.
    #[must_use]
    pub fn networks(&self) -> &str {
        &self.networks
    }

    /// Returns the volume-inspect JSON array.
    #[must_use]
    pub fn volumes(&self) -> &str {
        &self.volumes
    }

    /// Returns the pod-inspect JSON array.
    #[must_use]
    pub fn pods(&self) -> &str {
        &self.pods
    }
}

impl fmt::Debug for PodmanInspectDocuments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanInspectDocuments")
            .field("containers", &"[REDACTED]")
            .field("images", &"[REDACTED]")
            .field("networks", &"[REDACTED]")
            .field("volumes", &"[REDACTED]")
            .field("pods", &"[REDACTED]")
            .finish()
    }
}

/// Caller-selected application identity, producing Podman version, and inspect documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodmanInspectSource {
    application_name: Identifier,
    version: PlatformVersion,
    documents: PodmanInspectDocuments,
}

impl PodmanInspectSource {
    /// Creates a deterministic Podman inspect source.
    #[must_use]
    pub const fn new(
        application_name: Identifier,
        version: PlatformVersion,
        documents: PodmanInspectDocuments,
    ) -> Self {
        Self {
            application_name,
            version,
            documents,
        }
    }

    /// Returns the neutral application name selected by the caller.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns the exact Podman version that produced these documents.
    #[must_use]
    pub const fn version(&self) -> PlatformVersion {
        self.version
    }

    /// Returns the explicit inspect document set.
    #[must_use]
    pub const fn documents(&self) -> &PodmanInspectDocuments {
        &self.documents
    }
}
