//! Explicit input bundle for one native Quadlet document set.

use std::{collections::BTreeMap, error::Error, fmt};

use boxferry_engine::{NativeFinding, NativeFindingLabel, NativeFindingLabelKind, Severity};
use boxferry_model::{Identifier, ModelError, SourceId};
use quadlet_lens::{
    diagnostic::{Diagnostic as NativeDiagnostic, Severity as NativeSeverity},
    model::{
        DocumentSetError, NamedQuadletDocument, QuadletDocument, QuadletDocumentSet, TypedModelError, UnitFileName,
    },
    source::{SourceId as QuadletSourceId, SourceSpan as QuadletSourceSpan},
};

/// Origin of one native diagnostic retained by [`QuadletSource::parse`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletParseDiagnosticOrigin {
    /// `QuadletLens` syntax parsing.
    Syntax,
    /// `QuadletLens` typed-model validation.
    Model,
    /// `QuadletLens` cross-document validation.
    DocumentSet,
}

/// Severity of one native diagnostic retained by [`QuadletSource::parse`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletParseDiagnosticSeverity {
    /// The native input cannot cross the parse boundary.
    Error,
    /// The native input remains usable but requires attention.
    Warning,
    /// Informational native context.
    Note,
}

/// One labelled byte range on a [`QuadletParseDiagnostic`].
///
/// This DTO deliberately retains only the numeric source identity, half-open byte offsets, and
/// static native label message. It never retains source text or a source name.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletParseDiagnosticLabel {
    source_id: u32,
    start: usize,
    end: usize,
    message: &'static str,
}

impl QuadletParseDiagnosticLabel {
    /// Returns the caller-owned numeric source identity.
    #[must_use]
    pub const fn source_id(&self) -> u32 {
        self.source_id
    }

    /// Returns the inclusive byte offset of this label.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// Returns the exclusive byte offset of this label.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }

    /// Returns the static, value-free native label message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

/// One recoverable `QuadletLens` diagnostic retained without source contents.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletParseDiagnostic {
    code: &'static str,
    severity: QuadletParseDiagnosticSeverity,
    summary: &'static str,
    origin: QuadletParseDiagnosticOrigin,
    labels: Vec<QuadletParseDiagnosticLabel>,
}

impl QuadletParseDiagnostic {
    /// Returns the stable native diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the native diagnostic severity.
    #[must_use]
    pub const fn severity(&self) -> QuadletParseDiagnosticSeverity {
        self.severity
    }

    /// Returns the static, value-free native diagnostic summary.
    #[must_use]
    pub const fn summary(&self) -> &'static str {
        self.summary
    }

    /// Returns the native diagnostic layer that produced this diagnostic.
    #[must_use]
    pub const fn origin(&self) -> QuadletParseDiagnosticOrigin {
        self.origin
    }

    /// Returns every labelled native source span in native order.
    #[must_use]
    pub fn labels(&self) -> &[QuadletParseDiagnosticLabel] {
        &self.labels
    }

    /// Converts this diagnostic into the shared protected finding envelope.
    #[must_use]
    pub fn native_finding(&self) -> NativeFinding {
        native_finding(self)
    }
}

/// Stage that produced a non-recoverable parse failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletParseFailureStage {
    /// Native unit filename validation.
    Filename,
    /// Native typed-model construction.
    TypedModel,
    /// Native named-document metadata construction.
    DocumentMetadata,
    /// Native document-set construction.
    DocumentSet,
    /// `BoxFerry` neutral source construction.
    NeutralModel,
}

/// A non-recoverable failure that prevented a parse result.
///
/// The failure retains structured stage and location metadata only. It never stores a filename,
/// source text, native display rendering, or model value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletParseFailure {
    stage: QuadletParseFailureStage,
    input_index: Option<usize>,
    source_id: Option<u32>,
    span: Option<QuadletParseDiagnosticLabel>,
    summary: &'static str,
}

impl QuadletParseFailure {
    /// Returns the stage that could not recover.
    #[must_use]
    pub const fn stage(&self) -> QuadletParseFailureStage {
        self.stage
    }

    /// Returns the caller-order index of the input when known.
    #[must_use]
    pub const fn input_index(&self) -> Option<usize> {
        self.input_index
    }

    /// Returns the caller-owned numeric source identity when known.
    #[must_use]
    pub const fn source_id(&self) -> Option<u32> {
        self.source_id
    }

    /// Returns the parser-owned span when the failure has one.
    #[must_use]
    pub const fn span(&self) -> Option<&QuadletParseDiagnosticLabel> {
        self.span.as_ref()
    }

    /// Returns a static, value-free failure summary.
    #[must_use]
    pub const fn summary(&self) -> &'static str {
        self.summary
    }
}

/// Successful native parse with every recoverable diagnostic retained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletParseResult {
    source: QuadletSource,
    diagnostics: Vec<QuadletParseDiagnostic>,
}

impl QuadletParseResult {
    /// Returns the native source boundary, including a recoverably incomplete document graph.
    #[must_use]
    pub const fn source(&self) -> &QuadletSource {
        &self.source
    }

    /// Returns native syntax, model, and document-set diagnostics in collection order.
    #[must_use]
    pub fn diagnostics(&self) -> &[QuadletParseDiagnostic] {
        &self.diagnostics
    }

    /// Consumes this result and returns the valid source boundary.
    #[must_use]
    pub fn into_source(self) -> QuadletSource {
        self.source
    }
}

/// Quadlet parse failure with recoverable diagnostics retained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletParseError {
    diagnostics: Vec<QuadletParseDiagnostic>,
    failures: Vec<QuadletParseFailure>,
}

impl QuadletParseError {
    /// Returns every recoverable native diagnostic collected before the boundary failed.
    #[must_use]
    pub fn diagnostics(&self) -> &[QuadletParseDiagnostic] {
        &self.diagnostics
    }

    /// Returns non-recoverable native or neutral-model failures in collection order.
    #[must_use]
    pub fn failures(&self) -> &[QuadletParseFailure] {
        &self.failures
    }
}

impl fmt::Display for QuadletParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Quadlet parse failed")
    }
}

impl Error for QuadletParseError {}

/// One caller-supplied Quadlet unit file held entirely in memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuadletDocumentInput {
    name: String,
    source_id: QuadletSourceId,
    text: String,
}

impl QuadletDocumentInput {
    /// Creates an input without reading or validating an external path.
    #[must_use]
    pub fn new(name: impl Into<String>, source_id: QuadletSourceId, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source_id,
            text: text.into(),
        }
    }

    /// Returns the caller-supplied unit-file basename.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the caller-owned native source identity.
    #[must_use]
    pub const fn source_id(&self) -> QuadletSourceId {
        self.source_id
    }

    /// Returns the complete authored unit text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A named Quadlet document set and caller-owned source identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuadletSource {
    application_name: Identifier,
    documents: QuadletDocumentSet,
    source_ids: BTreeMap<QuadletSourceId, SourceId>,
    fallback_source_id: SourceId,
    native_findings: Vec<NativeFinding>,
}

impl QuadletSource {
    /// Parses explicit in-memory unit files while retaining every recoverable native diagnostic.
    ///
    /// Diagnostics are collected in caller input order: syntax diagnostics followed by typed-model
    /// diagnostics for each input, then document-set diagnostics. Recoverable document-set
    /// diagnostics retain an incomplete graph in the successful result. Syntax/model errors and
    /// non-recoverable metadata failures return [`QuadletParseError`].
    ///
    /// The parse DTOs contain static diagnostic messages, numeric source identities, and byte
    /// spans only; source text, filenames, protected values, and native terminal renderings are
    /// deliberately excluded.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletParseError`] when a syntax/model error-severity diagnostic or a
    /// non-recoverable native or neutral-model construction failure prevents a usable source.
    pub fn parse(
        application_name: Identifier,
        inputs: impl IntoIterator<Item = QuadletDocumentInput>,
    ) -> Result<QuadletParseResult, QuadletParseError> {
        let mut documents = Vec::new();
        let mut source_names = Vec::new();
        let mut diagnostics = Vec::new();
        let mut failures = Vec::new();

        for (input_index, input) in inputs.into_iter().enumerate() {
            let source_id = input.source_id;
            let filename = match UnitFileName::new(input.name.clone()) {
                Ok(filename) => filename,
                Err(error) => {
                    failures.push(document_metadata_failure(
                        QuadletParseFailureStage::Filename,
                        Some(input_index),
                        Some(source_id.get()),
                        None,
                        &error,
                    ));
                    continue;
                }
            };
            let parsed = match QuadletDocument::parse(filename.unit_type(), source_id, input.text) {
                Ok(parsed) => parsed,
                Err(error) => {
                    failures.push(typed_model_failure(input_index, source_id, error));
                    continue;
                }
            };
            diagnostics.extend(copy_diagnostics(
                QuadletParseDiagnosticOrigin::Syntax,
                parsed.syntax().diagnostics(),
            ));
            diagnostics.extend(copy_diagnostics(
                QuadletParseDiagnosticOrigin::Model,
                parsed.model_diagnostics(),
            ));
            match NamedQuadletDocument::new(filename.as_str(), parsed.document().clone()) {
                Ok(document) => {
                    source_names.push((source_id, filename.as_str().to_owned()));
                    documents.push(document);
                }
                Err(error) => failures.push(document_metadata_failure(
                    QuadletParseFailureStage::DocumentMetadata,
                    Some(input_index),
                    Some(source_id.get()),
                    None,
                    &error,
                )),
            }
        }

        let documents = match QuadletDocumentSet::new(documents) {
            Ok(documents) => {
                diagnostics.extend(copy_diagnostics(
                    QuadletParseDiagnosticOrigin::DocumentSet,
                    documents.diagnostics(),
                ));
                Some(documents)
            }
            Err(error) => {
                failures.push(document_set_failure(&error));
                None
            }
        };

        if diagnostics.iter().any(|diagnostic| {
            diagnostic.severity() == QuadletParseDiagnosticSeverity::Error
                && !matches!(diagnostic.origin(), QuadletParseDiagnosticOrigin::DocumentSet)
        }) || !failures.is_empty()
        {
            return Err(QuadletParseError { diagnostics, failures });
        }

        let Some(documents) = documents else {
            return Err(QuadletParseError {
                diagnostics,
                failures: vec![QuadletParseFailure {
                    stage: QuadletParseFailureStage::DocumentSet,
                    input_index: None,
                    source_id: None,
                    span: None,
                    summary: "Quadlet document set could not be constructed",
                }],
            });
        };
        let mut source = match Self::from_validated_documents(application_name, documents) {
            Ok(source) => source,
            Err(error) => {
                failures.push(neutral_model_failure(error));
                return Err(QuadletParseError { diagnostics, failures });
            }
        };
        for (native, name) in source_names {
            let neutral = match SourceId::new(name) {
                Ok(neutral) => neutral,
                Err(error) => {
                    failures.push(neutral_model_failure(error));
                    return Err(QuadletParseError { diagnostics, failures });
                }
            };
            source = source.with_source_id(native, neutral);
        }
        source.native_findings = diagnostics.iter().map(native_finding).collect();
        Ok(QuadletParseResult { source, diagnostics })
    }

    /// Creates an explicit source without reading a Quadlet search path or installed units.
    ///
    /// Every document initially receives the stable neutral identity
    /// `quadlet-source-<numeric-id>`. Call [`Self::with_source_id`] to replace it with a
    /// caller-owned path, URI, or other display identity.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] if a generated source identity violates the neutral-model
    /// invariant. The current generated spelling is always valid.
    pub fn from_validated_documents(
        application_name: Identifier,
        documents: QuadletDocumentSet,
    ) -> Result<Self, ModelError> {
        let fallback_source_id = SourceId::new("quadlet-source")?;
        let native_findings = copy_diagnostics(QuadletParseDiagnosticOrigin::DocumentSet, documents.diagnostics())
            .iter()
            .map(native_finding)
            .collect();
        let source_ids = documents
            .documents()
            .iter()
            .map(|document| document.document().source_id())
            .map(|source_id| {
                SourceId::new(format!("quadlet-source-{}", source_id.get())).map(|neutral| (source_id, neutral))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            application_name,
            documents,
            source_ids,
            fallback_source_id,
            native_findings,
        })
    }

    /// Assigns a caller-owned neutral identity to one Quadlet source document.
    #[must_use]
    pub fn with_source_id(mut self, quadlet: QuadletSourceId, neutral: SourceId) -> Self {
        self.source_ids.insert(quadlet, neutral);
        self
    }

    /// Returns the caller-selected neutral application name.
    #[must_use]
    pub const fn application_name(&self) -> &Identifier {
        &self.application_name
    }

    /// Returns the complete named native document set.
    #[must_use]
    pub const fn documents(&self) -> &QuadletDocumentSet {
        &self.documents
    }

    /// Resolves a Quadlet source identity into its neutral-model identity.
    #[must_use]
    pub fn source_id(&self, quadlet: QuadletSourceId) -> &SourceId {
        self.source_ids.get(&quadlet).unwrap_or(&self.fallback_source_id)
    }

    /// Returns every recoverable native finding retained by the source boundary.
    ///
    /// The findings contain stable native codes and value-free source metadata, but never source
    /// contents or filenames. Importing this source forwards the same findings to embedded callers.
    #[must_use]
    pub fn native_findings(&self) -> &[NativeFinding] {
        &self.native_findings
    }
}

fn native_finding(diagnostic: &QuadletParseDiagnostic) -> NativeFinding {
    let mut finding = NativeFinding::new(
        "quadlet",
        "quadlet-lens",
        diagnostic.code(),
        match diagnostic.origin() {
            QuadletParseDiagnosticOrigin::Syntax => "syntax",
            QuadletParseDiagnosticOrigin::Model => "model",
            QuadletParseDiagnosticOrigin::DocumentSet => "document-set",
        },
        match diagnostic.severity() {
            QuadletParseDiagnosticSeverity::Error => Severity::Error,
            QuadletParseDiagnosticSeverity::Warning => Severity::Warning,
            QuadletParseDiagnosticSeverity::Note => Severity::Note,
        },
        diagnostic.summary(),
    );
    for label in diagnostic.labels() {
        finding = finding.with_label(NativeFindingLabel::new(
            NativeFindingLabelKind::Primary,
            label.source_id(),
            label.start(),
            label.end(),
            label.message(),
        ));
    }
    finding
}

fn copy_diagnostics(
    origin: QuadletParseDiagnosticOrigin,
    diagnostics: &[NativeDiagnostic],
) -> Vec<QuadletParseDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| QuadletParseDiagnostic {
            code: diagnostic.code().as_str(),
            severity: match diagnostic.severity() {
                NativeSeverity::Error => QuadletParseDiagnosticSeverity::Error,
                NativeSeverity::Warning => QuadletParseDiagnosticSeverity::Warning,
                NativeSeverity::Note => QuadletParseDiagnosticSeverity::Note,
            },
            summary: diagnostic.summary(),
            origin,
            labels: diagnostic.labels().iter().map(diagnostic_label).collect(),
        })
        .collect()
}

fn diagnostic_label(label: &quadlet_lens::diagnostic::Label) -> QuadletParseDiagnosticLabel {
    span_label(label.span(), label.message())
}

fn span_label(span: QuadletSourceSpan, message: &'static str) -> QuadletParseDiagnosticLabel {
    QuadletParseDiagnosticLabel {
        source_id: span.source_id().get(),
        start: span.start(),
        end: span.end(),
        message,
    }
}

fn typed_model_failure(input_index: usize, source_id: QuadletSourceId, error: TypedModelError) -> QuadletParseFailure {
    match error {
        TypedModelError::InvalidSourceSpan(span) => QuadletParseFailure {
            stage: QuadletParseFailureStage::TypedModel,
            input_index: Some(input_index),
            source_id: Some(source_id.get()),
            span: Some(span_label(span, "native typed-model span could not be resolved")),
            summary: "Quadlet typed-model construction failed",
        },
        _ => QuadletParseFailure {
            stage: QuadletParseFailureStage::TypedModel,
            input_index: Some(input_index),
            source_id: Some(source_id.get()),
            span: None,
            summary: "Quadlet typed-model construction failed",
        },
    }
}

fn document_metadata_failure(
    stage: QuadletParseFailureStage,
    input_index: Option<usize>,
    source_id: Option<u32>,
    span: Option<QuadletParseDiagnosticLabel>,
    error: &DocumentSetError,
) -> QuadletParseFailure {
    let summary = match error {
        DocumentSetError::InvalidUnitFileName(_) => "Quadlet unit filename is invalid",
        DocumentSetError::UnsupportedUnitFileExtension(_) => "Quadlet unit filename has an unsupported extension",
        DocumentSetError::UnitTypeMismatch { .. } => "Quadlet unit filename and typed document kind differ",
        DocumentSetError::DuplicateSourceId(_) => "Quadlet named document has a duplicate source identity",
        _ => "Quadlet native document metadata could not be constructed",
    };
    QuadletParseFailure {
        stage,
        input_index,
        source_id,
        span,
        summary,
    }
}

fn document_set_failure(error: &DocumentSetError) -> QuadletParseFailure {
    let source_id = match error {
        DocumentSetError::DuplicateSourceId(source_id) => Some(source_id.get()),
        _ => None,
    };
    let summary = match error {
        DocumentSetError::InvalidUnitFileName(_) => "Quadlet document set contains an invalid unit filename",
        DocumentSetError::UnsupportedUnitFileExtension(_) => {
            "Quadlet document set contains an unsupported unit extension"
        }
        DocumentSetError::UnitTypeMismatch { .. } => "Quadlet document set has mismatched document metadata",
        DocumentSetError::DuplicateSourceId(_) => "Quadlet document set has duplicate source identities",
        _ => "Quadlet document set could not be constructed",
    };
    QuadletParseFailure {
        stage: QuadletParseFailureStage::DocumentSet,
        input_index: None,
        source_id,
        span: None,
        summary,
    }
}

fn neutral_model_failure(_error: ModelError) -> QuadletParseFailure {
    QuadletParseFailure {
        stage: QuadletParseFailureStage::NeutralModel,
        input_index: None,
        source_id: None,
        span: None,
        summary: "BoxFerry neutral Quadlet source construction failed",
    }
}
