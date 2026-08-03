//! Validated native output file set.

use std::fmt;

use quadlet_lens::{
    model::{DocumentSetError, NamedQuadletDocument, QuadletDocumentSet, UnitFileName},
    render::GeneratedQuadletDocument,
};

/// One generated Quadlet file whose contents passed `QuadletLens` parse-back validation.
#[derive(Clone, Eq, PartialEq)]
pub struct QuadletFile {
    name: UnitFileName,
    text: String,
}

impl QuadletFile {
    /// Returns the validated unit-file basename.
    #[must_use]
    pub const fn name(&self) -> &UnitFileName {
        &self.name
    }

    /// Explicitly exposes the generated file contents.
    ///
    /// Generated files may contain environment values and other sensitive application data.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for QuadletFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuadletFile")
            .field("name", &self.name)
            .field("text", &"[REDACTED]")
            .field("bytes", &self.text.len())
            .finish()
    }
}

/// Deterministically ordered generated files and their validated dependency graph.
#[derive(Clone, Eq, PartialEq)]
pub struct QuadletOutput {
    files: Vec<QuadletFile>,
    documents: QuadletDocumentSet,
}

impl QuadletOutput {
    pub(crate) fn from_generated(generated: Vec<(String, GeneratedQuadletDocument)>) -> Result<Self, DocumentSetError> {
        let mut files = Vec::with_capacity(generated.len());
        let mut documents = Vec::with_capacity(generated.len());
        for (name, generated) in generated {
            let file_name = UnitFileName::new(name.clone())?;
            documents.push(NamedQuadletDocument::new(name, generated.document().clone())?);
            files.push(QuadletFile {
                name: file_name,
                text: generated.text().to_owned(),
            });
        }
        let documents = QuadletDocumentSet::new(documents)?;
        Ok(Self { files, documents })
    }

    /// Returns generated files in deterministic network, volume, optional pod, then service order.
    #[must_use]
    pub fn files(&self) -> &[QuadletFile] {
        &self.files
    }

    /// Returns a generated file by exact basename.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&QuadletFile> {
        self.files.iter().find(|file| file.name().as_str() == name)
    }

    /// Returns the parse-back-validated native document set and dependency graph.
    #[must_use]
    pub const fn document_set(&self) -> &QuadletDocumentSet {
        &self.documents
    }
}

impl fmt::Debug for QuadletOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.files.iter().map(|file| file.name().as_str()).collect();
        formatter
            .debug_struct("QuadletOutput")
            .field("files", &names)
            .field("complete_graph", &self.documents.graph().is_complete())
            .finish()
    }
}
