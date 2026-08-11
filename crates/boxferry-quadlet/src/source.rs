//! Explicit input bundle for one native Quadlet document set.

use std::{collections::BTreeMap, error::Error, fmt};

use boxferry_model::{Identifier, ModelError, SourceId};
use quadlet_lens::{
    diagnostic::{Diagnostic as NativeDiagnostic, Severity as NativeSeverity},
    model::{
        DocumentSetError, NamedQuadletDocument, QuadletDocument, QuadletDocumentSet, TypedModelError, UnitFileName,
    },
    source::{SourceId as QuadletSourceId, SourceSpan as QuadletSourceSpan},
};

/// Origin of one native diagnostic retained by [`QuadletSource::parse_detailed`].
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

/// Severity of one native diagnostic retained by [`QuadletSource::parse_detailed`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletParseDiagnosticSeverity {
    /// The native input cannot cross the detailed parse boundary.
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
}

/// Stage that produced a non-recoverable detailed parse failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletDetailedParseFailureStage {
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

/// A non-recoverable failure that prevented a detailed parse result.
///
/// The failure retains structured stage and location metadata only. It never stores a filename,
/// source text, native display rendering, or model value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletDetailedParseFailure {
    stage: QuadletDetailedParseFailureStage,
    input_index: Option<usize>,
    source_id: Option<u32>,
    span: Option<QuadletParseDiagnosticLabel>,
    summary: &'static str,
}

impl QuadletDetailedParseFailure {
    /// Returns the stage that could not recover.
    #[must_use]
    pub const fn stage(&self) -> QuadletDetailedParseFailureStage {
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

/// Successful detailed native parse with every non-error diagnostic retained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletDetailedParseResult {
    source: QuadletSource,
    diagnostics: Vec<QuadletParseDiagnostic>,
}

impl QuadletDetailedParseResult {
    /// Returns the fully valid source boundary.
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

/// Detailed Quadlet parse failure with recoverable diagnostics retained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QuadletDetailedParseError {
    diagnostics: Vec<QuadletParseDiagnostic>,
    failures: Vec<QuadletDetailedParseFailure>,
}

impl QuadletDetailedParseError {
    /// Returns every recoverable native diagnostic collected before the boundary failed.
    #[must_use]
    pub fn diagnostics(&self) -> &[QuadletParseDiagnostic] {
        &self.diagnostics
    }

    /// Returns non-recoverable native or neutral-model failures in collection order.
    #[must_use]
    pub fn failures(&self) -> &[QuadletDetailedParseFailure] {
        &self.failures
    }
}

impl fmt::Display for QuadletDetailedParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("detailed Quadlet parse failed")
    }
}

impl Error for QuadletDetailedParseError {}

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
}

impl QuadletSource {
    /// Parses explicit in-memory unit files and retains only a fully valid native typed boundary.
    ///
    /// Unit basenames become the default neutral source identities. The resulting document set
    /// may still contain unresolved or ambiguous cross-unit references; the importer reports
    /// those as structured conversion diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletSourceError`] for an invalid filename, a typed-model construction failure,
    /// syntax or model diagnostics with error severity, invalid document-set metadata, or a
    /// neutral source identity that violates model invariants.
    pub fn parse(
        application_name: Identifier,
        inputs: impl IntoIterator<Item = QuadletDocumentInput>,
    ) -> Result<Self, QuadletSourceError> {
        let mut documents = Vec::new();
        let mut source_names = Vec::new();
        for input in inputs {
            let filename = UnitFileName::new(input.name.clone())
                .map_err(|error| QuadletSourceError::native(input.name.clone(), error))?;
            let parsed = QuadletDocument::parse(filename.unit_type(), input.source_id, input.text)
                .map_err(|error| QuadletSourceError::native(input.name.clone(), error))?;
            if !parsed.is_valid() {
                return Err(QuadletSourceError::InvalidDocument {
                    name: input.name,
                    syntax_diagnostics: parsed.syntax().diagnostics().len(),
                    model_diagnostics: parsed.model_diagnostics().len(),
                });
            }
            source_names.push((input.source_id, filename.as_str().to_owned()));
            documents.push(
                NamedQuadletDocument::new(filename.as_str(), parsed.document().clone())
                    .map_err(|error| QuadletSourceError::native(filename.as_str(), error))?,
            );
        }
        let documents =
            QuadletDocumentSet::new(documents).map_err(|error| QuadletSourceError::native("document set", error))?;
        let mut source = Self::from_validated_documents(application_name, documents)?;
        for (native, name) in source_names {
            source = source.with_source_id(native, SourceId::new(name)?);
        }
        Ok(source)
    }

    /// Parses explicit in-memory unit files while retaining every recoverable native diagnostic.
    ///
    /// Diagnostics are collected in caller input order: syntax diagnostics followed by typed-model
    /// diagnostics for each input, then document-set diagnostics. Error-severity native
    /// diagnostics and non-recoverable metadata failures return
    /// [`QuadletDetailedParseError`] without exposing a usable [`QuadletSource`]. The legacy
    /// [`Self::parse`] boundary remains available when only its aggregate compatibility summary is
    /// needed.
    ///
    /// The detailed DTOs contain static diagnostic messages, numeric source identities, and byte
    /// spans only; source text, filenames, protected values, and native terminal renderings are
    /// deliberately excluded.
    ///
    /// # Errors
    ///
    /// Returns [`QuadletDetailedParseError`] when a native error-severity diagnostic or a
    /// non-recoverable native or neutral-model construction failure prevents a usable source.
    pub fn parse_detailed(
        application_name: Identifier,
        inputs: impl IntoIterator<Item = QuadletDocumentInput>,
    ) -> Result<QuadletDetailedParseResult, QuadletDetailedParseError> {
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
                        QuadletDetailedParseFailureStage::Filename,
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
                    QuadletDetailedParseFailureStage::DocumentMetadata,
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

        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == QuadletParseDiagnosticSeverity::Error)
            || !failures.is_empty()
        {
            return Err(QuadletDetailedParseError { diagnostics, failures });
        }

        let Some(documents) = documents else {
            return Err(QuadletDetailedParseError {
                diagnostics,
                failures: vec![QuadletDetailedParseFailure {
                    stage: QuadletDetailedParseFailureStage::DocumentSet,
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
                return Err(QuadletDetailedParseError { diagnostics, failures });
            }
        };
        for (native, name) in source_names {
            let neutral = match SourceId::new(name) {
                Ok(neutral) => neutral,
                Err(error) => {
                    failures.push(neutral_model_failure(error));
                    return Err(QuadletDetailedParseError { diagnostics, failures });
                }
            };
            source = source.with_source_id(native, neutral);
        }
        Ok(QuadletDetailedParseResult { source, diagnostics })
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

fn typed_model_failure(
    input_index: usize,
    source_id: QuadletSourceId,
    error: TypedModelError,
) -> QuadletDetailedParseFailure {
    match error {
        TypedModelError::InvalidSourceSpan(span) => QuadletDetailedParseFailure {
            stage: QuadletDetailedParseFailureStage::TypedModel,
            input_index: Some(input_index),
            source_id: Some(source_id.get()),
            span: Some(span_label(span, "native typed-model span could not be resolved")),
            summary: "Quadlet typed-model construction failed",
        },
        _ => QuadletDetailedParseFailure {
            stage: QuadletDetailedParseFailureStage::TypedModel,
            input_index: Some(input_index),
            source_id: Some(source_id.get()),
            span: None,
            summary: "Quadlet typed-model construction failed",
        },
    }
}

fn document_metadata_failure(
    stage: QuadletDetailedParseFailureStage,
    input_index: Option<usize>,
    source_id: Option<u32>,
    span: Option<QuadletParseDiagnosticLabel>,
    error: &DocumentSetError,
) -> QuadletDetailedParseFailure {
    let summary = match error {
        DocumentSetError::InvalidUnitFileName(_) => "Quadlet unit filename is invalid",
        DocumentSetError::UnsupportedUnitFileExtension(_) => "Quadlet unit filename has an unsupported extension",
        DocumentSetError::UnitTypeMismatch { .. } => "Quadlet unit filename and typed document kind differ",
        DocumentSetError::DuplicateSourceId(_) => "Quadlet named document has a duplicate source identity",
        _ => "Quadlet native document metadata could not be constructed",
    };
    QuadletDetailedParseFailure {
        stage,
        input_index,
        source_id,
        span,
        summary,
    }
}

fn document_set_failure(error: &DocumentSetError) -> QuadletDetailedParseFailure {
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
    QuadletDetailedParseFailure {
        stage: QuadletDetailedParseFailureStage::DocumentSet,
        input_index: None,
        source_id,
        span: None,
        summary,
    }
}

fn neutral_model_failure(_error: ModelError) -> QuadletDetailedParseFailure {
    QuadletDetailedParseFailure {
        stage: QuadletDetailedParseFailureStage::NeutralModel,
        input_index: None,
        source_id: None,
        span: None,
        summary: "BoxFerry neutral Quadlet source construction failed",
    }
}

/// Failure while creating a native Quadlet source boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QuadletSourceError {
    /// `QuadletLens` rejected native filename, syntax, typed-model, or document-set metadata.
    Native {
        /// Unit filename or source stage.
        name: String,
        /// Redaction-safe native error text.
        reason: String,
    },
    /// A parsed unit contains syntax or model errors and cannot be silently imported.
    InvalidDocument {
        /// Unit-file basename.
        name: String,
        /// Number of syntax diagnostics retained by `QuadletLens`.
        syntax_diagnostics: usize,
        /// Number of typed-model diagnostics retained by `QuadletLens`.
        model_diagnostics: usize,
    },
    /// A neutral source identity violated `BoxFerry` model invariants.
    Model(ModelError),
}

impl QuadletSourceError {
    fn native(name: impl Into<String>, error: impl fmt::Display) -> Self {
        Self::Native {
            name: name.into(),
            reason: error.to_string(),
        }
    }
}

impl fmt::Display for QuadletSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native { name, reason } => write!(formatter, "invalid Quadlet input `{name}`: {reason}"),
            Self::InvalidDocument {
                name,
                syntax_diagnostics,
                model_diagnostics,
            } => write!(
                formatter,
                "Quadlet input `{name}` is invalid ({syntax_diagnostics} syntax diagnostic(s), {model_diagnostics} model diagnostic(s))"
            ),
            Self::Model(error) => write!(formatter, "invalid neutral Quadlet source identity: {error}"),
        }
    }
}

impl Error for QuadletSourceError {}

impl From<ModelError> for QuadletSourceError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}
