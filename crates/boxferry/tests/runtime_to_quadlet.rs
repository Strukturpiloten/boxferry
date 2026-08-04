//! Public runtime-import to Quadlet-export vertical slice.

#![cfg(all(feature = "runtime", feature = "quadlet"))]

use boxferry::{
    ContainerObservation, EffectiveCommand, Identifier, ImageReference, LossPolicy, OverrideReconstruction,
    PlatformVersion, QuadletExporter, QuadletFile, RuntimeImplementation, RuntimeImporter, RuntimeSnapshot, SourceId,
    TargetProfile, convert,
};

#[cfg(feature = "docker-runtime")]
use boxferry::{DockerApiVersion, DockerImporter, DockerInspectDocuments, DockerInspectSource};

#[test]
fn observed_container_generates_reviewable_quadlet_through_public_api() -> Result<(), String> {
    let mut container = ContainerObservation::new(
        SourceId::new("runtime:podman:container:web").map_err(|error| error.to_string())?,
        Identifier::new("web").map_err(|error| error.to_string())?,
    );
    container.set_image(
        ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
        None,
    );
    container.set_command(EffectiveCommand::exec(["server", "--foreground"]));
    container.set_environment(Vec::new());
    container.set_user("1001:1002");
    container.set_working_directory("/srv/runtime");
    container.set_read_only_root_filesystem(true);

    let mut snapshot = RuntimeSnapshot::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        RuntimeImplementation::Podman,
    );
    snapshot.add_container(container).map_err(|error| error.to_string())?;

    let importer =
        RuntimeImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let exporter = QuadletExporter::new().map_err(|error| error.to_string())?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )
    .map_err(|error| error.to_string())?;
    let result = convert(&importer, &snapshot, &exporter, &target, LossPolicy::AllowApproximate)
        .map_err(|error| error.to_string())?;

    assert_eq!(
        result
            .output()
            .and_then(|output| output.file("web.container"))
            .map(QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Exec=server --foreground\n",
            "User=1001\n",
            "Group=1002\n",
            "WorkingDir=/srv/runtime\n",
            "ReadOnly=true\n",
        ))
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "application.reconstruction"
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0001")
    }));
    Ok(())
}

#[cfg(feature = "docker-runtime")]
#[test]
fn docker_inspection_generates_reviewable_quadlet_through_public_api() -> Result<(), String> {
    let source = DockerInspectSource::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        DockerApiVersion::new(1, 40),
        DockerInspectDocuments::new(
            r#"[{
                "Id":"container-id",
                "Image":"image-id",
                "Name":"/web",
                "Path":"server",
                "Args":["--foreground"],
                "Config":{"Image":"example.invalid/web:1","Env":[],"User":"1001:1002","WorkingDir":"/srv/runtime"},
                "HostConfig":{"ReadonlyRootfs":true},
                "Mounts":[],
                "NetworkSettings":{"Ports":{},"Networks":{}}
            }]"#,
            r#"[{"Id":"image-id","Config":{"Env":[]}}]"#,
            "[]",
            "[]",
        ),
    );
    let importer =
        DockerImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let exporter = QuadletExporter::new().map_err(|error| error.to_string())?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )
    .map_err(|error| error.to_string())?;
    let result = convert(&importer, &source, &exporter, &target, LossPolicy::AllowApproximate)
        .map_err(|error| error.to_string())?;

    assert_eq!(
        result
            .output()
            .and_then(|output| output.file("web.container"))
            .map(QuadletFile::text),
        Some(concat!(
            "[Container]\n",
            "Image=example.invalid/web:1\n",
            "Exec=server --foreground\n",
            "User=1001\n",
            "Group=1002\n",
            "WorkingDir=/srv/runtime\n",
            "ReadOnly=true\n",
        ))
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "application.reconstruction"
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0001")
    }));
    Ok(())
}
