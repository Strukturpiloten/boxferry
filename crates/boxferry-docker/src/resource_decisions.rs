//! Explicit source-resource to destination-resource decisions for Docker imports.

use std::{error::Error, fmt, str::FromStr};

/// The inspected resource kind; names and IDs are never interchangeable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockerResourceKind {
    /// A canonical full Docker network ID.
    Network,
    /// An exact inspected named-volume name.
    Volume,
}

/// Destination lifecycle intent, independent of source-state promotion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockerResourceAction {
    /// Include a new destination create request.
    Create,
    /// Require an existing destination resource.
    External,
}

/// A checked source key, action, and explicit destination identity.
#[derive(Clone, Eq, PartialEq)]
pub struct DockerResourceDecision {
    kind: DockerResourceKind,
    source: String,
    action: DockerResourceAction,
    target: String,
}

impl fmt::Debug for DockerResourceDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerResourceDecision")
            .field("kind", &self.kind)
            .field("source", &"[redacted]")
            .field("action", &self.action)
            .field("target", &"[redacted]")
            .finish()
    }
}

/// Value-free rejection of a malformed or conflicting resource decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerResourceDecisionError {
    /// The source kind, key, action, or destination spelling is invalid.
    Malformed,
    /// A source resource received more than one decision.
    Duplicate,
}

impl fmt::Display for DockerResourceDecisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("malformed Docker resource decision"),
            Self::Duplicate => formatter.write_str("duplicate Docker resource decision"),
        }
    }
}

impl Error for DockerResourceDecisionError {}

impl DockerResourceDecision {
    /// Checks the complete source key and explicit target identity without exposing either in errors.
    ///
    /// # Errors
    ///
    /// Returns a value-free error for invalid native identity spellings.
    pub fn new(
        kind: DockerResourceKind,
        source: impl Into<String>,
        action: DockerResourceAction,
        target: impl Into<String>,
    ) -> Result<Self, DockerResourceDecisionError> {
        let source = source.into();
        let target = target.into();
        let valid_source = match kind {
            DockerResourceKind::Network => {
                source.len() == 64
                    && source
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            }
            DockerResourceKind::Volume => valid_name(&source),
        };
        if !valid_source || !valid_name(&target) {
            return Err(DockerResourceDecisionError::Malformed);
        }
        Ok(Self {
            kind,
            source,
            action,
            target,
        })
    }

    /// Native resource kind; source and target names remain opaque to reports.
    #[must_use]
    pub const fn kind(&self) -> DockerResourceKind {
        self.kind
    }

    /// Requested target lifecycle action.
    #[must_use]
    pub const fn action(&self) -> DockerResourceAction {
        self.action
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }
}

impl FromStr for DockerResourceDecision {
    type Err = DockerResourceDecisionError;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let (source, destination) = spec.split_once('=').ok_or(DockerResourceDecisionError::Malformed)?;
        let (kind, source) = source.split_once(':').ok_or(DockerResourceDecisionError::Malformed)?;
        let (action, target) = destination
            .split_once(':')
            .ok_or(DockerResourceDecisionError::Malformed)?;
        let kind = match kind {
            "network" => DockerResourceKind::Network,
            "volume" => DockerResourceKind::Volume,
            _ => return Err(DockerResourceDecisionError::Malformed),
        };
        let action = match action {
            "create" => DockerResourceAction::Create,
            "external" => DockerResourceAction::External,
            _ => return Err(DockerResourceDecisionError::Malformed),
        };
        Self::new(kind, source, action, target)
    }
}

/// Validated, non-conflicting caller decisions for one selected Docker inventory.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct DockerResourceDecisions {
    entries: Vec<DockerResourceDecision>,
}

impl fmt::Debug for DockerResourceDecisions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerResourceDecisions")
            .field("count", &self.entries.len())
            .finish()
    }
}

impl DockerResourceDecisions {
    /// Empty decisions preserve uncertain source ownership.
    #[must_use]
    pub const fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Adds one checked decision, rejecting a repeated kind-qualified source key.
    ///
    /// # Errors
    ///
    /// Returns a value-free duplicate error.
    pub fn add(&mut self, decision: DockerResourceDecision) -> Result<(), DockerResourceDecisionError> {
        if self
            .entries
            .iter()
            .any(|entry| entry.kind == decision.kind && entry.source == decision.source)
        {
            return Err(DockerResourceDecisionError::Duplicate);
        }
        self.entries.push(decision);
        Ok(())
    }

    pub(crate) fn entries(&self) -> &[DockerResourceDecision] {
        &self.entries
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    name.len() <= 255
        && name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::{DockerResourceAction, DockerResourceDecision, DockerResourceDecisionError, DockerResourceDecisions};

    #[test]
    fn parses_kind_qualified_decisions_and_redacts_debug() -> Result<(), DockerResourceDecisionError> {
        let id = "a".repeat(64);
        let network = format!("network:{id}=create:new-private-network");
        let decision = network.parse::<DockerResourceDecision>()?;
        assert_eq!(decision.action(), DockerResourceAction::Create);
        let debug = format!("{decision:?}");
        assert!(!debug.contains(&id) && !debug.contains("new-private-network"));

        let volume = "volume:source_data=external:destination_data".parse::<DockerResourceDecision>()?;
        let mut decisions = DockerResourceDecisions::new();
        decisions.add(decision)?;
        decisions.add(volume.clone())?;
        assert_eq!(decisions.add(volume), Err(DockerResourceDecisionError::Duplicate));
        assert!(!format!("{decisions:?}").contains("source_data"));
        Ok(())
    }

    #[test]
    fn rejects_partial_ids_missing_destinations_and_unreviewed_spelling_without_echo()
    -> Result<(), DockerResourceDecisionError> {
        for spec in [
            "network:short=create:target",
            "network:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=create:target",
            "volume:/absolute=create:target",
            "volume:source=create:",
            "volume:source=transfer:target",
            "volume:source=create:target/other",
            "network:source=create:target",
        ] {
            let error = spec
                .parse::<DockerResourceDecision>()
                .err()
                .ok_or(DockerResourceDecisionError::Malformed)?;
            assert_eq!(error, DockerResourceDecisionError::Malformed);
            assert!(!format!("{error:?}{error}").contains(spec));
        }
        Ok(())
    }
}
