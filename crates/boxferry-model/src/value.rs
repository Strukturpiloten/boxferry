//! Values that retain whether their contents are sensitive.

use std::fmt;

/// Plain or sensitive text with redacting debug output.
#[derive(Clone, Eq, PartialEq)]
pub struct ProtectedString {
    value: String,
    sensitive: bool,
}

impl ProtectedString {
    /// Creates ordinary non-sensitive text.
    #[must_use]
    pub fn plain(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: false,
        }
    }

    /// Creates sensitive text whose debug output is redacted.
    #[must_use]
    pub fn sensitive(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: true,
        }
    }

    /// Returns whether the value is sensitive.
    #[must_use]
    pub const fn is_sensitive(&self) -> bool {
        self.sensitive
    }

    /// Explicitly exposes the contained text to an authorized caller.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// Returns the value or the standard redaction marker.
    #[must_use]
    pub fn redacted(&self) -> &str {
        if self.sensitive { "[REDACTED]" } else { &self.value }
    }
}

impl fmt::Debug for ProtectedString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtectedString")
            .field(&self.redacted())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::ProtectedString;

    #[test]
    fn sensitive_debug_output_is_redacted() {
        let value = ProtectedString::sensitive("never-print-this");
        let debug = format!("{value:?}");
        assert!(!debug.contains("never-print-this"));
        assert!(debug.contains("[REDACTED]"));
        assert_eq!(value.expose(), "never-print-this");
    }
}
