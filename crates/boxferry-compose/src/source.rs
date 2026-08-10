//! Explicit input bundle for one processed Compose project.

use std::collections::BTreeMap;

use boxferry_model::{Identifier, ModelError, SourceId};
use compose_lens::{merge::MergedProject, profiles::ProfileSelection, source::SourceId as ComposeSourceId};

/// A merged Compose project and the caller-owned context needed to import it safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeSource {
    project: MergedProject,
    fallback_application_name: Identifier,
    source_ids: BTreeMap<ComposeSourceId, SourceId>,
    profile_selection: Option<ProfileSelection>,
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
}
