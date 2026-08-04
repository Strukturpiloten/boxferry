//! External-style contract for the additive Docker runtime facade feature.

#![cfg(feature = "docker-runtime")]

use boxferry::{
    DockerAcquisitionError, DockerApiVersion, DockerCommandExecutor, DockerCommandOutput, DockerExpansionPolicy,
    DockerImporter, DockerInspectCommand, DockerInspectDocuments, DockerInspectSource, DockerInspector,
    DockerResourceSelection, Identifier, OverrideReconstruction, RuntimeImplementation,
};

struct EmptyExecutor;

impl DockerCommandExecutor for EmptyExecutor {
    fn execute(&self, _command: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError> {
        Ok(DockerCommandOutput::new("[]"))
    }
}

#[test]
fn embedded_caller_decodes_explicit_docker_documents_through_the_facade() -> Result<(), String> {
    let source = DockerInspectSource::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new("[]", "[]", "[]", "[]"),
    );
    let importer =
        DockerImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let result = importer.decode(&source);
    let snapshot = result.snapshot().ok_or("snapshot expected")?;

    assert_eq!(snapshot.implementation(), &RuntimeImplementation::Docker);
    assert!(snapshot.containers().is_empty());
    Ok(())
}

#[test]
fn embedded_caller_replaces_docker_execution_through_the_facade() -> Result<(), String> {
    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    let inspector = DockerInspector::new(
        EmptyExecutor,
        "docker",
        "unix:///run/user/1000/docker.sock",
        DockerApiVersion::new(1, 40),
    )
    .map_err(|error| error.to_string())?;
    let source = inspector
        .inspect_with_policy(
            Identifier::new("example").map_err(|error| error.to_string())?,
            &selection,
            DockerExpansionPolicy::ContainerResources,
        )
        .map_err(|error| error.to_string())?;

    assert_eq!(source.documents().containers(), "[]");
    Ok(())
}
