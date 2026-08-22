//! Deterministic inert Podman output.

use std::fmt;

use boxferry_engine::PlatformVersion;

/// Serialized `PodmanLens` deployment artifact and review-only shell rendering.
#[derive(Clone, Eq, PartialEq)]
pub struct PodmanOutput {
    target_version: PlatformVersion,
    deployment_json: String,
    review_shell: String,
}

impl PodmanOutput {
    pub(crate) const fn new(target_version: PlatformVersion, deployment_json: String, review_shell: String) -> Self {
        Self {
            target_version,
            deployment_json,
            review_shell,
        }
    }

    /// Returns the exact reviewed Podman version selected for rendering.
    #[must_use]
    pub const fn target_version(&self) -> PlatformVersion {
        self.target_version
    }

    /// Returns deterministic deployment-v1 JSON.
    ///
    /// The artifact is inert but may contain caller-authorized public application values.
    #[must_use]
    pub fn deployment_json(&self) -> &str {
        &self.deployment_json
    }

    /// Returns a review-only shell script. This crate never executes it.
    #[must_use]
    pub fn review_shell(&self) -> &str {
        &self.review_shell
    }
}

impl fmt::Debug for PodmanOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanOutput")
            .field("target_version", &self.target_version)
            .field("deployment_json", &"[REDACTED]")
            .field("deployment_json_bytes", &self.deployment_json.len())
            .field("review_shell", &"[REDACTED]")
            .field("review_shell_bytes", &self.review_shell.len())
            .finish()
    }
}
