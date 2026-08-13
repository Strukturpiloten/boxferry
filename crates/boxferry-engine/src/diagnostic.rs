//! Structured diagnostics with explicit sensitive fields.

use std::{error::Error, fmt};

/// Producer-neutral role of one source location attached to a native finding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeFindingLabelKind {
    /// Location primarily responsible for the finding.
    Primary,
    /// Related location that adds context.
    Secondary,
}

/// One value-free source location attached to a native-format finding.
///
/// Numeric source identities are invocation-local. Adapters map them to caller-owned aliases;
/// source paths and source contents never enter this DTO.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NativeFindingLabel {
    kind: NativeFindingLabelKind,
    source_id: u32,
    start: usize,
    end: usize,
    message: String,
}

impl NativeFindingLabel {
    /// Creates a labelled half-open byte range.
    #[must_use]
    pub fn new(
        kind: NativeFindingLabelKind,
        source_id: u32,
        start: usize,
        end: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            source_id,
            start,
            end,
            message: message.into(),
        }
    }

    /// Returns the location role.
    #[must_use]
    pub const fn kind(&self) -> NativeFindingLabelKind {
        self.kind
    }

    /// Returns the invocation-local numeric source identity.
    #[must_use]
    pub const fn source_id(&self) -> u32 {
        self.source_id
    }

    /// Returns the inclusive byte offset.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// Returns the exclusive byte offset.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }

    /// Returns the value-free location explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

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

/// A native parser, model, runtime, or platform finding retained at an adapter boundary.
///
/// This envelope is intentionally independent of Compose, Quadlet, Docker, Podman, and
/// Kubernetes types. Native libraries keep ownership of their codes; `BoxFerry` retains those codes
/// as provenance while applying its own rule and loss policy in [`Diagnostic`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NativeFinding {
    source_format: String,
    producer: String,
    producer_version: Option<String>,
    code: String,
    stage: String,
    severity: Severity,
    summary: String,
    fields: Vec<DiagnosticField>,
    labels: Vec<NativeFindingLabel>,
    notes: Vec<String>,
    help: Option<String>,
}

impl NativeFinding {
    /// Creates a value-free native finding.
    #[must_use]
    pub fn new(
        source_format: impl Into<String>,
        producer: impl Into<String>,
        code: impl Into<String>,
        stage: impl Into<String>,
        severity: Severity,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            source_format: source_format.into(),
            producer: producer.into(),
            producer_version: None,
            code: code.into(),
            stage: stage.into(),
            severity,
            summary: summary.into(),
            fields: Vec::new(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    /// Attaches the exact native producer version when the adapter can prove it.
    #[must_use]
    pub fn with_producer_version(mut self, version: impl Into<String>) -> Self {
        self.producer_version = Some(version.into());
        self
    }

    /// Appends protected structured native context.
    #[must_use]
    pub fn with_field(mut self, field: DiagnosticField) -> Self {
        self.fields.push(field);
        self
    }

    /// Appends a value-free labelled location.
    #[must_use]
    pub fn with_label(mut self, label: NativeFindingLabel) -> Self {
        self.labels.push(label);
        self
    }

    /// Appends value-free native context.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Attaches native remediation distinct from the `BoxFerry` rule help.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Returns the source format, such as `compose` or `quadlet`.
    #[must_use]
    pub fn source_format(&self) -> &str {
        &self.source_format
    }

    /// Returns the native producer, such as `compose-lens`.
    #[must_use]
    pub fn producer(&self) -> &str {
        &self.producer
    }

    /// Returns the exact producer version when recorded.
    #[must_use]
    pub fn producer_version(&self) -> Option<&str> {
        self.producer_version.as_deref()
    }

    /// Returns the producer-owned stable code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the native processing stage.
    #[must_use]
    pub fn stage(&self) -> &str {
        &self.stage
    }

    /// Returns the native severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the value-free native summary.
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Returns protected structured native context.
    #[must_use]
    pub fn fields(&self) -> &[DiagnosticField] {
        &self.fields
    }

    /// Returns labelled native locations in producer order.
    #[must_use]
    pub fn labels(&self) -> &[NativeFindingLabel] {
        &self.labels
    }

    /// Returns value-free producer notes.
    #[must_use]
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Returns producer remediation when recorded.
    #[must_use]
    pub fn help(&self) -> Option<&str> {
        self.help.as_deref()
    }
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
    native_finding: Option<NativeFinding>,
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
            native_finding: None,
        }
    }

    /// Appends structured context in presentation order.
    #[must_use]
    pub fn with_field(mut self, field: DiagnosticField) -> Self {
        self.fields.push(field);
        self
    }

    /// Attaches the native finding that caused this `BoxFerry` rule occurrence.
    #[must_use]
    pub fn with_native_finding(mut self, finding: NativeFinding) -> Self {
        self.native_finding = Some(finding);
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

    /// Returns the native finding retained by the source adapter.
    #[must_use]
    pub const fn native_finding(&self) -> Option<&NativeFinding> {
        self.native_finding.as_ref()
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
    use super::{
        Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, NativeFinding, NativeFindingLabel,
        NativeFindingLabelKind, Severity,
    };

    #[test]
    fn sensitive_fields_are_redacted_from_debug_and_display() -> Result<(), String> {
        let finding = NativeFinding::new(
            "compose",
            "compose-lens",
            "compose.example",
            "model",
            Severity::Warning,
            "native value needs review",
        )
        .with_field(DiagnosticField::new(
            "native_value",
            DiagnosticValue::sensitive("never-print-native-this"),
        ))
        .with_label(NativeFindingLabel::new(
            NativeFindingLabelKind::Primary,
            1,
            4,
            8,
            "value is here",
        ));
        let diagnostic = Diagnostic::new(code("BFE0001")?, Severity::Warning, "value was adjusted")
            .with_field(DiagnosticField::new(
                "value",
                DiagnosticValue::sensitive("never-print-this"),
            ))
            .with_native_finding(finding);
        for rendered in [format!("{diagnostic:?}"), diagnostic.to_string()] {
            assert!(!rendered.contains("never-print-this"));
            assert!(!rendered.contains("never-print-native-this"));
            assert!(rendered.contains("[REDACTED]"));
        }
        let native = diagnostic.native_finding().ok_or("missing native finding")?;
        assert_eq!(native.producer(), "compose-lens");
        assert_eq!(native.labels()[0].source_id(), 1);
        Ok(())
    }

    fn code(value: &str) -> Result<DiagnosticCode, String> {
        DiagnosticCode::new(value).map_err(|error| error.to_string())
    }
}
