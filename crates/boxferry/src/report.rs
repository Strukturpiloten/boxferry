//! Versioned, privacy-safe conversion reports.
//!
//! The report model is intentionally independent of terminal presentation and of
//! any individual input or output format.  Serialization is available with the
//! additive `cli` feature; the DTO itself remains available to no-default-feature
//! embedded users.
#![allow(missing_docs)] // Field names are the v1 JSON Schema contract.

/// The only schema version currently emitted by `BoxFerry`.
pub const SCHEMA_VERSION: u32 = 1;
/// Replacement used for every redacted value.
pub const REDACTED: &str = "<redacted>";
/// Maximum number of diagnostics or events in a v1 report.
pub const MAX_COLLECTION_ITEMS: usize = 2_048;
/// Maximum UTF-8 byte length of a v1 text field.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Maximum encoded JSON byte length of a v1 report.
pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", serde(rename_all = "snake_case"))]
pub enum ReportStatus {
    Success,
    Blocked,
    Failure,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", serde(rename_all = "kebab-case"))]
pub enum ExitCategory {
    Success,
    PolicyBlocked,
    InputOrExecution,
    OutputWrite,
    ReportWrite,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "cli", serde(rename_all = "kebab-case"))]
pub enum FailedStage {
    InputDiscovery,
    InputRead,
    Interpolation,
    ComposeLoad,
    ComposeMerge,
    ProfileSelection,
    Conversion,
    OutputWrite,
    ReportWrite,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionBounds {
    pub minimum: String,
    pub maximum: String,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportInput {
    pub alias: String,
    pub kind: String,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDecision {
    pub selected: String,
    pub ignored: Vec<String>,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportChoice {
    pub name: String,
    pub value: String,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedInvocation {
    pub command_kind: String,
    pub provided_option_names: Vec<String>,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostMetadata {
    pub os_family: String,
    pub architecture: String,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FidelityCounts {
    pub exact: usize,
    pub approximate: usize,
    pub unsupported: usize,
    pub invalid: usize,
    pub other: usize,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportSpan {
    pub source: String,
    pub start: usize,
    pub end: usize,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportField {
    pub name: String,
    pub value: String,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportDiagnostic {
    pub code: String,
    pub severity: String,
    pub summary: String,
    pub fields: Vec<ReportField>,
    pub spans: Vec<ReportSpan>,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputArtifact {
    pub name: String,
    pub size: u64,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionSummary {
    pub classes: Vec<String>,
    pub count: usize,
    pub review_required: bool,
}

#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Truncation {
    pub field: String,
    pub original: usize,
    pub retained: usize,
}

/// Stable structured result for a conversion invocation.
#[cfg_attr(feature = "cli", derive(serde::Serialize))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionReport {
    pub schema_version: u32,
    pub boxferry_version: String,
    pub status: ReportStatus,
    pub exit_category: ExitCategory,
    pub failed_stage: Option<FailedStage>,
    pub source_type: String,
    pub target_type: String,
    pub application: Option<String>,
    pub inputs: Vec<ReportInput>,
    pub discovery: Vec<DiscoveryDecision>,
    pub choices: Vec<ReportChoice>,
    pub invocation: SanitizedInvocation,
    pub host: HostMetadata,
    pub fidelity: FidelityCounts,
    pub requested_versions: VersionBounds,
    pub resolved_versions: VersionBounds,
    pub diagnostics: Vec<ReportDiagnostic>,
    pub events: Vec<String>,
    pub output_artifacts: Vec<OutputArtifact>,
    pub review_required: bool,
    pub redaction: RedactionSummary,
    pub truncations: Vec<Truncation>,
}

impl ConversionReport {
    /// Creates an empty v1 report with explicit route and version bounds.
    #[must_use]
    pub fn new(
        boxferry_version: impl Into<String>,
        source_type: impl Into<String>,
        target_type: impl Into<String>,
        requested_versions: VersionBounds,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            boxferry_version: boxferry_version.into(),
            status: ReportStatus::Failure,
            exit_category: ExitCategory::InputOrExecution,
            failed_stage: None,
            source_type: source_type.into(),
            target_type: target_type.into(),
            application: None,
            inputs: Vec::new(),
            discovery: Vec::new(),
            choices: Vec::new(),
            invocation: SanitizedInvocation {
                command_kind: "unknown".into(),
                provided_option_names: Vec::new(),
            },
            host: HostMetadata {
                os_family: "unknown".into(),
                architecture: "unknown".into(),
            },
            fidelity: FidelityCounts::default(),
            requested_versions,
            resolved_versions: VersionBounds {
                minimum: "unresolved".into(),
                maximum: "unresolved".into(),
            },
            diagnostics: Vec::new(),
            events: Vec::new(),
            output_artifacts: Vec::new(),
            review_required: true,
            redaction: RedactionSummary {
                classes: Vec::new(),
                count: 0,
                review_required: true,
            },
            truncations: Vec::new(),
        }
    }

    /// Applies v1 collection and text limits, recording each lossy bound.
    pub fn enforce_v1_limits(&mut self) {
        truncate_collection(&mut self.inputs, "inputs", &mut self.truncations);
        truncate_collection(&mut self.discovery, "discovery", &mut self.truncations);
        truncate_collection(&mut self.choices, "choices", &mut self.truncations);
        truncate_collection(
            &mut self.invocation.provided_option_names,
            "invocation.provided_option_names",
            &mut self.truncations,
        );
        truncate_collection(&mut self.diagnostics, "diagnostics", &mut self.truncations);
        truncate_collection(&mut self.events, "events", &mut self.truncations);
        truncate_collection(&mut self.output_artifacts, "output_artifacts", &mut self.truncations);
        truncate_collection(&mut self.redaction.classes, "redaction.classes", &mut self.truncations);
        truncate_collection(&mut self.truncations, "truncations", &mut Vec::new());
        truncate_text(&mut self.boxferry_version, "boxferry_version", &mut self.truncations);
        truncate_text(&mut self.source_type, "source_type", &mut self.truncations);
        truncate_text(&mut self.target_type, "target_type", &mut self.truncations);
        if let Some(application) = &mut self.application {
            truncate_text(application, "application", &mut self.truncations);
        }
        truncate_text(
            &mut self.requested_versions.minimum,
            "requested_versions.minimum",
            &mut self.truncations,
        );
        truncate_text(
            &mut self.requested_versions.maximum,
            "requested_versions.maximum",
            &mut self.truncations,
        );
        truncate_text(
            &mut self.resolved_versions.minimum,
            "resolved_versions.minimum",
            &mut self.truncations,
        );
        truncate_text(
            &mut self.resolved_versions.maximum,
            "resolved_versions.maximum",
            &mut self.truncations,
        );
        for input in &mut self.inputs {
            truncate_text(&mut input.alias, "inputs.alias", &mut self.truncations);
            truncate_text(&mut input.kind, "inputs.kind", &mut self.truncations);
        }
        for discovery in &mut self.discovery {
            truncate_text(&mut discovery.selected, "discovery.selected", &mut self.truncations);
            truncate_collection(&mut discovery.ignored, "discovery.ignored", &mut self.truncations);
            for ignored in &mut discovery.ignored {
                truncate_text(ignored, "discovery.ignored", &mut self.truncations);
            }
        }
        for choice in &mut self.choices {
            truncate_text(&mut choice.name, "choices.name", &mut self.truncations);
            truncate_text(&mut choice.value, "choices.value", &mut self.truncations);
        }
        truncate_text(
            &mut self.invocation.command_kind,
            "invocation.command_kind",
            &mut self.truncations,
        );
        for option_name in &mut self.invocation.provided_option_names {
            truncate_text(option_name, "invocation.provided_option_names", &mut self.truncations);
        }
        truncate_text(&mut self.host.os_family, "host.os_family", &mut self.truncations);
        truncate_text(&mut self.host.architecture, "host.architecture", &mut self.truncations);
        for diagnostic in &mut self.diagnostics {
            truncate_collection(&mut diagnostic.fields, "diagnostics.fields", &mut self.truncations);
            truncate_collection(&mut diagnostic.spans, "diagnostics.spans", &mut self.truncations);
            truncate_text(&mut diagnostic.code, "diagnostics.code", &mut self.truncations);
            truncate_text(&mut diagnostic.severity, "diagnostics.severity", &mut self.truncations);
            truncate_text(&mut diagnostic.summary, "diagnostics.summary", &mut self.truncations);
            for field in &mut diagnostic.fields {
                truncate_text(&mut field.name, "diagnostics.fields.name", &mut self.truncations);
                truncate_text(&mut field.value, "diagnostics.fields.value", &mut self.truncations);
            }
            for span in &mut diagnostic.spans {
                truncate_text(&mut span.source, "diagnostics.spans.source", &mut self.truncations);
            }
        }
        for event in &mut self.events {
            truncate_text(event, "events", &mut self.truncations);
        }
        for artifact in &mut self.output_artifacts {
            truncate_text(&mut artifact.name, "output_artifacts.name", &mut self.truncations);
        }
        for class in &mut self.redaction.classes {
            truncate_text(class, "redaction.classes", &mut self.truncations);
        }
        for truncation in &mut self.truncations {
            truncate_text(&mut truncation.field, "truncations.field", &mut Vec::new());
        }
    }

    /// Drops one deterministic bounded report item when JSON still exceeds v1.
    pub fn reduce_for_json(&mut self) -> bool {
        if let Some(diagnostic) = self
            .diagnostics
            .iter_mut()
            .rev()
            .find(|diagnostic| !diagnostic.fields.is_empty())
        {
            let original = diagnostic.fields.len();
            diagnostic.fields.pop();
            record_truncation(&mut self.truncations, "diagnostics.fields", original, original - 1);
            return true;
        }
        if let Some(diagnostic) = self
            .diagnostics
            .iter_mut()
            .rev()
            .find(|diagnostic| !diagnostic.spans.is_empty())
        {
            let original = diagnostic.spans.len();
            diagnostic.spans.pop();
            record_truncation(&mut self.truncations, "diagnostics.spans", original, original - 1);
            return true;
        }
        if !self.diagnostics.is_empty() {
            let original = self.diagnostics.len();
            self.diagnostics.pop();
            record_truncation(&mut self.truncations, "diagnostics", original, original - 1);
            return true;
        }
        if !self.events.is_empty() {
            let original = self.events.len();
            self.events.pop();
            record_truncation(&mut self.truncations, "events", original, original - 1);
            return true;
        }
        if !self.output_artifacts.is_empty() {
            let original = self.output_artifacts.len();
            self.output_artifacts.pop();
            record_truncation(&mut self.truncations, "output_artifacts", original, original - 1);
            return true;
        }
        if let Some(discovery) = self
            .discovery
            .iter_mut()
            .rev()
            .find(|discovery| !discovery.ignored.is_empty())
        {
            let original = discovery.ignored.len();
            discovery.ignored.pop();
            record_truncation(&mut self.truncations, "discovery.ignored", original, original - 1);
            return true;
        }
        if !self.discovery.is_empty() {
            let original = self.discovery.len();
            self.discovery.pop();
            record_truncation(&mut self.truncations, "discovery", original, original - 1);
            return true;
        }
        if !self.inputs.is_empty() {
            let original = self.inputs.len();
            self.inputs.pop();
            record_truncation(&mut self.truncations, "inputs", original, original - 1);
            return true;
        }
        if !self.choices.is_empty() {
            let original = self.choices.len();
            self.choices.pop();
            record_truncation(&mut self.truncations, "choices", original, original - 1);
            return true;
        }
        if !self.invocation.provided_option_names.is_empty() {
            let original = self.invocation.provided_option_names.len();
            self.invocation.provided_option_names.pop();
            record_truncation(
                &mut self.truncations,
                "invocation.provided_option_names",
                original,
                original - 1,
            );
            return true;
        }
        if !self.redaction.classes.is_empty() {
            let original = self.redaction.classes.len();
            self.redaction.classes.pop();
            record_truncation(&mut self.truncations, "redaction.classes", original, original - 1);
            return true;
        }
        false
    }
}

fn truncate_collection<T>(values: &mut Vec<T>, field: &str, truncations: &mut Vec<Truncation>) {
    if values.len() > MAX_COLLECTION_ITEMS {
        let original = values.len();
        values.truncate(MAX_COLLECTION_ITEMS);
        record_truncation(truncations, field, original, MAX_COLLECTION_ITEMS);
    }
}

fn truncate_text(value: &mut String, field: &str, truncations: &mut Vec<Truncation>) {
    if value.len() <= MAX_TEXT_BYTES {
        return;
    }
    let original = value.len();
    let mut retained = MAX_TEXT_BYTES;
    while !value.is_char_boundary(retained) {
        retained -= 1;
    }
    value.truncate(retained);
    record_truncation(truncations, field, original, retained);
}

fn record_truncation(truncations: &mut Vec<Truncation>, field: &str, original: usize, retained: usize) {
    if let Some(existing) = truncations.iter_mut().find(|existing| existing.field == field) {
        existing.original = existing.original.max(original);
        existing.retained = existing.retained.min(retained);
        return;
    }
    if truncations.len() < MAX_COLLECTION_ITEMS {
        truncations.push(Truncation {
            field: field.into(),
            original,
            retained,
        });
    }
}

/// Whether a field name denotes a credential or protected value.
#[must_use]
pub fn is_protected_name(name: &str) -> bool {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && previous_lower && !current.is_empty() {
                words.push(current.to_ascii_lowercase());
                current.clear();
            }
            current.push(character);
            previous_lower = character.is_ascii_lowercase();
        } else {
            if !current.is_empty() {
                words.push(current.to_ascii_lowercase());
                current.clear();
            }
            previous_lower = false;
        }
    }
    if !current.is_empty() {
        words.push(current.to_ascii_lowercase());
    }
    let has = |word: &str| words.iter().any(|item| item == word);
    has("password")
        || has("passwd")
        || has("passphrase")
        || has("pwd")
        || has("pw")
        || has("secret")
        || has("token")
        || has("credential")
        || has("credentials")
        || has("authorization")
        || (has("private") && has("key"))
        || (has("api") && has("key"))
        || has("apikey")
}

/// Redacts sensitive field values and common secret-bearing plain text patterns.
#[must_use]
pub fn redact_text(field_name: &str, value: &str, explicitly_sensitive: bool) -> (String, bool) {
    if explicitly_sensitive || is_protected_name(field_name) || plain_text_looks_protected(value) {
        return (REDACTED.into(), true);
    }
    (value.into(), false)
}

fn plain_text_looks_protected(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    if lowered.contains("-----begin") && lowered.contains("private key-----") {
        return true;
    }
    if lowered.starts_with("authorization:") || lowered.contains("\nauthorization:") {
        return true;
    }
    if let Some(scheme) = value.find("://") {
        if value[scheme + 3..].contains('@') {
            return true;
        }
    }
    value
        .split_whitespace()
        .any(|part| part.split_once('=').is_some_and(|(name, _)| is_protected_name(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_spelling_and_plain_text_defences_redact() {
        for name in [
            "DB_PASSWORD",
            "databasePassword",
            "client-secret",
            "DB_PW",
            "api_key",
            "privateKey",
        ] {
            assert_eq!(redact_text(name, "canary", false).0, REDACTED, "{name}");
        }
        for value in [
            "https://user:canary@example.invalid",
            "Authorization: Bearer canary",
            "-----BEGIN PRIVATE KEY-----\ncanary",
        ] {
            assert_eq!(redact_text("message", value, false).0, REDACTED);
        }
    }

    #[test]
    fn v1_limits_are_explicit() {
        let mut report = ConversionReport::new(
            "version",
            "compose",
            "quadlet",
            VersionBounds {
                minimum: "5.4".into(),
                maximum: "6.0".into(),
            },
        );
        report.events = (0..=MAX_COLLECTION_ITEMS).map(|item| item.to_string()).collect();
        report.diagnostics.push(ReportDiagnostic {
            code: "BFC0001".into(),
            severity: "error".into(),
            summary: "x".repeat(MAX_TEXT_BYTES + 1),
            fields: Vec::new(),
            spans: Vec::new(),
        });
        report.enforce_v1_limits();
        assert_eq!(report.events.len(), MAX_COLLECTION_ITEMS);
        assert!(report.truncations.iter().any(|truncation| truncation.field == "events"));
        assert!(
            report
                .truncations
                .iter()
                .any(|truncation| truncation.field == "diagnostics.summary")
        );
    }
}
