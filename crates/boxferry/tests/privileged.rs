//! Explicit privileged intent across the public neutral conversion pipeline.

#![cfg(all(feature = "compose", feature = "quadlet"))]

use std::error::Error;

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};
use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeImporter,
    ComposeSource, ConversionKind, ExportAdapter, Identifier, ImportAdapter, LossPolicy, PlatformVersion,
    QuadletDocumentInput, QuadletExporter, QuadletImporter, QuadletSource, SourceId, TargetProfile, convert,
};

#[test]
fn explicit_privileged_choices_round_trip_at_both_quadlet_boundaries() -> Result<(), Box<dyn Error>> {
    for choice in [None, Some(false), Some(true)] {
        let field = choice.map_or_else(String::new, |value| format!("    privileged: {value}\n"));
        let source = compose_source(&format!(
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n{field}    read_only: false\n    security_opt: [no-new-privileges:false]\n"
        ))?;
        let imported = ComposeImporter::new()?.import(&source);
        let application = imported.application().ok_or("Compose application")?;
        let service = application.services()[0].value();
        assert_eq!(service.privileged().map(|value| *value.value()), choice);
        if let Some(privileged) = service.privileged() {
            assert!(!privileged.origins().is_empty());
            assert!(privileged.origins().iter().all(|origin| origin.span().is_some()));
        }
        for version in [PlatformVersion::new(5, 4, 0), PlatformVersion::new(6, 1, 2)] {
            let target = TargetProfile::new("podman", version, Some(version))?;
            let result = convert(
                &ComposeImporter::new()?,
                &source,
                &QuadletExporter::new()?,
                &target,
                LossPolicy::ExactOnly,
            )?;
            assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
            let text = result
                .output()
                .and_then(|output| output.file("web.container"))
                .ok_or("Quadlet output")?
                .text();
            match choice {
                None => assert!(!text.contains("PodmanArgs=")),
                Some(true) => assert!(text.contains("PodmanArgs=--privileged\n"), "{text}"),
                Some(false) => assert!(text.contains("PodmanArgs=--privileged=false\n"), "{text}"),
            }
            assert!(text.contains("ReadOnly=false\n"));
            assert!(text.contains("NoNewPrivileges=false\n"));
            let quadlet = quadlet_source(text)?;
            let reimported = QuadletImporter::new()?.import(&quadlet);
            let round_trip = reimported.application().ok_or("Quadlet application")?;
            assert!(round_trip.retained_native_evidence().is_empty());
            assert_eq!(
                round_trip.services()[0]
                    .value()
                    .privileged()
                    .map(|value| *value.value()),
                choice
            );
            let compose = ComposeExporter::new()?
                .plan(round_trip, &compose_target()?)?
                .authorize(LossPolicy::ExactOnly);
            assert!(!compose.is_blocked(), "{:#?}", compose.diagnostics());
            let text = compose.output().ok_or("Compose output")?.text();
            assert_eq!(text.contains("privileged:"), choice.is_some());
            if let Some(value) = choice {
                assert!(text.contains(&format!("privileged: {value}")), "{text}");
            }
            let final_source = compose_source(text)?;
            let final_import = ComposeImporter::new()?.import(&final_source);
            assert_eq!(
                final_import.application().ok_or("final application")?.services()[0]
                    .value()
                    .privileged()
                    .map(|value| *value.value()),
                choice
            );
        }
    }
    Ok(())
}

#[test]
fn privileged_provenance_survives_compose_export_and_quadlet_planning() -> Result<(), Box<dyn Error>> {
    let source = compose_source("---\nservices:\n  web:\n    image: example.invalid/web:1\n    privileged: false\n")?;
    let imported = ComposeImporter::new()?.import(&source);
    let application = imported.application().ok_or("application")?;
    let origins = application.services()[0]
        .value()
        .privileged()
        .ok_or("explicit false")?
        .origins();
    let version = PlatformVersion::new(5, 4, 0);
    let quadlet = QuadletExporter::new()?.plan(application, &TargetProfile::new("podman", version, Some(version))?)?;
    let compose = ComposeExporter::new()?.plan(application, &compose_target()?)?;
    for outcomes in [imported.outcomes(), quadlet.outcomes(), compose.outcomes()] {
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome.subject() == "services.web.privileged")
            .ok_or("privileged outcome")?;
        assert_eq!(outcome.kind(), ConversionKind::Exact);
        assert_eq!(outcome.origins(), origins);
    }
    for version in [PlatformVersion::new(5, 3, 99), PlatformVersion::new(6, 1, 3)] {
        let plan = QuadletExporter::new()?.plan(application, &TargetProfile::new("podman", version, Some(version))?)?;
        assert!(plan.authorize(LossPolicy::AllowPartial).is_blocked());
    }
    Ok(())
}

#[test]
fn complex_native_arguments_stay_protected_unsupported_evidence() -> Result<(), Box<dyn Error>> {
    for arguments in [
        "PodmanArgs=--privileged --secret=private-sentinel\n",
        "PodmanArgs=--privileged\nPodmanArgs=--privileged=false\n",
        "PodmanArgs=--privileged\nPodmanArgs=\n",
        "PodmanArgs=--privileged=true\n",
        "PodmanArgs=\"--privileged\"\n",
        "PodmanArgs=--privileged \\\n  --secret=private-sentinel\n",
    ] {
        let source = quadlet_source(&format!("[Container]\nImage=example.invalid/web:1\n{arguments}"))?;
        let imported = QuadletImporter::new()?.import(&source);
        let application = imported.application().ok_or("application")?;
        assert!(application.services()[0].value().privileged().is_none());
        assert!(!application.retained_native_evidence().is_empty());
        assert!(!format!("{application:?}").contains("private-sentinel"));
        let plan = ComposeExporter::new()?.plan(application, &compose_target()?)?;
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "services.web.podman_args"
                    && outcome.kind() == ConversionKind::Unsupported)
        );
        assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
        let partial = plan.authorize(LossPolicy::AllowPartial);
        let text = partial.output().ok_or("partial output")?.text();
        assert!(!text.contains("privileged:"));
        assert!(!text.contains("private-sentinel"));
    }
    Ok(())
}

#[cfg(feature = "podman")]
#[test]
fn podman_output_reports_explicit_true_and_false_with_origins() -> Result<(), Box<dyn Error>> {
    for choice in [false, true] {
        let source = compose_source(&format!(
            "---\nservices:\n  web:\n    image: example.invalid/web:1\n    privileged: {choice}\n"
        ))?;
        let imported = ComposeImporter::new()?.import(&source);
        let application = imported.application().ok_or("application")?;
        let origins = application.services()[0]
            .value()
            .privileged()
            .ok_or("explicit choice")?
            .origins();
        let version = PlatformVersion::new(6, 1, 0);
        let plan = boxferry::PodmanExporter::new()?
            .plan(application, &TargetProfile::new("podman", version, Some(version))?)?;
        let outcome = plan
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == "services.web.privileged")
            .ok_or("unsupported outcome")?;
        assert_eq!(outcome.kind(), ConversionKind::Unsupported);
        assert_eq!(outcome.origins(), origins);
        assert!(plan.clone().authorize(LossPolicy::ExactOnly).is_blocked());
        assert!(!plan.authorize(LossPolicy::AllowPartial).is_blocked());
    }
    Ok(())
}

fn compose_source(text: &str) -> Result<ComposeSource, Box<dyn Error>> {
    let source_id = ComposeSourceId::new(1);
    let loaded = LoadedProject::load([DocumentInput::new(
        source_id,
        DocumentOrigin::new("privileged.yaml", "privileged.yaml"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    Ok(ComposeSource::new(
        merged.project().ok_or("merged project")?.clone(),
        Identifier::new("privileged")?,
    )?
    .with_source_id(source_id, SourceId::new("privileged.yaml")?))
}

fn quadlet_source(text: &str) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(
        Identifier::new("privileged")?,
        [QuadletDocumentInput::new(
            "web.container",
            QuadletSourceId::new(1),
            text,
        )],
    )?
    .into_source())
}

fn compose_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?)
}
