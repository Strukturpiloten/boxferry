//! Explicit already-acquired Podman source boundary.

use boxferry_model::Identifier;
use podman_lens::{ResourceGraph, ResourceInventory};

/// Explicit authorization for promoting effective runtime observations into desired intent.
///
/// The conservative default promotes nothing. Runtime-assigned and local-resolution values are
/// never promotable through this policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PodmanPromotionPolicy {
    effective_named_volume_mounts: bool,
    effective_named_networks: bool,
}

impl PodmanPromotionPolicy {
    /// Creates the conservative policy that retains effective values as observations only.
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            effective_named_volume_mounts: false,
            effective_named_networks: false,
        }
    }

    /// Authorizes or rejects portable named-volume mount promotion.
    #[must_use]
    pub const fn with_effective_named_volume_mounts(mut self, enabled: bool) -> Self {
        self.effective_named_volume_mounts = enabled;
        self
    }

    /// Authorizes or rejects named-network attachment promotion.
    #[must_use]
    pub const fn with_effective_named_networks(mut self, enabled: bool) -> Self {
        self.effective_named_networks = enabled;
        self
    }

    /// Returns whether portable effective named-volume mounts may be promoted.
    #[must_use]
    pub const fn promotes_effective_named_volume_mounts(self) -> bool {
        self.effective_named_volume_mounts
    }

    /// Returns whether effective named-network attachments may be promoted.
    #[must_use]
    pub const fn promotes_effective_named_networks(self) -> bool {
        self.effective_named_networks
    }
}
/// One explicit Podman import source after caller-owned acquisition and discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PodmanSource {
    application_name: Identifier,
    inventory: ResourceInventory,
    graph: ResourceGraph,
    promotion_policy: PodmanPromotionPolicy,
}

impl PodmanSource {
    /// Creates a source without reading files, environment variables, connections, or runtimes.
    #[must_use]
    pub const fn new(application_name: Identifier, inventory: ResourceInventory, graph: ResourceGraph) -> Self {
        Self {
            application_name,
            inventory,
            graph,
            promotion_policy: PodmanPromotionPolicy::conservative(),
        }
    }

    /// Returns the caller-selected neutral application name.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns the acquired Podman inventory.
    #[must_use]
    pub const fn inventory(&self) -> &ResourceInventory {
        &self.inventory
    }

    /// Returns the caller-selected discovery graph.
    #[must_use]
    pub const fn graph(&self) -> &ResourceGraph {
        &self.graph
    }

    /// Replaces the conservative effective-observation promotion policy.
    #[must_use]
    pub const fn with_promotion_policy(mut self, policy: PodmanPromotionPolicy) -> Self {
        self.promotion_policy = policy;
        self
    }

    /// Returns the explicit effective-observation promotion policy.
    #[must_use]
    pub const fn promotion_policy(&self) -> PodmanPromotionPolicy {
        self.promotion_policy
    }
}
