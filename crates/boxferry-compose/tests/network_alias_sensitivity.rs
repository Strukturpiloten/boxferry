//! Independent public importer contracts for scalar-specific network-alias sensitivity.

use std::error::Error;

use boxferry_compose::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeImporter,
    ComposeSource,
};
use boxferry_engine::{ConversionKind, ExportAdapter, ImportAdapter, ImportResult, LossPolicy, TargetProfile};
use boxferry_model::{HealthcheckCommand, Identifier, NetworkAttachment, ProvenanceKind, SourceId};
use compose_lens::{
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PRIVATE_ALIAS: &str = "protected-alias-canary-never-log";
const PRIVATE_HEALTH: &str = "protected-health-canary-never-log";
const BASE: &str = concat!(
    "---\nservices:\n  app.with.dot:\n    image: example.invalid/app:1\n",
    "    networks:\n      edge.with.dot:\n        aliases:\n",
    "          - public-literal\n          - ${PUBLIC_ALIAS}\n          - ${PRIVATE_ALIAS}\n",
    "        ipv4_address: 192.0.2.20\n",
    "    healthcheck:\n      test: [CMD-SHELL, \"printf ${PRIVATE_HEALTH}\"]\n",
    "networks:\n  edge.with.dot: {}\n",
);

fn environment() -> MapEnvironment {
    let mut values = MapEnvironment::new();
    let _ = values.insert("PUBLIC_ALIAS", "public-provider-alias");
    let _ = values.insert_sensitive("PRIVATE_ALIAS", PRIVATE_ALIAS);
    let _ = values.insert_sensitive("PRIVATE_HEALTH", PRIVATE_HEALTH);
    values
}

fn import(documents: &[(u32, &str, &str)], values: &MapEnvironment) -> TestResult<ImportResult> {
    let loaded = LoadedProject::load(documents.iter().map(|(id, name, text)| {
        DocumentInput::new(
            ComposeSourceId::new(*id),
            DocumentOrigin::new(*name, "/alias-sensitivity-offline"),
            *text,
        )
    }))?;
    // Native document interpolation accepts plain/double-quoted values; single quotes are literal.
    let interpolation = loaded.interpolate(values);
    assert!(interpolation.is_valid(), "explicit alias interpolation failed");
    let merged = merge_project(&loaded, Some(&interpolation));
    assert!(merged.is_valid(), "alias fixture merge failed");
    let mut source = ComposeSource::new(
        merged.project().ok_or("merged alias fixture missing")?.clone(),
        Identifier::new("alias-fixture")?,
    )?;
    for (id, name, _) in documents {
        source = source.with_source_id(ComposeSourceId::new(*id), SourceId::new(*name)?);
    }
    Ok(ComposeImporter::new()?.import(&source))
}

fn attachment(result: &ImportResult) -> TestResult<&NetworkAttachment> {
    let service = result
        .application()
        .ok_or("alias fixture application missing")?
        .services()
        .first()
        .ok_or("alias fixture service missing")?;
    Ok(service
        .value()
        .networks()
        .first()
        .ok_or("alias fixture attachment missing")?
        .value())
}

fn assert_origin(attachment: &NetworkAttachment, index: usize, document: &str, text: &str, scalar: &str) -> TestResult {
    let origins = attachment.alias_origins().get(index).ok_or("alias origins missing")?;
    assert_eq!(origins.len(), 1);
    let origin = &origins[0];
    assert_eq!(origin.kind(), ProvenanceKind::SourceDocument);
    assert_eq!(origin.source_id().as_str(), document);
    let span = origin.span().ok_or("alias source span missing")?;
    assert_eq!(text.get(span.start()..span.end()), Some(scalar));
    Ok(())
}

fn assert_privacy(result: &ImportResult) {
    let debug = format!("{result:?}");
    assert!(!debug.contains(PRIVATE_ALIAS), "protected alias leaked into Debug");
    assert!(
        !debug.contains(PRIVATE_HEALTH),
        "protected health command leaked into Debug"
    );
    let presentation = format!("{:?} {:?}", result.outcomes(), result.diagnostics());
    assert!(
        !presentation.contains(PRIVATE_ALIAS),
        "protected alias leaked into outcomes or diagnostics"
    );
    assert!(
        !presentation.contains(PRIVATE_HEALTH),
        "protected health command leaked into outcomes or diagnostics"
    );
}

#[test]
fn mixed_aliases_use_exact_scalar_flags_and_keep_unrelated_privacy() -> TestResult {
    let result = import(&[(41, "mixed.yaml", BASE)], &environment())?;
    let network = attachment(&result)?;
    assert_eq!(network.network().as_str(), "edge.with.dot");
    assert_eq!(network.alias_sensitivities(), [false, false, true]);
    assert!(
        network
            .aliases()
            .iter()
            .map(String::as_str)
            .eq(["public-literal", "public-provider-alias", PRIVATE_ALIAS]),
        "ordered alias values changed"
    );
    assert_origin(network, 0, "mixed.yaml", BASE, "public-literal")?;
    assert_origin(network, 1, "mixed.yaml", BASE, "${PUBLIC_ALIAS}")?;
    assert_origin(network, 2, "mixed.yaml", BASE, "${PRIVATE_ALIAS}")?;
    assert!(
        network
            .ipv4_address()
            .is_some_and(|address| address.value().is_sensitive()),
        "alias correction changed address protection"
    );
    let service = result.application().ok_or("application missing")?.services()[0].value();
    let command = service
        .healthcheck()
        .and_then(|health| health.value().command())
        .ok_or("health command missing")?;
    assert!(
        matches!(command.value(), HealthcheckCommand::Shell(text) if text.is_sensitive() && text.expose().contains(PRIVATE_HEALTH)),
        "alias correction changed health-command protection"
    );
    assert_privacy(&result);
    Ok(())
}

#[test]
fn equal_alias_values_do_not_merge_distinct_sensitivity_slots() -> TestResult {
    let text = "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      edge:\n        aliases: [same-value, \"${PRIVATE_ALIAS}\"]\nnetworks: {edge: {}}\n";
    let mut values = MapEnvironment::new();
    let _ = values.insert_sensitive("PRIVATE_ALIAS", "same-value");
    let result = import(&[(42, "duplicates.yaml", text)], &values)?;
    let network = attachment(&result)?;
    assert_eq!(network.alias_sensitivities(), [false, true]);
    assert!(network.aliases().iter().all(|alias| alias == "same-value"));
    assert_eq!(network.aliases().len(), 2);
    assert_origin(network, 0, "duplicates.yaml", text, "same-value")?;
    assert_origin(network, 1, "duplicates.yaml", text, "\"${PRIVATE_ALIAS}\"")?;
    let debug = format!("{network:?}");
    assert!(
        debug.contains("aliases: [\"same-value\", \"[REDACTED]\"]"),
        "Debug must retain the public alias and redact only the private slot"
    );
    assert_eq!(
        debug.matches("same-value").count(),
        1,
        "private duplicate leaked into Debug"
    );
    Ok(())
}

#[test]
fn aliases_use_provider_classification_not_variable_or_value_names() -> TestResult {
    let text = "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      edge:\n        aliases: [password-looking-literal, \"${PRIVATE_LOOKING_PUBLIC_INPUT}\", \"${ORDINARY}\"]\nnetworks: {edge: {}}\n";
    let mut values = MapEnvironment::new();
    let _ = values.insert("PRIVATE_LOOKING_PUBLIC_INPUT", "public-provider-alias");
    let _ = values.insert_sensitive("ORDINARY", PRIVATE_ALIAS);
    let result = import(&[(43, "classification.yaml", text)], &values)?;
    assert_eq!(attachment(&result)?.alias_sensitivities(), [false, false, true]);
    assert_privacy(&result);
    Ok(())
}

#[test]
fn appended_overlay_preserves_alias_order_flags_and_each_document_origin() -> TestResult {
    let base = "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      edge:\n        aliases: [base-public]\nnetworks: {edge: {}}\n";
    let overlay =
        "---\nservices:\n  app:\n    networks:\n      edge:\n        aliases: [\"${PRIVATE_ALIAS}\", overlay-public]\n";
    let result = import(
        &[(44, "base.yaml", base), (45, "overlay.yaml", overlay)],
        &environment(),
    )?;
    let network = attachment(&result)?;
    assert_eq!(network.alias_sensitivities(), [false, true, false]);
    assert!(
        network
            .aliases()
            .iter()
            .map(String::as_str)
            .eq(["base-public", PRIVATE_ALIAS, "overlay-public"]),
        "appended alias order changed"
    );
    assert_origin(network, 0, "base.yaml", base, "base-public")?;
    assert_origin(network, 1, "overlay.yaml", overlay, "\"${PRIVATE_ALIAS}\"")?;
    assert_origin(network, 2, "overlay.yaml", overlay, "overlay-public")?;
    assert_privacy(&result);
    Ok(())
}

#[test]
fn override_overlay_uses_replacement_scalar_flags_not_stale_base_sensitivity() -> TestResult {
    let base = "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      edge:\n        aliases: [\"${PRIVATE_ALIAS}\"]\nnetworks: {edge: {}}\n";
    let overlay = "---\nservices:\n  app:\n    networks:\n      edge:\n        aliases: !override [replacement-public, \"${PUBLIC_ALIAS}\"]\n";
    let result = import(
        &[(46, "base.yaml", base), (47, "override.yaml", overlay)],
        &environment(),
    )?;
    let network = attachment(&result)?;
    assert_eq!(network.alias_sensitivities(), [false, false]);
    assert_eq!(network.aliases(), ["replacement-public", "public-provider-alias"]);
    assert_origin(network, 0, "override.yaml", overlay, "replacement-public")?;
    assert_origin(network, 1, "override.yaml", overlay, "\"${PUBLIC_ALIAS}\"")?;
    assert_privacy(&result);
    Ok(())
}

fn target() -> TestResult<TargetProfile> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?)
}

#[test]
fn protected_imported_aliases_stay_neutral_but_refuse_every_compose_output_policy() -> TestResult {
    let imported = import(&[(48, "protected.yaml", BASE)], &environment())?;
    let application = imported.application().ok_or("application missing")?;
    let before = application.clone();
    for policy in [
        LossPolicy::ExactOnly,
        LossPolicy::AllowApproximate,
        LossPolicy::AllowPartial,
    ] {
        let plan = ComposeExporter::new()?.plan(application, &target()?)?;
        assert!(plan.candidate().is_none());
        let loss = plan
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == "services.app.with.dot.networks.edge.with.dot.aliases[2]")
            .ok_or("protected alias loss missing")?;
        assert_eq!(loss.kind(), ConversionKind::Unsupported);
        assert_eq!(loss.origins(), attachment(&imported)?.alias_origins()[2].as_slice());
        assert!(!format!("{plan:?}").contains(PRIVATE_ALIAS));
        assert!(!format!("{plan:?}").contains(PRIVATE_HEALTH));
        let result = plan.authorize(policy);
        assert!(result.is_blocked() && result.output().is_none());
        assert!(!format!("{:?} {:?}", result.outcomes(), result.diagnostics()).contains(PRIVATE_ALIAS));
    }
    assert!(
        *application == before,
        "export changed retained alias values or provenance"
    );
    assert_privacy(&imported);
    Ok(())
}

#[test]
fn ordinary_public_aliases_export_and_fresh_text_reimport_exactly() -> TestResult {
    let text = "---\nservices:\n  app:\n    image: example.invalid/app:1\n    networks:\n      edge:\n        aliases: [public-literal, \"${PUBLIC_ALIAS}\"]\nnetworks: {edge: {}}\n";
    let imported = import(&[(49, "public.yaml", text)], &environment())?;
    let plan = ComposeExporter::new()?.plan(imported.application().ok_or("application missing")?, &target()?)?;
    assert!(
        plan.outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("ordinary public output missing")?;
    // Reload physical text, never the generator's in-memory document/sensitivity flags.
    let fresh = import(&[(50, "fresh.yaml", output.text())], &MapEnvironment::new())?;
    let network = attachment(&fresh)?;
    assert_eq!(network.aliases(), ["public-literal", "public-provider-alias"]);
    assert_eq!(network.alias_sensitivities(), [false, false]);
    for (index, expected) in ["public-literal", "public-provider-alias"].iter().enumerate() {
        let origins = &network.alias_origins()[index];
        assert_eq!(origins.len(), 1);
        assert_eq!(origins[0].kind(), ProvenanceKind::SourceDocument);
        assert_eq!(origins[0].source_id().as_str(), "fresh.yaml");
        let span = origins[0].span().ok_or("fresh alias span missing")?;
        assert!(
            output
                .text()
                .get(span.start()..span.end())
                .is_some_and(|source| source.contains(expected)),
            "fresh alias origin is not its generated scalar"
        );
    }
    Ok(())
}
