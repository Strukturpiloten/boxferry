//! Structured diagnostics with explicit sensitive fields.

use std::{error::Error, fmt};

/// Error returned for an invalid machine-readable diagnostic code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidDiagnosticCode;

impl fmt::Display for InvalidDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("diagnostic code must contain only uppercase ASCII letters and digits")
    }
}

impl Error for InvalidDiagnosticCode {}

/// Stable machine-readable diagnostic identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    /// Creates an uppercase ASCII alphanumeric code such as `BFE0001`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] for an empty or nonconforming code.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidDiagnosticCode> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(InvalidDiagnosticCode);
        }
        Ok(Self(value))
    }

    /// Returns the code string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Diagnostic severity independent from presentation and process exit codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Severity {
    /// Conversion cannot continue safely.
    Error,
    /// Conversion can continue only under an explicit loss policy.
    Warning,
    /// Context that does not change conversion fidelity.
    Note,
}

/// Plain or sensitive diagnostic field value.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum DiagnosticValue {
    /// Non-sensitive text that presentation may show.
    Plain(String),
    /// Sensitive text that presentation and debug output must redact.
    Sensitive(String),
}

impl DiagnosticValue {
    /// Creates a non-sensitive value.
    #[must_use]
    pub fn plain(value: impl Into<String>) -> Self {
        Self::Plain(value.into())
    }

    /// Creates a sensitive value.
    #[must_use]
    pub fn sensitive(value: impl Into<String>) -> Self {
        Self::Sensitive(value.into())
    }

    /// Returns whether the value is sensitive.
    #[must_use]
    pub const fn is_sensitive(&self) -> bool {
        matches!(self, Self::Sensitive(_))
    }

    /// Explicitly exposes the original field value.
    #[must_use]
    pub fn expose(&self) -> &str {
        match self {
            Self::Plain(value) | Self::Sensitive(value) => value,
        }
    }

    /// Returns presentation-safe text.
    #[must_use]
    pub fn redacted(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Sensitive(_) => "[REDACTED]",
        }
    }
}

impl fmt::Debug for DiagnosticValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DiagnosticValue")
            .field(&self.redacted())
            .finish()
    }
}

/// Named structured context attached to a diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticField {
    name: String,
    value: DiagnosticValue,
}

impl DiagnosticField {
    /// Creates a named field.
    #[must_use]
    pub fn new(name: impl Into<String>, value: DiagnosticValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Returns the field name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the protected field value.
    #[must_use]
    pub const fn value(&self) -> &DiagnosticValue {
        &self.value
    }
}

/// Structured conversion diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    severity: Severity,
    summary: String,
    fields: Vec<DiagnosticField>,
}

impl Diagnostic {
    /// Creates a value-free summary with no fields.
    #[must_use]
    pub fn new(code: DiagnosticCode, severity: Severity, summary: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            summary: summary.into(),
            fields: Vec::new(),
        }
    }

    /// Appends structured context in presentation order.
    #[must_use]
    pub fn with_field(mut self, field: DiagnosticField) -> Self {
        self.fields.push(field);
        self
    }

    /// Returns the stable code.
    #[must_use]
    pub const fn code(&self) -> &DiagnosticCode {
        &self.code
    }

    /// Returns the severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the human-readable, value-free summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Returns structured fields in presentation order.
    #[must_use]
    pub fn fields(&self) -> &[DiagnosticField] {
        &self.fields
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.summary)?;
        for field in &self.fields {
            write!(formatter, " {}={}", field.name(), field.value().redacted())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, Severity};

    #[test]
    fn sensitive_fields_are_redacted_from_debug_and_display() -> Result<(), String> {
        let diagnostic = Diagnostic::new(code("BFE0001")?, Severity::Warning, "value was adjusted").with_field(
            DiagnosticField::new("value", DiagnosticValue::sensitive("never-print-this")),
        );
        for rendered in [format!("{diagnostic:?}"), diagnostic.to_string()] {
            assert!(!rendered.contains("never-print-this"));
            assert!(rendered.contains("[REDACTED]"));
        }
        Ok(())
    }

    fn code(value: &str) -> Result<DiagnosticCode, String> {
        DiagnosticCode::new(value).map_err(|error| error.to_string())
    }
}
