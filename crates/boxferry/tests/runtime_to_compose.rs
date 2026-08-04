//! Public runtime-import to Compose-export vertical slice.

#![cfg(all(feature = "runtime", feature = "compose"))]

use boxferry::{
    ComposeExporter, ComposeRuntime, ContainerObservation, DOCKER_COMPOSE_TARGET, EffectiveCommand, Identifier,
    ImageReference, LossPolicy, OverrideReconstruction, PlatformVersion, RuntimeEnvironmentVariable,
    RuntimeImplementation, RuntimeImporter, RuntimeSnapshot, SourceId, TargetProfile, convert,
};

#[test]
fn observed_container_generates_reviewable_compose_through_public_api() -> Result<(), String> {
    let mut container = ContainerObservation::new(
        SourceId::new("runtime:docker:container:web").map_err(|error| error.to_string())?,
        Identifier::new("web").map_err(|error| error.to_string())?,
    );
    container.set_image(
        ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
        None,
    );
    container.set_command(EffectiveCommand::exec(["server", "--foreground"]));
    container.set_environment(vec![RuntimeEnvironmentVariable::new(
        Identifier::new("TOKEN").map_err(|error| error.to_string())?,
        "runtime-secret",
    )]);
    container.set_user("1001:1002");
    container.set_working_directory("/srv/runtime");
    container.set_read_only_root_filesystem(true);

    let mut snapshot = RuntimeSnapshot::new(
        Identifier::new("example").map_err(|error| error.to_string())?,
        RuntimeImplementation::Docker,
    );
    snapshot.add_container(container).map_err(|error| error.to_string())?;

    let importer =
        RuntimeImporter::new(OverrideReconstruction::PreserveObservedState).map_err(|error| error.to_string())?;
    let exporter = ComposeExporter::new()
        .map_err(|error| error.to_string())?
        .with_runtime(ComposeRuntime::DockerEngine(PlatformVersion::new(29, 7, 1)));
    let target = TargetProfile::new(
        DOCKER_COMPOSE_TARGET,
        PlatformVersion::new(5, 3, 1),
        Some(PlatformVersion::new(5, 3, 1)),
    )
    .map_err(|error| error.to_string())?;
    let result = convert(&importer, &snapshot, &exporter, &target, LossPolicy::AllowApproximate)
        .map_err(|error| error.to_string())?;

    let output = result
        .output()
        .ok_or_else(|| format!("generated Compose output expected: {result:#?}"))?;
    assert_eq!(
        output.text(),
        concat!(
            "name: \"example\"\n",
            "services:\n",
            "  \"web\":\n",
            "    image: \"example.invalid/web:1\"\n",
            "    command:\n",
            "      - \"server\"\n",
            "      - \"--foreground\"\n",
            "    environment:\n",
            "      - \"TOKEN=runtime-secret\"\n",
            "    user: \"1001:1002\"\n",
            "    working_dir: \"/srv/runtime\"\n",
            "    read_only: true\n",
        )
    );
    assert!(output.is_sensitive());
    assert!(!format!("{output:?}").contains("runtime-secret"));
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.subject() == "application.reconstruction"
            && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFR0001")
    }));
    Ok(())
}
