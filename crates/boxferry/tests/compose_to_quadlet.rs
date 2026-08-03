//! Golden Compose-to-Quadlet conversion through the public facade.

#![cfg(all(feature = "compose", feature = "quadlet"))]

use std::{error::Error, fs, path::PathBuf};

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::{
    ComposeImporter, ComposeSource, ConversionKind, Identifier, LossPolicy, PlatformVersion, QuadletExporter,
    QuadletGroupingPolicy, SourceId, TargetProfile, convert,
};

const BASE_SOURCE_ID: u32 = 91;
const OVERRIDE_SOURCE_ID: u32 = 92;
const POD_SOURCE_ID: u32 = 93;

#[test]
fn converts_the_golden_project_with_explicit_partial_authorization() -> Result<(), Box<dyn Error>> {
    let directory = fixture_directory();
    let base = fixture_text("compose.yaml")?;
    let override_text = fixture_text("compose.override.yaml")?;
    let base_id = ComposeSourceId::new(BASE_SOURCE_ID);
    let override_id = ComposeSourceId::new(OVERRIDE_SOURCE_ID);
    let loaded = LoadedProject::load([
        DocumentInput::new(
            base_id,
            DocumentOrigin::new("compose.yaml", directory.display().to_string()),
            base,
        ),
        DocumentInput::new(
            override_id,
            DocumentOrigin::new("compose.override.yaml", directory.display().to_string()),
            override_text,
        ),
    ])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let project = merged.project().ok_or("merged project expected")?.clone();
    let selection = select_profiles(&project, &ProfileRequest::new());
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(base_id, SourceId::new("compose.yaml")?)
        .with_source_id(override_id, SourceId::new("compose.override.yaml")?)
        .with_profile_selection(selection);
    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?;
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let strict = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(strict.is_blocked());
    assert!(strict.output().is_none());

    let partial = convert(&importer, &source, &exporter, &target, LossPolicy::AllowPartial)?;
    let output = partial.output().ok_or("partial output expected")?;
    assert_eq!(
        output
            .files()
            .iter()
            .map(|file| file.name().as_str())
            .collect::<Vec<_>>(),
        ["frontend.network", "data.volume", "web.container"]
    );
    for name in ["frontend.network", "data.volume", "web.container"] {
        assert_eq!(
            output.file(name).map(boxferry::QuadletFile::text),
            Some(fixture_text(name)?.as_str()),
            "{name} differs from its reviewed golden output"
        );
    }
    assert!(output.file("worker.container").is_none());
    assert!(output.document_set().is_valid());
    assert_eq!(output.document_set().graph().edges().len(), 2);

    let diagnostics = partial
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0003")
        .map(|diagnostic| {
            let subject = diagnostic
                .fields()
                .iter()
                .find(|field| field.name() == "subject")
                .map_or("missing-subject", |field| field.value().expose());
            format!("{} {subject}", diagnostic.code().as_str())
        })
        .collect::<Vec<_>>();
    let expected = fixture_text("expected-diagnostics.txt")?
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics, expected);

    let unsupported = partial
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .collect::<Vec<_>>();
    assert_eq!(unsupported.len(), 5);
    assert!(unsupported.iter().all(|outcome| !outcome.origins().is_empty()));
    Ok(())
}

#[test]
fn converts_a_compatible_project_into_an_explicitly_authorized_pod() -> Result<(), Box<dyn Error>> {
    let directory = pod_fixture_directory();
    let compose = pod_fixture_text("compose.yaml")?;
    let compose_id = ComposeSourceId::new(POD_SOURCE_ID);
    let loaded = LoadedProject::load([DocumentInput::new(
        compose_id,
        DocumentOrigin::new("compose.yaml", directory.display().to_string()),
        compose,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    let project = merged.project().ok_or("merged project expected")?.clone();
    let selection = select_profiles(&project, &ProfileRequest::new());
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(compose_id, SourceId::new("compose.yaml")?)
        .with_profile_selection(selection);
    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?.with_grouping_policy(QuadletGroupingPolicy::SinglePod);
    let target = TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 0, 2)),
    )?;

    let strict = convert(&importer, &source, &exporter, &target, LossPolicy::ExactOnly)?;
    assert!(strict.is_blocked());
    let approximate = convert(&importer, &source, &exporter, &target, LossPolicy::AllowApproximate)?;
    let output = approximate.output().ok_or("approximate pod output expected")?;
    for name in ["frontend.network", "ferry-pod.pod", "web.container", "worker.container"] {
        assert_eq!(
            output.file(name).map(boxferry::QuadletFile::text),
            Some(pod_fixture_text(name)?.as_str()),
            "{name} differs from its reviewed golden output"
        );
    }
    assert_eq!(output.document_set().graph().edges().len(), 3);
    let grouping = approximate
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "application.grouping")
        .ok_or("grouping outcome expected")?;
    assert_eq!(grouping.kind(), ConversionKind::Approximate);
    assert!(!grouping.origins().is_empty());
    assert_eq!(
        approximate
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code().as_str() == "BFQ0007")
            .count(),
        1
    );
    Ok(())
}

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-core")
}

fn pod_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conversion/compose-to-quadlet-pod")
}

fn fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(fixture_directory().join(name))?)
}

fn pod_fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(pod_fixture_directory().join(name))?)
}
