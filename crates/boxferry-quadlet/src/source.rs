//! Explicit input bundle for one native Quadlet document set.

use std::{collections::BTreeMap, error::Error, fmt};

use boxferry_model::{Identifier, ModelError, SourceId};
use quadlet_lens::{
    model::{NamedQuadletDocument, QuadletDocument, QuadletDocumentSet, UnitFileName},
    source::SourceId as QuadletSourceId,
};

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
