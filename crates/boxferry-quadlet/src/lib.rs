//! Maps between native Quadlet documents and `BoxFerry`'s neutral application model.

mod export;
mod import;
mod output;
mod source;

pub use export::{QuadletExporter, QuadletExporterError, QuadletGroupingPolicy};
pub use import::QuadletImporter;
pub use output::{QuadletFile, QuadletOutput};
pub use source::{QuadletDocumentInput, QuadletSource, QuadletSourceError};

/// The native Quadlet library consumed by this adapter.
pub use quadlet_lens;
