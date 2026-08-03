//! Public adapter behavior backed by the repository fixture contract.

use std::{error::Error, fs, path::PathBuf};

use boxferry_compose::{ComposeImporter, ComposeSource};
use boxferry_engine::{ConversionKind, ImportAdapter, Severity};
use boxferry_model::{Command, EnvironmentValue, Identifier, MountSource, Protocol, SelinuxRelabel, SourceId};
use compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::{MergedProject, merge_project},
    profiles::{ProfileRequest, ProfileSelection, select_profiles},
    source::SourceId as ComposeSourceId,
};

const COMPOSE_SOURCE_ID: u32 = 71;
const OVERRIDE_SOURCE_ID: u32 = 72;

#[test]
fn imports_the_core_fixture_without_loss_and_excludes_inactive_profiles() -> Result<(), Box<dyn Error>> {
    let base = fixture_text("compose.yaml")?;
    let overlay = fixture_text("compose.override.yaml")?;
    let (project, selection) = processed_project(&base, &overlay, &ProfileRequest::new())?;
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?)
        .with_source_id(
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            SourceId::new("compose.override.yaml")?,
        )
        .with_profile_selection(selection);

    let result = ComposeImporter::new()?.import(&source);
    assert!(result.diagnostics().is_empty(), "{:#?}", result.diagnostics());
    assert!(
        result
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact),
        "{:#?}",
        result.outcomes()
    );
    let application = result.application().ok_or("application expected")?;
    assert_eq!(application.name().as_str(), "ferry-demo");
    assert_eq!(application.services().len(), 1);
    assert_eq!(application.volumes().len(), 1);
    assert_eq!(application.networks().len(), 1);

    let web = application.services()[0].value();
    assert_eq!(web.name().as_str(), "web");
    assert_eq!(
        web.image().map(|image| image.value().as_str()),
        Some("registry.example:5000/team/web:1.3@sha256:fedcba")
    );
    assert!(matches!(
        web.command().map(boxferry_model::Sourced::value),
        Some(Command::Exec(values))
            if values
                .iter()
                .map(boxferry_model::ProtectedString::expose)
                .eq(["php", "-v"])
    ));
    assert_eq!(web.environment().len(), 4);
    assert!(matches!(web.environment()[1].value().value(), EnvironmentValue::Host));
    assert!(matches!(
        web.environment()[2].value().value(),
        EnvironmentValue::Literal(value) if value.is_sensitive() && value.expose() == "9090"
    ));
    assert_eq!(web.ports().len(), 2);
    assert_eq!(web.ports()[0].value().container(), 80);
    assert_eq!(web.ports()[0].value().published(), Some(8080));
    assert_eq!(web.ports()[0].value().host_address(), Some("127.0.0.1"));
    assert!(matches!(web.ports()[0].value().protocol(), Protocol::Tcp));
    assert_eq!(web.mounts().len(), 4);
    assert!(matches!(web.mounts()[0].value().source(), MountSource::Volume(name) if name.as_str() == "data"));
    assert!(web.mounts()[0].value().read_only());
    assert_eq!(web.mounts()[1].value().selinux_relabel(), Some(SelinuxRelabel::Private));
    assert_eq!(web.mounts()[2].value().selinux_relabel(), Some(SelinuxRelabel::Shared));
    assert_eq!(web.mounts()[3].value().selinux_relabel(), Some(SelinuxRelabel::Private));
    assert_eq!(web.networks()[0].value().aliases(), ["web.local"]);
    assert_eq!(
        web.image()
            .map(|image| {
                image
                    .origins()
                    .iter()
                    .map(|origin| origin.source_id().as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ["compose.yaml", "compose.override.yaml"]
    );
    Ok(())
}

#[test]
fn rejects_implicit_profile_guessing() -> Result<(), Box<dyn Error>> {
    let text = fixture_text("compose.yaml")?;
    let project = merged_project([(ComposeSourceId::new(COMPOSE_SOURCE_ID), "compose.yaml", text.as_str())])?;
    let source = ComposeSource::new(project, Identifier::new("fallback")?)?
        .with_source_id(ComposeSourceId::new(COMPOSE_SOURCE_ID), SourceId::new("compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFC0002" && diagnostic.severity() == Severity::Error })
    );
    assert!(
        result
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    Ok(())
}

#[test]
fn reports_port_ranges_as_policy_controlled_unsupported_intent() -> Result<(), Box<dyn Error>> {
    let text = "services:\n  web:\n    image: example.invalid/web\n    ports: [\"8000-8002:80-82\"]\n";
    let compose_source_id = ComposeSourceId::new(73);
    let project = merged_project([(compose_source_id, "ranges.compose.yaml", text)])?;
    let source = ComposeSource::new(project, Identifier::new("ranges")?)?
        .with_source_id(compose_source_id, SourceId::new("ranges.compose.yaml")?);

    let result = ComposeImporter::new()?.import(&source);
    assert!(
        result.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFC0004" && diagnostic.severity() == Severity::Warning
        })
    );
    assert!(result.outcomes().iter().any(|outcome| {
        outcome.kind() == ConversionKind::Unsupported && outcome.subject() == "services.web.ports[0]"
    }));
    Ok(())
}

fn processed_project(
    base: &str,
    overlay: &str,
    request: &ProfileRequest,
) -> Result<(MergedProject, ProfileSelection), Box<dyn Error>> {
    let project = merged_project([
        (ComposeSourceId::new(COMPOSE_SOURCE_ID), "compose.yaml", base),
        (
            ComposeSourceId::new(OVERRIDE_SOURCE_ID),
            "compose.override.yaml",
            overlay,
        ),
    ])?;
    let selection = select_profiles(&project, request);
    Ok((project, selection))
}

fn merged_project<'a>(
    inputs: impl IntoIterator<Item = (ComposeSourceId, &'a str, &'a str)>,
) -> Result<MergedProject, Box<dyn Error>> {
    let loaded = LoadedProject::load(inputs.into_iter().map(|(source_id, name, text)| {
        DocumentInput::new(
            source_id,
            DocumentOrigin::new(name, "fixtures/adapter-contract/compose-import-core"),
            text,
        )
    }))?;
    let merged = merge_project(&loaded, None);
    if !merged.is_valid() {
        return Err(format!("merge diagnostics: {:#?}", merged.diagnostics()).into());
    }
    Ok(merged.project().ok_or("merged project expected")?.clone())
}

fn fixture_text(name: &str) -> Result<String, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-contract/compose-import-core")
        .join(name);
    Ok(fs::read_to_string(path)?)
}
