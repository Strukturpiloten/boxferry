//! Explicit, already-decoded Docker source boundary.

use std::{error::Error, fmt};

use boxferry_model::Identifier;
use docker_lens::{
    acquisition::AcquisitionError,
    decoder::{DecodeError, DecodedInventory},
};

use crate::{DockerPublicLabelDecisions, DockerResourceDecisions};

/// Failure while acquiring or decoding a caller-selected Docker source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerSourceError {
    /// `DockerLens` could not complete bounded read-only acquisition.
    Acquisition(AcquisitionError),
    /// `DockerLens` rejected the captured native response shape.
    Decode(DecodeError),
}

impl From<AcquisitionError> for DockerSourceError {
    fn from(error: AcquisitionError) -> Self {
        Self::Acquisition(error)
    }
}

impl From<DecodeError> for DockerSourceError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

impl fmt::Display for DockerSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acquisition(error) => write!(formatter, "Docker acquisition failed: {error:?}"),
            Self::Decode(error) => write!(formatter, "Docker decoding failed: {error:?}"),
        }
    }
}

impl Error for DockerSourceError {}

/// Explicit authorization to reconstruct reviewed current state from effective observations.
///
/// The default is conservative. Promotion does not establish original authorship, authorize
/// target conversion loss, or turn runtime-assigned values into configured observations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DockerPromotionPolicy {
    enabled: u8,
}

const PORTABLE_EFFECTIVE_SETTINGS: u8 = 1 << 0;
const NAMED_VOLUME_MOUNTS: u8 = 1 << 1;
const SAME_HOST_BIND_MOUNTS: u8 = 1 << 2;
const PROTECTED_ENVIRONMENT_VALUES: u8 = 1 << 3;
const OBSERVED_NETWORK_MEMBERSHIP: u8 = 1 << 4;
const INSPECTED_BRIDGE_NETWORK_SETTINGS: u8 = 1 << 5;
const OBSERVED_NETWORK_ALIASES: u8 = 1 << 6;

impl DockerPromotionPolicy {
    /// Retains effective observations as evidence without reconstructing them as desired state.
    #[must_use]
    pub const fn conservative() -> Self {
        Self { enabled: 0 }
    }

    /// Authorizes the reviewed portable snapshot subset for a future caller-selected CLI preset.
    ///
    /// Host bind paths, protected environment values, and public metadata labels need separate
    /// decisions. Reviewed,
    /// ID-correlated runtime membership, aliases, and finite inspected bridge settings are
    /// included only as approximate topology with uncertain lifecycle ownership. A target
    /// exporter makes an independent fidelity and protected-value decision.
    #[must_use]
    pub const fn portable_application() -> Self {
        Self {
            enabled: PORTABLE_EFFECTIVE_SETTINGS
                | NAMED_VOLUME_MOUNTS
                | OBSERVED_NETWORK_MEMBERSHIP
                | INSPECTED_BRIDGE_NETWORK_SETTINGS
                | OBSERVED_NETWORK_ALIASES,
        }
    }

    /// Authorizes finite image, process, health, restart, and configured-port reconstruction.
    #[must_use]
    pub const fn with_portable_effective_settings(mut self, enabled: bool) -> Self {
        self.set(PORTABLE_EFFECTIVE_SETTINGS, enabled);
        self
    }

    /// Authorizes uniquely correlated inspected named-volume attachments.
    ///
    /// Successfully promoted mounts retain their logical volume source and access,
    /// without neutral relabel intent. Native volume-label modes remain protected
    /// evidence and an Unsupported `.mode` outcome. Exact and approximate loss
    /// policies refuse this limitation; only reviewed partial loss may emit an
    /// artifact. This never infers a host bind or permits silent mount omission.
    #[must_use]
    pub const fn with_named_volume_mounts(mut self, enabled: bool) -> Self {
        self.set(NAMED_VOLUME_MOUNTS, enabled);
        self
    }

    /// Authorizes reconstruction of effective bind mounts on the same host path.
    #[must_use]
    pub const fn with_same_host_bind_mounts(mut self, enabled: bool) -> Self {
        self.set(SAME_HOST_BIND_MOUNTS, enabled);
        self
    }

    /// Authorizes protected literal environment values, separately from portable settings.
    ///
    /// This affects source import only; downstream artifact creation requires its own
    /// sensitive-value authorization and must never assume this policy grants it.
    #[must_use]
    pub const fn with_protected_environment_values(mut self, enabled: bool) -> Self {
        self.set(PROTECTED_ENVIRONMENT_VALUES, enabled);
        self
    }

    /// Authorizes approximate reconstruction of ID-correlated observed network membership.
    ///
    /// This does not establish authored membership, network lifecycle ownership, aliases, or
    /// static addresses. Alias and bridge-setting promotion each require a separate switch.
    /// It does not authorize target conversion or runtime mutation.
    #[must_use]
    pub const fn with_observed_network_membership(mut self, enabled: bool) -> Self {
        self.set(OBSERVED_NETWORK_MEMBERSHIP, enabled);
        self
    }

    /// Authorizes finite effective settings of uniquely identified inspected bridge networks.
    ///
    /// This does not establish who owns or should create a network, nor authorize IPAM, driver
    /// options, or target output.
    #[must_use]
    pub const fn with_inspected_bridge_network_settings(mut self, enabled: bool) -> Self {
        self.set(INSPECTED_BRIDGE_NETWORK_SETTINGS, enabled);
        self
    }

    /// Authorizes approximate aliases only on uniquely ID-correlated observed attachments.
    ///
    /// Membership promotion is separately required. This does not prove alias authorship or
    /// authorize rendering protected aliases into an output artifact.
    #[must_use]
    pub const fn with_observed_network_aliases(mut self, enabled: bool) -> Self {
        self.set(OBSERVED_NETWORK_ALIASES, enabled);
        self
    }

    /// Whether reviewed effective process and service settings may be reconstructed.
    #[must_use]
    pub const fn promotes_portable_effective_settings(self) -> bool {
        self.contains(PORTABLE_EFFECTIVE_SETTINGS)
    }

    /// Whether uniquely correlated named-volume mounts may be reconstructed.
    #[must_use]
    pub const fn promotes_named_volume_mounts(self) -> bool {
        self.contains(NAMED_VOLUME_MOUNTS)
    }

    /// Whether effective bind paths may be reconstructed as same-host intent.
    #[must_use]
    pub const fn promotes_same_host_bind_mounts(self) -> bool {
        self.contains(SAME_HOST_BIND_MOUNTS)
    }

    /// Whether protected literal environment values may enter the neutral source model.
    #[must_use]
    pub const fn promotes_protected_environment_values(self) -> bool {
        self.contains(PROTECTED_ENVIRONMENT_VALUES)
    }

    /// Whether uniquely ID-correlated runtime membership may become approximate topology.
    #[must_use]
    pub const fn promotes_observed_network_membership(self) -> bool {
        self.contains(OBSERVED_NETWORK_MEMBERSHIP)
    }

    /// Whether finite inspected bridge settings may become approximate neutral current state.
    #[must_use]
    pub const fn promotes_inspected_bridge_network_settings(self) -> bool {
        self.contains(INSPECTED_BRIDGE_NETWORK_SETTINGS)
    }

    /// Whether aliases on verified observed attachments may become approximate neutral aliases.
    #[must_use]
    pub const fn promotes_observed_network_aliases(self) -> bool {
        self.contains(OBSERVED_NETWORK_ALIASES)
    }

    const fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.enabled |= flag;
        } else {
            self.enabled &= !flag;
        }
    }

    const fn contains(self, flag: u8) -> bool {
        self.enabled & flag != 0
    }
}

/// How the caller selected this source; discovery metadata is not inspected intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerSelection {
    /// The caller requested list metadata only.
    Discovery,
    /// The caller requested inspected resources, which may still yield no match.
    Explicit,
    /// A caller supplied an already-decoded inventory without selector context.
    Unspecified,
}

/// One caller-named application with `DockerLens` typed observations and source context.
pub struct DockerSource {
    application_name: Identifier,
    inventory: DecodedInventory,
    selection: DockerSelection,
    promotion_policy: DockerPromotionPolicy,
    resource_decisions: DockerResourceDecisions,
    public_label_decisions: DockerPublicLabelDecisions,
}

impl DockerSource {
    /// Wraps an already-decoded native inventory without contacting a runtime.
    #[must_use]
    pub const fn new(application_name: Identifier, inventory: DecodedInventory) -> Self {
        Self {
            application_name,
            inventory,
            selection: DockerSelection::Unspecified,
            promotion_policy: DockerPromotionPolicy::conservative(),
            resource_decisions: DockerResourceDecisions::new(),
            public_label_decisions: DockerPublicLabelDecisions::empty(),
        }
    }

    /// Retains caller selection context when acquisition consumes its selector.
    #[must_use]
    pub const fn with_selection(mut self, selection: DockerSelection) -> Self {
        self.selection = selection;
        self
    }

    /// Replaces the conservative source-state promotion policy.
    #[must_use]
    pub const fn with_promotion_policy(mut self, policy: DockerPromotionPolicy) -> Self {
        self.promotion_policy = policy;
        self
    }

    /// Supplies separate, explicit source-resource and destination-lifecycle decisions.
    #[must_use]
    pub fn with_resource_decisions(mut self, decisions: DockerResourceDecisions) -> Self {
        self.resource_decisions = decisions;
        self
    }

    /// Declares exact observed labels ordinary and non-sensitive for generated artifacts.
    ///
    /// Empty by default even under portable promotion. This grants no resource lifecycle,
    /// protected environment, target-loss, or runtime mutation authorization.
    #[must_use]
    pub fn with_public_label_decisions(mut self, decisions: DockerPublicLabelDecisions) -> Self {
        self.public_label_decisions = decisions;
        self
    }

    /// Returns the caller-selected application name; native labels never choose it.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns typed observations, including separate observed daemon context.
    #[must_use]
    pub const fn inventory(&self) -> &DecodedInventory {
        &self.inventory
    }

    /// Returns whether this source was a metadata-only discovery or resource selection.
    #[must_use]
    pub const fn selection(&self) -> DockerSelection {
        self.selection
    }

    /// Returns the caller-selected source-state promotion policy.
    #[must_use]
    pub const fn promotion_policy(&self) -> DockerPromotionPolicy {
        self.promotion_policy
    }

    /// Returns explicit resource decisions; absence keeps ownership uncertain.
    #[must_use]
    pub const fn resource_decisions(&self) -> &DockerResourceDecisions {
        &self.resource_decisions
    }

    /// Returns exact public-label declarations, independent of the source promotion preset.
    #[must_use]
    pub const fn public_label_decisions(&self) -> &DockerPublicLabelDecisions {
        &self.public_label_decisions
    }
}
