//! Explicit, resource-scoped declarations of ordinary public Docker metadata.

use std::{error::Error, fmt};

// BoxFerry reconstruction budgets, independent of native acquisition/decoder limits.
pub(crate) const PUBLIC_LABEL_VALUE_LIMIT: usize = 64 * 1024;
pub(crate) const PUBLIC_LABEL_TOTAL_LIMIT: usize = 8 * 1024 * 1024;

/// Exact inspected resource kind to which one public-label declaration applies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockerPublicLabelResourceKind {
    /// A selected inspected container, identified by its canonical full ID.
    Container,
    /// A selected inspected network, identified by its canonical full ID.
    Network,
    /// A selected inspected volume, identified by its exact native name.
    Volume,
}

/// Value-free rejection of a malformed, duplicate, or excessive declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DockerPublicLabelDecisionError {
    /// The source identity does not meet the resource kind's exact native grammar.
    InvalidSourceIdentity,
    /// The key is empty, exceeds 256 bytes, or contains NUL.
    InvalidKey,
    /// The same resource kind, source identity, and key appeared more than once.
    Duplicate,
    /// More than 128 declarations were supplied.
    TooMany,
}

impl fmt::Display for DockerPublicLabelDecisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSourceIdentity => "invalid Docker public-label source identity",
            Self::InvalidKey => "invalid Docker public-label key",
            Self::Duplicate => "duplicate Docker public-label declaration",
            Self::TooMany => "too many Docker public-label declarations",
        })
    }
}

impl Error for DockerPublicLabelDecisionError {}

/// Caller declaration that one observed label is ordinary, non-sensitive metadata.
///
/// This authorizes the actual inspected value to enter generated artifacts as public metadata;
/// it supplies neither a replacement value nor resource lifecycle intent. It is independent of
/// portable source promotion, protected environment consent, and target loss policy.
/// Import additionally bounds each observed value to 64 KiB and all copied keys/values
/// to 8 MiB. These are `BoxFerry` reconstruction budgets, not Docker API limits.
#[derive(Clone, Eq, PartialEq)]
pub struct DockerPublicLabelDecision {
    kind: DockerPublicLabelResourceKind,
    source: String,
    key: String,
}

impl DockerPublicLabelDecision {
    /// Checks a complete native source identity and one exact UTF-8 label key.
    ///
    /// # Errors
    ///
    /// Returns a value-free error for a noncanonical identity or invalid key.
    pub fn new(
        kind: DockerPublicLabelResourceKind,
        source_identity: String,
        key: String,
    ) -> Result<Self, DockerPublicLabelDecisionError> {
        let valid_source = match kind {
            DockerPublicLabelResourceKind::Container | DockerPublicLabelResourceKind::Network => {
                source_identity.len() == 64
                    && source_identity
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            }
            DockerPublicLabelResourceKind::Volume => crate::resource_decisions::valid_name(&source_identity),
        };
        if !valid_source {
            return Err(DockerPublicLabelDecisionError::InvalidSourceIdentity);
        }
        if key.is_empty() || key.len() > 256 || key.contains('\0') {
            return Err(DockerPublicLabelDecisionError::InvalidKey);
        }
        Ok(Self {
            kind,
            source: source_identity,
            key,
        })
    }

    /// Returns the exact inspected resource kind.
    #[must_use]
    pub const fn kind(&self) -> DockerPublicLabelResourceKind {
        self.kind
    }

    /// Explicitly exposes the source identity to the caller; diagnostics must not echo it.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Explicitly exposes the selected key; diagnostics must not echo it.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Debug for DockerPublicLabelDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerPublicLabelDecision")
            .field("kind", &self.kind)
            .field("source", &"[redacted]")
            .field("key", &"[redacted]")
            .finish()
    }
}

/// Bounded, duplicate-free declarations; empty by default, including portable imports.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct DockerPublicLabelDecisions {
    entries: Vec<DockerPublicLabelDecision>,
}

impl DockerPublicLabelDecisions {
    /// Validates at most 128 exact resource/key declarations.
    ///
    /// # Errors
    ///
    /// Returns a value-free error for duplicate declarations or an excessive count.
    pub fn new(entries: Vec<DockerPublicLabelDecision>) -> Result<Self, DockerPublicLabelDecisionError> {
        if entries.len() > 128 {
            return Err(DockerPublicLabelDecisionError::TooMany);
        }
        for (index, entry) in entries.iter().enumerate() {
            if entries[..index].iter().any(|other| other == entry) {
                return Err(DockerPublicLabelDecisionError::Duplicate);
            }
        }
        Ok(Self { entries })
    }

    pub(crate) const fn empty() -> Self {
        Self { entries: Vec::new() }
    }

    /// Returns only the explicitly supplied declarations in caller order.
    #[must_use]
    pub fn entries(&self) -> &[DockerPublicLabelDecision] {
        &self.entries
    }
}

impl fmt::Debug for DockerPublicLabelDecisions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockerPublicLabelDecisions")
            .field("count", &self.entries.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "public_labels_tests.rs"]
mod tests;
