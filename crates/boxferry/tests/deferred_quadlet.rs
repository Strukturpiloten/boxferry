//! Embedding API authorization, source evidence, and decode-once native literal roundtrips.
#![cfg(all(feature = "compose", feature = "quadlet"))]
use boxferry::{
    compose::compose_lens::{
        loader::{DocumentInput, DocumentOrigin, LoadedProject},
        merge::merge_project,
        source::SourceId as ComposeId,
    },
    *,
};
use std::error::Error;
fn source(text: &str, options: QuadletDeferredOptions) -> Result<ComposeSource, Box<dyn Error>> {
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeId::new(1),
        DocumentOrigin::new("deferred.yaml", "."),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    assert!(merged.is_valid(), "{:?}", merged.diagnostics());
    Ok(ComposeSource::new(
        merged.project().ok_or("merged project")?.clone(),
        Identifier::new("deferred")?,
    )?
    .with_source_id(ComposeId::new(1), SourceId::new("deferred.yaml")?)
    .with_deferred_options(options))
}
fn options() -> Result<QuadletDeferredOptions, Box<dyn Error>> {
    let mut options = QuadletDeferredOptions::new();
    options.add_service(
        Identifier::new("app")?,
        Sourced::generated(ServiceExecutionEnvironment::new(
            vec![Sourced::generated(ServiceEnvironmentName::new("RUN_MODE")?)],
            Vec::new(),
            vec![Sourced::generated(ServiceEnvironmentFile::new(
                "/nonexistent/provider.env",
            )?)],
        )?),
    )?;
    Ok(options)
}
fn target(minimum: PlatformVersion, maximum: PlatformVersion) -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new("podman", minimum, Some(maximum))?)
}
fn latest() -> Result<TargetProfile, Box<dyn Error>> {
    target(PlatformVersion::new(5, 4, 0), PlatformVersion::new(6, 1, 2))
}
const BASE: &str = "---\nname: deferred\nservices:\n  app:\n    image: example.invalid/app:1\n    restart: 'no'\n";
#[test]
fn embedding_defaults_to_safe_disclosure_and_retains_segment_provenance() -> Result<(), Box<dyn Error>> {
    let input = source(
        &format!("{BASE}    environment: {{ APP_MODE: \"pre-${{RUN_MODE}}\" }}\n"),
        options()?,
    )?;
    let imported = ComposeImporter::new()?.import(&input);
    let application = imported.application().ok_or("application")?;
    let EnvironmentValue::Deferred(expression) = application.services()[0].value().environment()[0].value().value()
    else {
        return Err("deferred expression missing".into());
    };
    assert!(expression.segments().iter().all(|part| {
        part.origins()
            .iter()
            .any(|origin| origin.span().is_some() && origin.source_id().as_str() == "deferred.yaml")
    }));
    let blocked = QuadletExporter::new()?.plan(application, &latest()?)?;
    assert!(
        blocked
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported && outcome.subject().contains("APP_MODE"))
    );
    let included = convert(
        &ComposeImporter::new()?,
        &input,
        &QuadletExporter::new()?.with_deferred_environment_values(true),
        &latest()?,
        LossPolicy::ExactOnly,
    )?;
    assert!(!included.is_blocked(), "{:?}", included.diagnostics());
    assert!(
        included.output().ok_or("output")?.files()[0]
            .text()
            .contains("APP_MODE=pre-${RUN_MODE}")
    );
    Ok(())
}
#[test]
fn target_boundaries_cover_minimum_and_current_and_reject_unsupported_ranges() -> Result<(), Box<dyn Error>> {
    let input = source(
        &format!("{BASE}    environment: {{ APP_MODE: \"$RUN_MODE\" }}\n"),
        options()?,
    )?;
    for version in [PlatformVersion::new(5, 4, 0), PlatformVersion::new(6, 1, 2)] {
        let result = convert(
            &ComposeImporter::new()?,
            &input,
            &QuadletExporter::new()?,
            &target(version, version)?,
            LossPolicy::ExactOnly,
        )?;
        assert!(!result.is_blocked(), "{:?}", result.diagnostics());
        assert!(
            result.output().ok_or("output")?.files()[0]
                .text()
                .contains("APP_MODE=${RUN_MODE}")
        );
    }
    let result = convert(
        &ComposeImporter::new()?,
        &input,
        &QuadletExporter::new()?,
        &target(PlatformVersion::new(5, 3, 0), PlatformVersion::new(5, 3, 0))?,
        LossPolicy::AllowPartial,
    );
    assert!(result.is_err() || result.as_ref().is_ok_and(ConversionResult::is_blocked));
    Ok(())
}
fn parse_native(text: &str) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(
        Identifier::new("deferred")?,
        [QuadletDocumentInput::new(
            "app.container",
            quadlet::quadlet_lens::source::SourceId::new(1),
            text,
        )],
    )?
    .into_source())
}
fn literal_signature(application: &Application) -> Result<(Vec<String>, Vec<String>), Box<dyn Error>> {
    let service = application.services().first().ok_or("service")?.value();
    let mut environment = service
        .environment()
        .iter()
        .map(|entry| match entry.value().value() {
            EnvironmentValue::Literal(value) => Ok(format!("{}={}", entry.value().name().as_str(), value.expose())),
            _ => Err("literal environment expected"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    environment.sort();
    let mounts = service
        .mounts()
        .iter()
        .map(|mount| match mount.value().source() {
            MountSource::HostPath(path) => Ok(path.clone()),
            _ => Err("literal host path expected"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((environment, mounts))
}
#[test]
fn escaped_braced_dollars_and_percent_survive_quadlet_reexport_with_compose_loss() -> Result<(), Box<dyn Error>> {
    let native = "[Container]\nImage=example.invalid/app:1\nEnvironment=\"BRACED=$${ROOT}\" \"UNBRACED=$$ROOT\" \"PAIRS=$$$$\" \"PERCENT=%%h\"\nVolume=/srv/%%h:/data/%%h:ro\n\n[Service]\nRestart=no\n";
    let first = QuadletImporter::new()?.import(&parse_native(native)?);
    let application = first.application().ok_or("imported application")?;
    assert!(
        !first
            .outcomes()
            .iter()
            .any(|outcome| matches!(outcome.kind(), ConversionKind::Unsupported | ConversionKind::Invalid)),
        "{:?}",
        first.diagnostics()
    );
    let expected = literal_signature(application)?;
    assert!(expected.0.contains(&"BRACED=${ROOT}".to_owned()));
    let compose_target = TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    let composed = ComposeExporter::new()?.plan(application, &compose_target)?;
    assert!(
        composed
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported && outcome.subject().contains("BRACED"))
    );
    let rendered = convert_imported(first, &QuadletExporter::new()?, &latest()?, LossPolicy::ExactOnly)?;
    assert!(!rendered.is_blocked(), "{:?}", rendered.diagnostics());
    let final_text = rendered.output().ok_or("quadlet output")?.files()[0].text();
    let final_import = QuadletImporter::new()?.import(&parse_native(final_text)?);
    assert_eq!(
        literal_signature(final_import.application().ok_or("final application")?)?,
        expected
    );
    assert!(
        !final_import
            .outcomes()
            .iter()
            .any(|outcome| matches!(outcome.kind(), ConversionKind::Unsupported | ConversionKind::Invalid)),
        "{:?}",
        final_import.diagnostics()
    );
    Ok(())
}
#[test]
fn generated_deferred_native_input_is_explicitly_unsupported_on_reverse() -> Result<(), Box<dyn Error>> {
    let input = source(
        &format!("{BASE}    environment: {{ APP_MODE: \"${{RUN_MODE}}\" }}\n"),
        options()?,
    )?;
    let result = convert(
        &ComposeImporter::new()?,
        &input,
        &QuadletExporter::new()?,
        &latest()?,
        LossPolicy::ExactOnly,
    )?;
    let text = result.output().ok_or("output")?.files()[0].text();
    let reverse = QuadletImporter::new()?.import(&parse_native(text)?);
    assert!(
        reverse
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported
                && outcome.subject().contains("environment.APP_MODE"))
    );
    assert!(
        reverse
            .application()
            .is_some_and(|app| app.services()[0].value().execution_environment().is_none())
    );
    Ok(())
}

#[test]
fn deferred_policy_rejects_previously_acquired_native_interpolation_values() -> Result<(), Box<dyn Error>> {
    let text = format!("{BASE}    environment: {{ APP_MODE: \"${{RUN_MODE}}\" }}\n");
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeId::new(1),
        DocumentOrigin::new("deferred.yaml", "."),
        text,
    )])?;
    let mut environment = compose::compose_lens::interpolation::MapEnvironment::new();
    let _ = environment.insert_sensitive("RUN_MODE", "${RUN_MODE}");
    let interpolated = loaded.interpolate(&environment);
    let merged = merge_project(&loaded, Some(&interpolated));
    let input = ComposeSource::new(merged.project().ok_or("project")?.clone(), Identifier::new("deferred")?)?
        .with_deferred_options(options()?)
        .with_interpolation_applied();
    let imported = ComposeImporter::new()?.import(&input);
    assert!(imported.application().is_none());
    assert!(
        imported
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0005")
    );
    Ok(())
}

#[test]
fn literals_never_activate_in_compose_regardless_of_source_origins() -> Result<(), Box<dyn Error>> {
    let compose_target = TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    for sourced in [false, true] {
        let mut app = Application::new(Identifier::new("literal")?);
        let mut service = Service::new(Identifier::new("app")?);
        service.set_image(Sourced::generated(ImageReference::parse("example.invalid/app:1")?));
        let value = EnvironmentVariable::new(
            Identifier::new("VALUE")?,
            EnvironmentValue::Literal(ProtectedString::sensitive("${PRIVATE_LITERAL}")),
        );
        service.add_environment(if sourced {
            Sourced::from_source(value, Provenance::source(SourceId::new("arbitrary-document")?))
        } else {
            Sourced::generated(value)
        });
        app.add_service(Sourced::generated(service))?;
        let plan = ComposeExporter::new()?.plan(&app, &compose_target)?;
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Unsupported
                    && outcome.subject().contains("environment.VALUE"))
        );
        assert!(!format!("{:?}", plan.diagnostics()).contains("PRIVATE_LITERAL"));
    }
    let imported = ComposeImporter::new()?.import(&source(
        &format!("{BASE}    environment:\n      VALUE: ${{UNRESOLVED}}\n"),
        QuadletDeferredOptions::new(),
    )?);
    let app = imported.application().ok_or("application")?;
    assert!(
        matches!(app.services()[0].value().environment()[0].value().value(), EnvironmentValue::UnresolvedSource { source_format, .. } if source_format.as_str() == "compose")
    );
    let mut app = app.clone();
    app.withhold_literal_environment_values();
    assert!(matches!(
        app.services()[0].value().environment()[0].value().value(),
        EnvironmentValue::Required
    ));
    Ok(())
}

#[test]
fn provider_canaries_stay_protected_in_model_plan_outcomes_and_diagnostics_debug() -> Result<(), Box<dyn Error>> {
    let name = ServiceEnvironmentName::new("PRIVATE_PROVIDER_NAME")?;
    let mut service = Service::new(Identifier::new("app")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/app:1")?));
    service.add_environment(Sourced::generated(EnvironmentVariable::new(
        Identifier::new("PUBLIC_CONTAINER_KEY")?,
        EnvironmentValue::Deferred(DeferredExpression::new(vec![Sourced::generated(
            DeferredSegment::Reference(name.clone()),
        )])?),
    )));
    service.set_execution_environment(Sourced::generated(ServiceExecutionEnvironment::new(
        vec![Sourced::generated(name.clone())],
        vec![Sourced::generated(ServiceEnvironmentAssignment::new(
            name,
            "PRIVATE_PROVIDER_VALUE",
        )?)],
        vec![Sourced::generated(ServiceEnvironmentFile::new(
            "/PRIVATE_PROVIDER_PATH",
        )?)],
    )?));
    let mut application = Application::new(Identifier::new("private")?);
    application.add_service(Sourced::generated(service))?;
    let plan = QuadletExporter::new()?
        .with_deferred_environment_values(true)
        .plan(&application, &latest()?)?;
    let debug = format!(
        "{application:?} {plan:?} {:?} {:?}",
        plan.outcomes(),
        plan.diagnostics()
    );
    for canary in [
        "PRIVATE_PROVIDER_NAME",
        "PRIVATE_PROVIDER_VALUE",
        "PRIVATE_PROVIDER_PATH",
    ] {
        assert!(!debug.contains(canary), "protected canary leaked");
    }
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.subject().ends_with(".inline[0]"))
    );
    Ok(())
}

#[test]
fn literal_path_marker_is_independent_of_provenance_and_preserves_legacy_context_paths() -> Result<(), Box<dyn Error>> {
    for sourced in [false, true] {
        for literal in [false, true] {
            let mut service = Service::new(Identifier::new("app")?);
            service.set_image(Sourced::generated(ImageReference::parse("example.invalid/app:1")?));
            let mount = Mount::new(MountSource::HostPath("/srv/%h".to_owned()), "/data", false)?;
            let mount = if literal { mount.with_literal_paths() } else { mount };
            assert_eq!(mount.paths_are_literal(), literal);
            service.add_mount(if sourced {
                Sourced::from_source(mount, Provenance::source(SourceId::new("arbitrary-source")?))
            } else {
                Sourced::generated(mount)
            });
            let mut app = Application::new(Identifier::new("paths")?);
            app.add_service(Sourced::generated(service))?;
            let plan = QuadletExporter::new()?
                .plan(&app, &latest()?)?
                .authorize(LossPolicy::ExactOnly);
            let text = plan.output().ok_or("authorized output")?.files()[0].text();
            let expected = if literal {
                "Volume=/srv/%%h:/data"
            } else {
                "Volume=/srv/%h:/data"
            };
            assert!(text.contains(expected), "{text}");
        }
    }
    Ok(())
}

#[test]
fn explicit_overlay_evidence_prevents_escaped_literals_from_becoming_deferred_refs() -> Result<(), Box<dyn Error>> {
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeId::new(1),
        DocumentOrigin::new("overlay.yaml", "."),
        format!("{BASE}    environment:\n      APP_MODE: $${{RUN_MODE}}\n"),
    )])?;
    let interpolated = loaded.interpolate(&compose::compose_lens::interpolation::MapEnvironment::new());
    let merged = merge_project(&loaded, Some(&interpolated));
    let project = merged.project().ok_or("project")?.clone();
    assert!(!project.root().is_sensitive());
    let input = ComposeSource::new(project, Identifier::new("deferred")?)?
        .with_interpolation_applied()
        .with_deferred_options(options()?);
    assert!(input.interpolation_applied());
    let result = ComposeImporter::new()?.import(&input);
    assert!(result.application().is_none());
    assert!(
        result
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFC0005")
    );
    Ok(())
}
