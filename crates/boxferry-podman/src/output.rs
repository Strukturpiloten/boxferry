//! Deterministic, reviewable Podman output that `BoxFerry` never executes.

use std::fmt;

use boxferry_engine::PlatformVersion;

/// Serialized `PodmanLens` deployment artifact and runnable shell rendering.
#[derive(Clone, Eq, PartialEq)]
pub struct PodmanOutput {
    target_version: PlatformVersion,
    deployment_json: String,
    commands_shell: String,
}

impl PodmanOutput {
    pub(crate) const fn new(target_version: PlatformVersion, deployment_json: String, commands_shell: String) -> Self {
        Self {
            target_version,
            deployment_json,
            commands_shell,
        }
    }

    /// Returns the exact reviewed Podman version selected for rendering.
    #[must_use]
    pub const fn target_version(&self) -> PlatformVersion {
        self.target_version
    }

    /// Returns deterministic deployment-v1 JSON.
    ///
    /// The artifact is data, but may contain caller-authorized public application values.
    #[must_use]
    pub fn deployment_json(&self) -> &str {
        &self.deployment_json
    }

    /// Returns the deterministic Podman command script.
    ///
    /// This crate never executes the script. Running it performs the rendered Podman operations,
    /// so callers must review it and select the intended Podman connection first.
    #[must_use]
    pub fn commands_shell(&self) -> &str {
        &self.commands_shell
    }
}

impl fmt::Debug for PodmanOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodmanOutput")
            .field("target_version", &self.target_version)
            .field("deployment_json", &"[REDACTED]")
            .field("deployment_json_bytes", &self.deployment_json.len())
            .field("commands_shell", &"[REDACTED]")
            .field("commands_shell_bytes", &self.commands_shell.len())
            .finish()
    }
}
