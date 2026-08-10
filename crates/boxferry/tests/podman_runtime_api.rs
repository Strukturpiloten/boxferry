//! External-style contract for the additive Podman runtime facade feature.

#![cfg(feature = "podman-runtime")]

use boxferry::{
    Identifier, OverrideReconstruction, PlatformVersion, PodmanAcquisitionError, PodmanCommandExecutor,
    PodmanCommandOutput, PodmanExpansionPolicy, PodmanImporter, PodmanInspectCommand, PodmanInspectDocuments,
    PodmanInspectSource, PodmanInspector, PodmanResourceSelection, RuntimeImplementation,
};

struct EmptyExecutor;

impl PodmanCommandExecutor for EmptyExecutor {
    fn execute(&self, _command: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError> {
        Ok(PodmanCommandOutput::new("[]"))
    }
}

#[test]
fn embedded_caller_decodes_explicit_podman_documents_through_the_facade() -> Result<(), String> {
    let source = PodmanInspectSource::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        PlatformVersion::new(5, 4, 0),
        PodmanInspectDocuments::new("[]", "[]", "[]", "[]", "[]"),
    );
    let importer =
        PodmanImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let result = importer.decode(&source);
    let snapshot = result.snapshot().ok_or("snapshot expected")?;

    assert_eq!(snapshot.implementation(), &RuntimeImplementation::Podman);
    assert!(snapshot.containers().is_empty());
    Ok(())
}

#[test]
fn embedded_caller_replaces_podman_execution_through_the_facade() -> Result<(), String> {
    let mut selection = PodmanResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    let inspector = PodmanInspector::new(EmptyExecutor, "podman", PlatformVersion::new(5, 4, 0))
        .map_err(|error| error.to_string())?;
    let source = inspector
        .inspect_with_policy(
            Identifier::new("example").map_err(|error| error.to_string())?,
            &selection,
            PodmanExpansionPolicy::ContainerResources,
        )
        .map_err(|error| error.to_string())?;

    assert_eq!(source.documents().containers(), "[]");
    Ok(())
}
