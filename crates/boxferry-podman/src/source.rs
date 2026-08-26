//! Explicit already-acquired Podman source boundary.

use boxferry_model::Identifier;
use podman_lens::{CapabilityCatalogueEntry, ResourceGraph, ResourceInventory};

/// Explicit authorization for promoting effective runtime observations into desired intent.
///
/// The conservative default promotes nothing. Runtime-assigned and local-resolution values are
/// never promotable through this policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PodmanPromotionPolicy {
    effective_named_volume_mounts: bool,
    effective_named_networks: bool,
    portable_effective_settings: bool,
}

impl PodmanPromotionPolicy {
    /// Creates the conservative policy that retains effective values as observations only.
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            effective_named_volume_mounts: false,
            effective_named_networks: false,
            portable_effective_settings: false,
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

    /// Authorizes or rejects the reviewed portable effective-settings subset.
    #[must_use]
    pub const fn with_portable_effective_settings(mut self, enabled: bool) -> Self {
        self.portable_effective_settings = enabled;
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

    /// Returns whether reviewed portable effective settings may be promoted.
    #[must_use]
    pub const fn promotes_portable_effective_settings(self) -> bool {
        self.portable_effective_settings
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

    /// Returns the exact Podman Engine version observed before acquisition started.
    ///
    /// This is source evidence only. In particular, an input-only legacy runtime is never
    /// treated as a Podman deployment target; exporters still require a separate explicit
    /// [`crate::ResolvedPodmanTarget`].
    #[must_use]
    pub fn observed_engine_version(&self) -> &str {
        self.inventory.service().engine_version().original()
    }

    /// Returns the exact Libpod API version used for acquisition.
    ///
    /// This is source evidence only and must not be inferred as an output target.
    #[must_use]
    pub fn observed_api_version(&self) -> &str {
        self.inventory.service().api_version().original()
    }

    /// Returns the immutable input capability evidence for the observed service.
    ///
    /// The returned entry may be input-only. Callers must select a separate explicit target
    /// profile before planning Podman output.
    #[must_use]
    pub fn input_capability(&self) -> &CapabilityCatalogueEntry {
        self.inventory.service().input_capability()
    }

    /// Returns an always-redacted, serialization-only inventory snapshot.
    ///
    /// The snapshot omits environment values, secrets, connection details, label values and raw
    /// unknown JSON. It is diagnostic evidence, not a supported Podman input format.
    #[must_use]
    pub fn redacted_inventory_snapshot(&self) -> podman_lens::snapshot::v1::InventorySnapshot {
        podman_lens::snapshot::v1::inventory(&self.inventory)
    }

    /// Returns an always-redacted, serialization-only discovery-graph snapshot.
    ///
    /// The snapshot is intended for an opt-in support bundle and cannot be used to recreate a
    /// live inventory or select an output target.
    #[must_use]
    pub fn redacted_graph_snapshot(&self) -> podman_lens::snapshot::v1::GraphSnapshot {
        podman_lens::snapshot::v1::graph(&self.graph)
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
