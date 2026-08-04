//! Source provenance attached to neutral-model values.

use crate::ModelError;

/// Caller-selected identity for one imported source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(String);

impl SourceId {
    /// Creates a non-empty source identity.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::EmptyValue`] for an empty identity and
    /// [`ModelError::ContainsNul`] for text containing a NUL byte.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        validate_text("source identity", &value)?;
        Ok(Self(value))
    }

    /// Returns the authored identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Half-open byte range in one imported source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceSpan {
    start: usize,
    end: usize,
}

impl SourceSpan {
    /// Creates a byte range whose end is not before its start.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError::ReversedSpan`] when `end < start`.
    pub const fn new(start: usize, end: usize) -> Result<Self, ModelError> {
        if end < start {
            return Err(ModelError::ReversedSpan { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive start offset.
    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the exclusive end offset.
    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }
}

/// Location from which a neutral-model value was derived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    kind: ProvenanceKind,
    source: SourceId,
    span: Option<SourceSpan>,
}

/// How a neutral-model value entered a conversion plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProvenanceKind {
    /// Authored source document or native definition.
    SourceDocument,
    /// Effective state read from a running container environment.
    RuntimeObservation,
    /// Explicit caller or user override.
    UserOverride,
    /// Default introduced by a named implementation profile.
    ImplementationDefault,
    /// Value selected by an explicit conversion decision.
    ConversionDecision,
}

impl Provenance {
    /// Creates provenance for an entire source.
    #[must_use]
    pub const fn source(source: SourceId) -> Self {
        Self {
            kind: ProvenanceKind::SourceDocument,
            source,
            span: None,
        }
    }

    /// Creates provenance for one byte range in a source.
    #[must_use]
    pub const fn spanned(source: SourceId, span: SourceSpan) -> Self {
        Self {
            kind: ProvenanceKind::SourceDocument,
            source,
            span: Some(span),
        }
    }

    /// Creates provenance for effective state observed from a runtime resource.
    #[must_use]
    pub const fn runtime_observation(source: SourceId) -> Self {
        Self {
            kind: ProvenanceKind::RuntimeObservation,
            source,
            span: None,
        }
    }

    /// Creates provenance for an explicit caller or user override.
    #[must_use]
    pub const fn user_override(source: SourceId) -> Self {
        Self {
            kind: ProvenanceKind::UserOverride,
            source,
            span: None,
        }
    }

    /// Creates provenance for a default introduced by a named implementation profile.
    #[must_use]
    pub const fn implementation_default(source: SourceId) -> Self {
        Self {
            kind: ProvenanceKind::ImplementationDefault,
            source,
            span: None,
        }
    }

    /// Creates provenance for a value chosen by an explicit conversion decision.
    #[must_use]
    pub const fn conversion_decision(source: SourceId) -> Self {
        Self {
            kind: ProvenanceKind::ConversionDecision,
            source,
            span: None,
        }
    }

    /// Returns how this origin entered the conversion plan.
    #[must_use]
    pub const fn kind(&self) -> ProvenanceKind {
        self.kind
    }

    /// Returns the source identity.
    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source
    }

    /// Returns the optional byte range.
    #[must_use]
    pub const fn span(&self) -> Option<SourceSpan> {
        self.span
    }
}

/// A value and every source location that contributed to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sourced<T> {
    value: T,
    origins: Vec<Provenance>,
}

impl<T> Sourced<T> {
    /// Creates a value without source provenance, such as a generated default.
    #[must_use]
    pub const fn generated(value: T) -> Self {
        Self {
            value,
            origins: Vec::new(),
        }
    }

    /// Creates a value with one source origin.
    #[must_use]
    pub fn from_source(value: T, origin: Provenance) -> Self {
        Self {
            value,
            origins: vec![origin],
        }
    }

    /// Adds another contributing origin in discovery order.
    pub fn add_origin(&mut self, origin: Provenance) {
        self.origins.push(origin);
    }

    /// Returns the value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns all origins in discovery order.
    #[must_use]
    pub fn origins(&self) -> &[Provenance] {
        &self.origins
    }

    /// Decomposes the sourced value.
    #[must_use]
    pub fn into_parts(self) -> (T, Vec<Provenance>) {
        (self.value, self.origins)
    }
}

fn validate_text(kind: &'static str, value: &str) -> Result<(), ModelError> {
    if value.is_empty() {
        return Err(ModelError::EmptyValue(kind));
    }
    if value.contains('\0') {
        return Err(ModelError::ContainsNul(kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Provenance, ProvenanceKind, SourceId, SourceSpan, Sourced};
    use crate::ModelError;

    #[test]
    fn retains_multiple_origins_in_discovery_order() -> Result<(), String> {
        let first_source = SourceId::new("compose.yaml").map_err(|error| error.to_string())?;
        let second_source = SourceId::new("compose.override.yaml").map_err(|error| error.to_string())?;
        let first_span = SourceSpan::new(10, 20).map_err(|error| error.to_string())?;
        let second_span = SourceSpan::new(30, 40).map_err(|error| error.to_string())?;
        let mut value = Sourced::from_source("image", Provenance::spanned(first_source, first_span));
        value.add_origin(Provenance::spanned(second_source, second_span));

        assert_eq!(value.origins()[0].span(), Some(first_span));
        assert_eq!(value.origins()[1].span(), Some(second_span));
        Ok(())
    }

    #[test]
    fn rejects_reversed_source_spans() {
        assert!(matches!(
            SourceSpan::new(20, 10),
            Err(ModelError::ReversedSpan { start: 20, end: 10 })
        ));
    }

    #[test]
    fn distinguishes_runtime_observations_from_authored_sources_and_decisions() -> Result<(), String> {
        let runtime = SourceId::new("runtime:container/web").map_err(|error| error.to_string())?;
        let decision = SourceId::new("decision:working-directory").map_err(|error| error.to_string())?;
        let runtime = Provenance::runtime_observation(runtime);
        let decision = Provenance::conversion_decision(decision);

        assert_eq!(runtime.kind(), ProvenanceKind::RuntimeObservation);
        assert_eq!(runtime.span(), None);
        assert_eq!(decision.kind(), ProvenanceKind::ConversionDecision);
        Ok(())
    }
}
