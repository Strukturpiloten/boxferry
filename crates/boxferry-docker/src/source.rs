//! Explicit, sensitive-by-default Docker inspect inputs.

use std::fmt;

use boxferry_model::Identifier;

use crate::DockerApiVersion;

/// Complete set of Docker inspect JSON arrays used for one runtime snapshot.
#[derive(Clone, Eq, PartialEq)]
pub struct DockerInspectDocuments {
    containers: String,
    images: String,
    networks: String,
    volumes: String,
}

impl DockerInspectDocuments {
    /// Creates an explicit document set without parsing or accessing Docker.
    #[must_use]
    pub fn new(
        containers: impl Into<String>,
        images: impl Into<String>,
        networks: impl Into<String>,
        volumes: impl Into<String>,
    ) -> Self {
        Self {
            containers: containers.into(),
            images: images.into(),
            networks: networks.into(),
            volumes: volumes.into(),
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
}

impl fmt::Debug for DockerInspectDocuments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerInspectDocuments")
            .field("containers", &"[REDACTED]")
            .field("images", &"[REDACTED]")
            .field("networks", &"[REDACTED]")
            .field("volumes", &"[REDACTED]")
            .finish()
    }
}

/// Caller-selected application identity, Engine API version, and inspect documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerInspectSource {
    application_name: Identifier,
    api_version: DockerApiVersion,
    documents: DockerInspectDocuments,
}

impl DockerInspectSource {
    /// Creates a deterministic Docker inspect source.
    #[must_use]
    pub const fn new(
        application_name: Identifier,
        api_version: DockerApiVersion,
        documents: DockerInspectDocuments,
    ) -> Self {
        Self {
            application_name,
            api_version,
            documents,
        }
    }

    /// Returns the neutral application name selected by the caller.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns the exact Engine API version that produced these documents.
    #[must_use]
    pub const fn api_version(&self) -> DockerApiVersion {
        self.api_version
    }

    /// Returns the explicit inspect document set.
    #[must_use]
    pub const fn documents(&self) -> &DockerInspectDocuments {
        &self.documents
    }
}
