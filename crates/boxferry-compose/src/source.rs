//! Explicit input bundle for one processed Compose project.

use std::{collections::BTreeMap, fmt};

use boxferry_engine::{
    Diagnostic, DiagnosticField, DiagnosticValue, InvalidDiagnosticCode, NativeFinding, NativeFindingLabel,
    NativeFindingLabelKind, RuleId, Severity,
};
use boxferry_model::{Identifier, ModelError, SourceId};
use compose_lens::{
    diagnostic::{Diagnostic as ComposeDiagnostic, LabelKind, Severity as ComposeSeverity},
    merge::MergedProject,
    profiles::ProfileSelection,
    render::render_canonical,
    source::SourceId as ComposeSourceId,
};

/// One canonical Compose document produced directly from a processed native Compose source.
///
/// Native canonicalization retains valid Compose-only values, including unresolved interpolation
/// expressions, without reading ambient environment values. The document deliberately redacts its
/// complete text from `Debug` output because source literals and expression defaults can contain
/// secrets even when no interpolation input marked them sensitive.
#[derive(Clone, Eq, PartialEq)]
pub struct CanonicalComposeDocument {
    text: String,
    sensitive: bool,
}

impl CanonicalComposeDocument {
    /// Returns deployable canonical Compose YAML.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Reports whether `ComposeLens` found protected rendered content.
    #[must_use]
    pub const fn is_sensitive(&self) -> bool {
        self.sensitive
    }
}

impl fmt::Debug for CanonicalComposeDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CanonicalComposeDocument")
            .field("text", &"<redacted>")
            .field("sensitive", &self.sensitive)
            .finish()
    }
}

/// Result of native Compose-to-Compose canonicalization.
///
/// An error-level retained or rendering finding suppresses the document. Warnings and notes remain
/// attached while allowing canonical output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeCanonicalization {
    document: Option<CanonicalComposeDocument>,
    diagnostics: Vec<Diagnostic>,
}

impl ComposeCanonicalization {
    /// Returns the canonical document when native validation succeeded.
    #[must_use]
    pub const fn document(&self) -> Option<&CanonicalComposeDocument> {
        self.document.as_ref()
    }

    /// Returns retained processing and rendering diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Consumes the result into its canonical document and diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<CanonicalComposeDocument>, Vec<Diagnostic>) {
        (self.document, self.diagnostics)
    }
}

/// Compose processing layer that emitted a retained native finding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ComposeFindingStage {
    /// YAML syntax or single-document loading.
    Load,
    /// Explicit variable interpolation.
    Interpolation,
    /// Ordered multi-document merge.
    Merge,
    /// Explicit service profile selection.
    ProfileSelection,
    /// Native merged-project typing.
    ProjectModel,
    /// Provider/runtime compatibility validation.
    Validation,
    /// Native Compose generation or parse-back validation.
    Rendering,
}

impl ComposeFindingStage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Interpolation => "interpolation",
            Self::Merge => "merge",
            Self::ProfileSelection => "profile-selection",
            Self::ProjectModel => "project-model",
            Self::Validation => "validation",
            Self::Rendering => "rendering",
        }
    }

    /// Converts one native diagnostic into the shared protected finding envelope.
    #[must_use]
    pub fn native_finding(self, diagnostic: &ComposeDiagnostic) -> NativeFinding {
        let severity = match diagnostic.severity() {
            ComposeSeverity::Error => Severity::Error,
            ComposeSeverity::Warning => Severity::Warning,
            ComposeSeverity::Note => Severity::Note,
        };
        let mut finding = NativeFinding::new(
            "compose",
            "compose-lens",
            diagnostic.code().as_str(),
            self.as_str(),
            severity,
            diagnostic.message(),
        );
        if let Some(variable) = interpolation_variable(diagnostic.code().as_str(), diagnostic.message()) {
            finding = finding.with_field(DiagnosticField::new("variable", DiagnosticValue::plain(variable)));
        }
        for label in diagnostic.labels() {
            finding = finding.with_label(NativeFindingLabel::new(
                match label.kind() {
                    LabelKind::Primary => NativeFindingLabelKind::Primary,
                    LabelKind::Secondary => NativeFindingLabelKind::Secondary,
                },
                label.span().source_id().get(),
                label.span().start(),
                label.span().end(),
                label.message(),
            ));
        }
        for note in diagnostic.notes() {
            finding = finding.with_note(note);
        }
        finding
    }
}

/// A merged Compose project and the caller-owned context needed to import it safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeSource {
    project: MergedProject,
    fallback_application_name: Identifier,
    source_ids: BTreeMap<ComposeSourceId, SourceId>,
    profile_selection: Option<ProfileSelection>,
    native_findings: Vec<NativeFinding>,
}

impl ComposeSource {
    /// Creates a source without guessing profiles or source filenames from ambient state.
    ///
    /// Every Compose source initially receives the stable neutral identity
    /// `compose-source-<numeric-id>`. Call [`Self::with_source_id`] to replace it with a caller-owned
    /// path, URI, or other display identity. The project's explicit top-level `name` wins when
    /// present; the fallback is used when Compose project naming was supplied externally or
    /// omitted.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] if a generated fallback source identity violates the neutral-model
    /// invariant. This cannot occur for the current `compose-source-<u32>` spelling, but keeping
    /// construction fallible avoids a hidden panic if that policy changes.
    pub fn new(project: MergedProject, fallback_application_name: Identifier) -> Result<Self, ModelError> {
        let source_ids = project
            .source_ids()
            .iter()
            .copied()
            .map(|source_id| {
                SourceId::new(format!("compose-source-{}", source_id.get())).map(|neutral| (source_id, neutral))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            project,
            fallback_application_name,
            source_ids,
            profile_selection: None,
            native_findings: Vec::new(),
        })
    }

    /// Assigns a caller-owned neutral identity to one Compose source document.
    ///
    /// Unknown Compose source IDs are retained in the map but cannot contribute provenance unless
    /// they also occur in the merged project.
    #[must_use]
    pub fn with_source_id(mut self, compose: ComposeSourceId, neutral: SourceId) -> Self {
        self.source_ids.insert(compose, neutral);
        self
    }

    /// Attaches the explicit selection produced by `ComposeLens` project processing.
    #[must_use]
    pub fn with_profile_selection(mut self, selection: ProfileSelection) -> Self {
        self.profile_selection = Some(selection);
        self
    }

    /// Retains native Compose diagnostics without terminal rendering or source contents.
    ///
    /// Diagnostics should be attached at the stage where the caller obtained them. The adapter
    /// keeps native codes as provenance and maps them to BoxFerry-owned rules during import.
    #[must_use]
    pub fn with_native_diagnostics<'a>(
        mut self,
        stage: ComposeFindingStage,
        diagnostics: impl IntoIterator<Item = &'a ComposeDiagnostic>,
    ) -> Self {
        self.native_findings.extend(
            diagnostics
                .into_iter()
                .map(|diagnostic| stage.native_finding(diagnostic)),
        );
        self
    }

    /// Returns the merged native project.
    #[must_use]
    pub const fn project(&self) -> &MergedProject {
        &self.project
    }

    /// Returns the caller-selected fallback application name.
    #[must_use]
    pub const fn fallback_application_name(&self) -> &Identifier {
        &self.fallback_application_name
    }

    /// Resolves a Compose source identity into its neutral-model identity.
    #[must_use]
    pub fn source_id(&self, compose: ComposeSourceId) -> Option<&SourceId> {
        self.source_ids.get(&compose)
    }

    /// Returns the explicit profile selection, when one was supplied.
    #[must_use]
    pub const fn profile_selection(&self) -> Option<&ProfileSelection> {
        self.profile_selection.as_ref()
    }

    /// Returns retained native Compose findings in processing order.
    #[must_use]
    pub fn native_findings(&self) -> &[NativeFinding] {
        &self.native_findings
    }

    /// Canonically renders this processed Compose project without evaluating unresolved variables.
    ///
    /// This same-format boundary delegates syntax generation to `ComposeLens`. Unlike export from
    /// the neutral application model, it can retain valid source-native values that have no
    /// format-independent representation, including interpolation expressions with defaults.
    /// Loader, interpolation, merge, profile, and rendering findings remain structured.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] if `BoxFerry`'s static diagnostic catalogue contains an
    /// invalid code.
    pub fn canonicalize(&self) -> Result<ComposeCanonicalization, InvalidDiagnosticCode> {
        let rendered = render_canonical(self.project(), self.profile_selection());
        let mut findings = self.native_findings.clone();
        findings.extend(
            rendered
                .diagnostics()
                .iter()
                .map(|diagnostic| ComposeFindingStage::Rendering.native_finding(diagnostic)),
        );
        let diagnostics = findings
            .into_iter()
            .map(compose_native_diagnostic)
            .collect::<Result<Vec<_>, _>>()?;
        let valid = rendered.is_valid()
            && diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity() != Severity::Error);
        let sensitive = rendered.is_sensitive();
        let document = valid.then(|| CanonicalComposeDocument {
            text: rendered.into_output(),
            sensitive,
        });
        Ok(ComposeCanonicalization { document, diagnostics })
    }
}

pub(crate) fn native_project_finding(diagnostic: &ComposeDiagnostic) -> NativeFinding {
    ComposeFindingStage::ProjectModel.native_finding(diagnostic)
}

fn compose_native_diagnostic(finding: NativeFinding) -> Result<Diagnostic, InvalidDiagnosticCode> {
    let rule = match finding.code() {
        "compose.interpolation.unset-variable" => RuleId::ComposeUnsetVariable,
        "compose.interpolation.required-variable" => RuleId::ComposeRequiredVariable,
        "compose.interpolation.invalid-expression" => RuleId::ComposeInterpolationInvalid,
        "compose.interpolation.nesting-limit" => RuleId::ComposeInterpolationNestingLimit,
        _ => match finding.severity() {
            Severity::Error => RuleId::ComposeNativeError,
            Severity::Note => RuleId::ComposeNativeNote,
            _ => RuleId::ComposeNativeWarning,
        },
    };
    Ok(Diagnostic::new(
        rule.definition().diagnostic_code()?,
        finding.severity(),
        "ComposeLens reported a native Compose finding",
    )
    .with_native_finding(finding))
}

fn interpolation_variable<'a>(code: &str, message: &'a str) -> Option<&'a str> {
    if !matches!(
        code,
        "compose.interpolation.unset-variable" | "compose.interpolation.required-variable"
    ) {
        return None;
    }
    let (_, remainder) = message.split_once('`')?;
    let (variable, _) = remainder.split_once('`')?;
    let mut bytes = variable.bytes();
    let first = bytes.next()?;
    ((first == b'_' || first.is_ascii_alphabetic()) && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric()))
        .then_some(variable)
}
