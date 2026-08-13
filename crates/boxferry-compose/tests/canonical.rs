//! Public same-format Compose canonicalization behavior.

use std::error::Error;

use boxferry_compose::ComposeSource;
use boxferry_model::Identifier;
use compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::{MergedProject, merge_project},
    profiles::{ProfileRequest, select_profiles},
    source::SourceId,
};

#[test]
fn canonicalization_preserves_unresolved_native_scalars_and_defaults() -> Result<(), Box<dyn Error>> {
    let source_id = SourceId::new(1);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("compose.yaml", "tests/canonical"),
        concat!(
            "name: canonical-example\n",
            "x-native: ${NATIVE:-retained}\n",
            "services:\n",
            "  web:\n",
            "    image: example.invalid/web:${TAG:-latest}\n",
            "    restart: ${RESTART_POLICY:-unless-stopped}\n",
            "    read_only: ${READ_ONLY:-true}\n",
            "    init: ${INIT:-true}\n",
            "    stop_grace_period: ${STOP_TIMEOUT:-30s}\n",
            "    environment:\n",
            "      VALUE: ${VALUE:-default}\n",
        ),
    )])?;
    let merged = merge_project(&loaded, None);
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    let project = merged.project().ok_or("merged project")?.clone();
    let source = ComposeSource::new(project, Identifier::new("canonical-example")?)?;

    let canonical = source.canonicalize()?;
    assert!(canonical.diagnostics().is_empty(), "{:#?}", canonical.diagnostics());
    let document = canonical.document().ok_or("canonical document")?;
    for expression in [
        "${NATIVE:-retained}",
        "${TAG:-latest}",
        "${RESTART_POLICY:-unless-stopped}",
        "${READ_ONLY:-true}",
        "${INIT:-true}",
        "${STOP_TIMEOUT:-30s}",
        "${VALUE:-default}",
    ] {
        assert!(document.text().contains(expression), "missing {expression}");
    }
    assert!(!document.is_sensitive());
    let debug = format!("{document:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("extension-default"));
    assert!(!debug.contains("default"));
    Ok(())
}

#[test]
fn canonicalization_suppresses_output_for_a_mismatched_profile_selection() -> Result<(), Box<dyn Error>> {
    let selected = merged_project(
        SourceId::new(2),
        "services:\n  selected:\n    image: example.invalid/selected:1\n",
    )?;
    let rendered = merged_project(
        SourceId::new(3),
        "services:\n  rendered:\n    image: example.invalid/rendered:1\n",
    )?;
    let selection = select_profiles(&selected, &ProfileRequest::new());
    let source = ComposeSource::new(rendered, Identifier::new("canonical-example")?)?.with_profile_selection(selection);

    let canonical = source.canonicalize()?;
    assert!(canonical.document().is_none());
    assert!(
        canonical
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0197")
    );
    Ok(())
}

fn merged_project(source_id: SourceId, text: &str) -> Result<MergedProject, Box<dyn Error>> {
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("compose.yaml", "tests/canonical"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    Ok(merged.project().ok_or("merged project")?.clone())
}
