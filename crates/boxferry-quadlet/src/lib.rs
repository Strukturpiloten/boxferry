//! Maps `BoxFerry`'s neutral application model into validated native Quadlet documents.

mod export;
mod output;

pub use export::{QuadletExporter, QuadletExporterError, QuadletGroupingPolicy};
pub use output::{QuadletFile, QuadletOutput};

/// The native Quadlet library consumed by this adapter.
pub use quadlet_lens;
