//! Bounded inert vendor metadata, independent of native configuration languages.
use crate::{Identifier, ProtectedString, Sourced};
use std::{error::Error, fmt};

/// Maximum supported recursive metadata depth.
pub const METADATA_MAX_DEPTH: usize = 64;
/// Maximum total metadata text and node accounting bytes per application.
pub const METADATA_MAX_BYTES: usize = 1024 * 1024;

/// Owner of an inert vendor extension.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataScope {
    /// Application-root extension.
    Application,
    /// Direct service extension, identified by its neutral service name.
    Service(Identifier),
}

/// One ordered string-keyed mapping entry, including key provenance and sensitivity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataEntry {
    key: Sourced<ProtectedString>,
    value: Sourced<MetadataNode>,
}
impl MetadataEntry {
    /// Creates an ordered mapping entry.
    #[must_use]
    pub const fn new(key: Sourced<ProtectedString>, value: Sourced<MetadataNode>) -> Self {
        Self { key, value }
    }
    /// Returns the original key and its source evidence.
    #[must_use]
    pub const fn key(&self) -> &Sourced<ProtectedString> {
        &self.key
    }
    /// Returns the nested value and its source evidence.
    #[must_use]
    pub const fn value(&self) -> &Sourced<MetadataNode> {
        &self.value
    }
}

/// Semantic value of vendor metadata. Map and sequence order is retained.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataValue {
    /// Null value.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// JSON-compatible decimal number spelling, without floating-point conversion.
    Number(ProtectedString),
    /// Semantic string value.
    String(ProtectedString),
    /// Ordered nested sequence.
    Sequence(Vec<Sourced<MetadataNode>>),
    /// Ordered string-keyed mapping.
    Mapping(Vec<MetadataEntry>),
}

impl fmt::Debug for MetadataValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("Null"),
            // A directly extracted enum does not carry its enclosing node's sensitivity bit.
            Self::Boolean(_) => f.write_str("Boolean([REDACTED])"),
            Self::Number(value) => f.debug_tuple("Number").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Sequence(values) => f.debug_tuple("Sequence").field(values).finish(),
            Self::Mapping(entries) => f.debug_tuple("Mapping").field(entries).finish(),
        }
    }
}

/// One value with its own sensitivity and source folded-text presentation hint.
#[derive(Clone, Eq, PartialEq)]
pub struct MetadataNode {
    value: MetadataValue,
    sensitive: bool,
    folded: bool,
}
impl MetadataNode {
    /// Creates a bounded node. The folded hint is valid only for strings.
    /// Protecting a collection also protects every nested value and mapping key.
    ///
    /// # Errors
    /// Rejects invalid decimal spellings, duplicate mapping keys, invalid folded hints and limits.
    pub fn new(value: MetadataValue, sensitive: bool, folded: bool) -> Result<Self, MetadataError> {
        let mut node = Self {
            value,
            sensitive,
            folded,
        };
        node.measure(1)?;
        if node.is_sensitive() {
            node.protect_contents();
        }
        Ok(node)
    }
    /// Returns the typed semantic value.
    #[must_use]
    pub const fn value(&self) -> &MetadataValue {
        &self.value
    }
    /// Returns the sensitivity of this node, independently of descendants.
    #[must_use]
    pub fn is_sensitive(&self) -> bool {
        self.sensitive
            || matches!(&self.value, MetadataValue::String(value) | MetadataValue::Number(value) if value.is_sensitive())
    }
    /// Returns whether the source requested folded text presentation.
    #[must_use]
    pub const fn is_folded(&self) -> bool {
        self.folded
    }
    /// Returns whether this node, any descendant or mapping key is protected.
    #[must_use]
    pub fn contains_sensitive(&self) -> bool {
        self.is_sensitive()
            || match &self.value {
                MetadataValue::Sequence(values) => values.iter().any(|value| value.value().contains_sensitive()),
                MetadataValue::Mapping(entries) => entries
                    .iter()
                    .any(|entry| entry.key.value().is_sensitive() || entry.value.value().contains_sensitive()),
                _ => false,
            }
    }
    fn protect_contents(&mut self) {
        self.sensitive = true;
        match &mut self.value {
            MetadataValue::String(value) | MetadataValue::Number(value) => {
                *value = ProtectedString::sensitive(value.expose());
            }
            MetadataValue::Sequence(values) => {
                for value in values {
                    value.value_mut().protect_contents();
                }
            }
            MetadataValue::Mapping(entries) => {
                for entry in entries {
                    *entry.key.value_mut() = ProtectedString::sensitive(entry.key.value().expose());
                    entry.value.value_mut().protect_contents();
                }
            }
            MetadataValue::Null | MetadataValue::Boolean(_) => {}
        }
    }

    pub(crate) fn measure(&self, depth: usize) -> Result<usize, MetadataError> {
        if depth > METADATA_MAX_DEPTH || (self.folded && !matches!(self.value, MetadataValue::String(_))) {
            return Err(MetadataError::InvalidValue);
        }
        let mut bytes = 32_usize;
        match &self.value {
            MetadataValue::Number(value) => {
                if !valid_number(value.expose()) {
                    return Err(MetadataError::InvalidValue);
                }
                bytes += value.expose().len();
            }
            MetadataValue::String(value) => bytes += value.expose().len(),
            MetadataValue::Sequence(values) => {
                for value in values {
                    bytes = bytes
                        .checked_add(value.value().measure(depth + 1)?)
                        .ok_or(MetadataError::Limit)?;
                }
            }
            MetadataValue::Mapping(entries) => {
                let mut keys = std::collections::BTreeSet::new();
                for entry in entries {
                    if !keys.insert(entry.key.value().expose()) {
                        return Err(MetadataError::Duplicate);
                    }
                    bytes = bytes
                        .checked_add(entry.key.value().expose().len())
                        .and_then(|bytes| bytes.checked_add(32))
                        .ok_or(MetadataError::Limit)?;
                    bytes = bytes
                        .checked_add(entry.value.value().measure(depth + 1)?)
                        .ok_or(MetadataError::Limit)?;
                }
            }
            _ => {}
        }
        if bytes > METADATA_MAX_BYTES {
            return Err(MetadataError::Limit);
        }
        Ok(bytes)
    }
}
impl fmt::Debug for MetadataNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_sensitive() {
            f.write_str("MetadataNode([REDACTED])")
        } else {
            f.debug_struct("MetadataNode")
                .field("value", &self.value)
                .field("sensitive", &self.sensitive)
                .field("folded", &self.folded)
                .finish()
        }
    }
}

/// One exact, case-sensitive vendor namespace at a supported neutral owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VendorMetadata {
    scope: MetadataScope,
    namespace: Sourced<ProtectedString>,
    payload: Sourced<MetadataNode>,
}
impl VendorMetadata {
    /// Creates an inert extension. Namespace validation does not derive a printable target section.
    ///
    /// # Errors
    /// Rejects empty/non-extension namespace spellings and bounded payload violations.
    pub fn new(
        scope: MetadataScope,
        namespace: Sourced<ProtectedString>,
        payload: Sourced<MetadataNode>,
    ) -> Result<Self, MetadataError> {
        if !namespace.value().expose().starts_with("x-") || namespace.value().expose().len() <= 2 {
            return Err(MetadataError::InvalidValue);
        }
        payload.value().measure(1)?;
        Ok(Self {
            scope,
            namespace,
            payload,
        })
    }
    /// Returns the owner.
    #[must_use]
    pub const fn scope(&self) -> &MetadataScope {
        &self.scope
    }
    /// Returns the exact original namespace and source evidence.
    #[must_use]
    pub const fn namespace(&self) -> &Sourced<ProtectedString> {
        &self.namespace
    }
    /// Returns the inert semantic tree.
    #[must_use]
    pub const fn payload(&self) -> &Sourced<MetadataNode> {
        &self.payload
    }
    /// Returns whether any namespace, key or value is protected.
    #[must_use]
    pub fn contains_sensitive(&self) -> bool {
        self.namespace.value().is_sensitive() || self.payload.value().contains_sensitive()
    }
    pub(crate) fn measure(&self) -> Result<usize, MetadataError> {
        self.payload
            .value()
            .measure(1)?
            .checked_add(self.namespace.value().expose().len())
            .ok_or(MetadataError::Limit)
    }
}

/// Value-free validation error for inert metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MetadataError {
    /// A shape, number, namespace or presentation hint is invalid.
    InvalidValue,
    /// A case-sensitive identity or map key occurs twice.
    Duplicate,
    /// Service owner does not exist.
    UnknownOwner,
    /// A finite size or recursion budget was exceeded.
    Limit,
}
impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidValue => "invalid vendor metadata value",
            Self::Duplicate => "duplicate vendor metadata identity or key",
            Self::UnknownOwner => "vendor metadata references an absent service",
            Self::Limit => "vendor metadata exceeds supported bounds",
        })
    }
}
impl Error for MetadataError {}

fn valid_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = usize::from(bytes.first() == Some(&b'-'));
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if start == i {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if start == i {
            return false;
        }
    }
    i == bytes.len()
}
