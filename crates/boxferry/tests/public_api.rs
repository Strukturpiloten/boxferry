//! Supported facade API exercised as an external crate would use it.

use boxferry::{Application, Identifier, InMemoryAdapter, LossPolicy, PlatformVersion, TargetProfile, convert};

#[test]
fn facade_converts_through_public_adapter_contracts() -> Result<(), String> {
    let application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    let adapter = InMemoryAdapter::exact("rendered target".to_owned());
    let target =
        TargetProfile::new("podman", PlatformVersion::new(5, 4, 0), None).map_err(|error| error.to_string())?;

    let result =
        convert(&adapter, &application, &adapter, &target, LossPolicy::ExactOnly).map_err(|error| error.to_string())?;
    assert_eq!(result.output().map(String::as_str), Some("rendered target"));
    Ok(())
}

#[cfg(feature = "compose")]
#[test]
fn facade_exposes_the_compose_import_adapter_additively() -> Result<(), String> {
    use boxferry::compose::compose_lens::{
        loader::{DocumentInput, DocumentOrigin, LoadedProject},
        merge::merge_project,
        source::SourceId as ComposeSourceId,
    };
    use boxferry::{ComposeImporter, ComposeSource, ImportAdapter, SourceId};

    let compose_source_id = ComposeSourceId::new(1);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_source_id,
        DocumentOrigin::new("compose.yaml", "."),
        "services:\n  web:\n    image: example.invalid/web:1\n",
    )])
    .map_err(|error| error.to_string())?;
    let merged = merge_project(&loaded, None);
    let project = merged.project().ok_or("merged Compose project expected")?.clone();
    let source = ComposeSource::new(project, Identifier::new("example").map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?
        .with_source_id(
            compose_source_id,
            SourceId::new("compose.yaml").map_err(|error| error.to_string())?,
        );
    let importer = ComposeImporter::new().map_err(|error| error.to_string())?;
    let result = importer.import(&source);

    assert_eq!(
        result.application().map(|application| application.services().len()),
        Some(1)
    );
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    Ok(())
}

#[cfg(feature = "quadlet")]
#[test]
fn facade_exposes_the_quadlet_export_adapter_additively() -> Result<(), String> {
    use boxferry::{ExportAdapter, ImageReference, QuadletExporter, QuadletGroupingPolicy, Service, Sourced};

    let mut application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
    let mut service = Service::new(Identifier::new("web").map_err(|error| error.to_string())?);
    service.set_image(Sourced::generated(
        ImageReference::parse("example.invalid/web:1").map_err(|error| error.to_string())?,
    ));
    application
        .add_service(Sourced::generated(service))
        .map_err(|error| error.to_string())?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )
    .map_err(|error| error.to_string())?;
    let exporter = QuadletExporter::new().map_err(|error| error.to_string())?;
    assert_eq!(exporter.grouping_policy(), QuadletGroupingPolicy::SeparateContainers);
    let output = exporter
        .plan(&application, &target)
        .map_err(|error| error.to_string())?
        .authorize(LossPolicy::ExactOnly);

    assert_eq!(
        output
            .output()
            .and_then(|output| output.file("web.container"))
            .map(boxferry::QuadletFile::text),
        Some("[Container]\nImage=example.invalid/web:1\n")
    );
    Ok(())
}
