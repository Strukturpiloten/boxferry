//! Explicit caller resolutions for lifecycle facts that runtime inspection cannot prove.

use std::{collections::BTreeMap, error::Error, fmt};

use boxferry_model::{Identifier, ProvenanceKind, ResourceOwnership, Sourced};

/// Kind of runtime-observed resource whose lifecycle is being resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuntimeResourceKind {
    /// A named network.
    Network,
    /// A named volume.
    Volume,
    /// A structural service group reconstructed from a runtime pod.
    ServiceGroup,
}

impl fmt::Display for RuntimeResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Network => "network",
            Self::Volume => "volume",
            Self::ServiceGroup => "service group",
        })
    }
}

/// Invalid or ambiguous caller-supplied runtime lifecycle resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuntimeResolutionError {
    /// Only application-owned and external resources can resolve runtime lifecycle uncertainty.
    UnsupportedOwnership {
        /// Resource kind being resolved.
        kind: RuntimeResourceKind,
        /// Neutral resource name.
        name: String,
    },
    /// A resolution did not carry explicit caller/user-override provenance.
    MissingUserOverride {
        /// Resource kind being resolved.
        kind: RuntimeResourceKind,
        /// Neutral resource name.
        name: String,
    },
    /// The same resource already has a resolution.
    Duplicate {
        /// Resource kind being resolved.
        kind: RuntimeResourceKind,
        /// Neutral resource name.
        name: String,
    },
}

impl fmt::Display for RuntimeResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOwnership { kind, name } => write!(
                formatter,
                "runtime {kind} `{name}` must be resolved as application-owned or external"
            ),
            Self::MissingUserOverride { kind, name } => write!(
                formatter,
                "runtime {kind} `{name}` resolution requires user-override provenance"
            ),
            Self::Duplicate { kind, name } => {
                write!(formatter, "runtime {kind} `{name}` already has a lifecycle resolution")
            }
        }
    }
}

impl Error for RuntimeResolutionError {}

/// Finite caller-owned lifecycle decisions applied during runtime reconstruction.
///
/// Every entry is keyed by its exact neutral resource name. A decision must select either
/// [`ResourceOwnership::Application`] or [`ResourceOwnership::External`] and must carry at least
/// one [`ProvenanceKind::UserOverride`] origin. Resolutions are never inferred, replaced, or
/// applied as blanket defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeResolutions {
    networks: BTreeMap<Identifier, Sourced<ResourceOwnership>>,
    volumes: BTreeMap<Identifier, Sourced<ResourceOwnership>>,
    service_groups: BTreeMap<Identifier, Sourced<ResourceOwnership>>,
}

impl RuntimeResolutions {
    /// Creates an empty resolution set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            networks: BTreeMap::new(),
            volumes: BTreeMap::new(),
            service_groups: BTreeMap::new(),
        }
    }

    /// Resolves lifecycle ownership for one exact network name.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeResolutionError`] for an unsupported ownership value, missing explicit
    /// user-override provenance, or a duplicate network resolution.
    pub fn set_network_ownership(
        &mut self,
        name: Identifier,
        ownership: Sourced<ResourceOwnership>,
    ) -> Result<(), RuntimeResolutionError> {
        insert_resolution(&mut self.networks, RuntimeResourceKind::Network, name, ownership)
    }

    /// Resolves lifecycle ownership for one exact volume name.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeResolutionError`] for an unsupported ownership value, missing explicit
    /// user-override provenance, or a duplicate volume resolution.
    pub fn set_volume_ownership(
        &mut self,
        name: Identifier,
        ownership: Sourced<ResourceOwnership>,
    ) -> Result<(), RuntimeResolutionError> {
        insert_resolution(&mut self.volumes, RuntimeResourceKind::Volume, name, ownership)
    }

    /// Resolves lifecycle ownership for one exact structural service-group name.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeResolutionError`] for an unsupported ownership value, missing explicit
    /// user-override provenance, or a duplicate service-group resolution.
    pub fn set_service_group_ownership(
        &mut self,
        name: Identifier,
        ownership: Sourced<ResourceOwnership>,
    ) -> Result<(), RuntimeResolutionError> {
        insert_resolution(
            &mut self.service_groups,
            RuntimeResourceKind::ServiceGroup,
            name,
            ownership,
        )
    }

    /// Returns the exact network resolution, when present.
    #[must_use]
    pub fn network_ownership(&self, name: &Identifier) -> Option<&Sourced<ResourceOwnership>> {
        self.networks.get(name)
    }

    /// Returns the exact volume resolution, when present.
    #[must_use]
    pub fn volume_ownership(&self, name: &Identifier) -> Option<&Sourced<ResourceOwnership>> {
        self.volumes.get(name)
    }

    /// Returns the exact service-group resolution, when present.
    #[must_use]
    pub fn service_group_ownership(&self, name: &Identifier) -> Option<&Sourced<ResourceOwnership>> {
        self.service_groups.get(name)
    }
}

fn insert_resolution(
    resolutions: &mut BTreeMap<Identifier, Sourced<ResourceOwnership>>,
    kind: RuntimeResourceKind,
    name: Identifier,
    ownership: Sourced<ResourceOwnership>,
) -> Result<(), RuntimeResolutionError> {
    let error_name = name.as_str().to_owned();
    if !matches!(
        ownership.value(),
        ResourceOwnership::Application | ResourceOwnership::External
    ) {
        return Err(RuntimeResolutionError::UnsupportedOwnership { kind, name: error_name });
    }
    if !ownership
        .origins()
        .iter()
        .any(|origin| origin.kind() == ProvenanceKind::UserOverride)
    {
        return Err(RuntimeResolutionError::MissingUserOverride { kind, name: error_name });
    }
    if resolutions.contains_key(&name) {
        return Err(RuntimeResolutionError::Duplicate { kind, name: error_name });
    }
    resolutions.insert(name, ownership);
    Ok(())
}

#[cfg(test)]
mod tests {
    use boxferry_model::{Identifier, Provenance, ResourceOwnership, SourceId, Sourced};

    use super::{RuntimeResolutionError, RuntimeResolutions, RuntimeResourceKind};

    #[test]
    fn requires_supported_ownership_and_explicit_user_provenance() -> Result<(), String> {
        let name = Identifier::new("data").map_err(|error| error.to_string())?;
        let source = SourceId::new("decision:runtime-lifecycle").map_err(|error| error.to_string())?;
        let mut resolutions = RuntimeResolutions::new();

        assert_eq!(
            resolutions.set_volume_ownership(
                name.clone(),
                Sourced::from_source(ResourceOwnership::Uncertain, Provenance::user_override(source.clone())),
            ),
            Err(RuntimeResolutionError::UnsupportedOwnership {
                kind: RuntimeResourceKind::Volume,
                name: "data".to_owned(),
            })
        );
        assert_eq!(
            resolutions.set_volume_ownership(
                name.clone(),
                Sourced::from_source(ResourceOwnership::Application, Provenance::runtime_observation(source)),
            ),
            Err(RuntimeResolutionError::MissingUserOverride {
                kind: RuntimeResourceKind::Volume,
                name: "data".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn retains_one_resolution_per_exact_resource_name() -> Result<(), String> {
        let name = Identifier::new("frontend").map_err(|error| error.to_string())?;
        let source = SourceId::new("decision:runtime-lifecycle").map_err(|error| error.to_string())?;
        let ownership = || Sourced::from_source(ResourceOwnership::External, Provenance::user_override(source.clone()));
        let mut resolutions = RuntimeResolutions::new();

        resolutions
            .set_network_ownership(name.clone(), ownership())
            .map_err(|error| error.to_string())?;
        assert_eq!(
            resolutions.set_network_ownership(name.clone(), ownership()),
            Err(RuntimeResolutionError::Duplicate {
                kind: RuntimeResourceKind::Network,
                name: "frontend".to_owned(),
            })
        );
        assert_eq!(
            resolutions.network_ownership(&name).map(Sourced::value),
            Some(&ResourceOwnership::External)
        );
        Ok(())
    }
}
