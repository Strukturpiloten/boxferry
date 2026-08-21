//! Explicit input bundle for one processed Compose project.

use std::collections::BTreeMap;

use boxferry_engine::{
    DiagnosticField, DiagnosticValue, NativeFinding, NativeFindingLabel, NativeFindingLabelKind, Severity,
};
use boxferry_model::{Identifier, ModelError, SourceId};
use compose_lens::{
    diagnostic::{Diagnostic as ComposeDiagnostic, LabelKind, Severity as ComposeSeverity},
    merge::MergedProject,
    profiles::ProfileSelection,
    source::SourceId as ComposeSourceId,
};

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
}

pub(crate) fn native_project_finding(diagnostic: &ComposeDiagnostic) -> NativeFinding {
    ComposeFindingStage::ProjectModel.native_finding(diagnostic)
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
