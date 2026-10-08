//! Fault-injection controls for the private typed-view/native-metadata corroboration boundary.
//!
//! Production always uses one merged project. These tests deliberately pair a valid typed network
//! with other publicly constructed merged projects to prove mismatch cannot authorize disclosure.

use std::error::Error;

use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, TargetProfile};
use boxferry_model::{Application, Identifier, ImageReference, Network, ResourceOwnership, Service, SourceId, Sourced};
use compose_lens::{
    diagnostic::{DiagnosticCode, Severity},
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::{MergeProvenance, MergeResult, MergedProject, merge_project},
    model::{EXPECTED_SCALAR, EXPECTED_SEQUENCE, ServiceNetwork, ServiceNetworks},
    project::build_project_view,
    source::SourceId as ComposeSourceId,
};

use super::{ComposeImporter, Mapping, corroborated_alias_sensitivities};
use crate::{COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeSource};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BASE: &str = concat!(
    "---\nservices:\n  service.main:\n    image: example.invalid/app:1\n",
    "    networks:\n      edge.shared:\n        aliases:\n",
    "          - base-public\n          - other-public\n",
    "networks:\n  edge.shared: {}\n",
);

fn load_project(id: u32, text: &str) -> TestResult<MergeResult> {
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeSourceId::new(id),
        DocumentOrigin::new("alias-corroboration.yaml", "/alias-corroboration-offline"),
        text,
    )])?;
    Ok(merge_project(&loaded, None))
}

fn project(id: u32, text: &str) -> TestResult<MergedProject> {
    let merged = load_project(id, text)?;
    assert!(merged.is_valid(), "fault fixture merge failed");
    Ok(merged.project().ok_or("fault fixture project missing")?.clone())
}

/// Malformed aliases produce a recoverable native project with one deliberate model error.
/// Only these fault controls may bypass validity, after checking the exact expected diagnostic.
fn malformed_alias_project(
    id: u32,
    text: &str,
    expected_code: DiagnosticCode,
    expected_message: &str,
) -> TestResult<MergedProject> {
    let merged = load_project(id, text)?;
    let errors = merged
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.severity() == Severity::Error)
        .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
        .collect::<Vec<_>>();
    assert_eq!(errors, [(expected_code, expected_message)]);
    assert!(!merged.is_valid(), "malformed alias fixture unexpectedly valid");
    Ok(merged
        .project()
        .ok_or("malformed alias fixture project missing")?
        .clone())
}

fn typed_network(project: &MergedProject) -> TestResult<(ServiceNetwork, MergeProvenance)> {
    let result = build_project_view(project, None);
    let networks = result
        .view()
        .ok_or("typed fixture view missing")?
        .services()
        .first()
        .ok_or("typed fixture service missing")?
        .networks()
        .ok_or("typed fixture networks missing")?;
    let ServiceNetworks::Long {
        networks: attachments, ..
    } = networks.value()
    else {
        return Err("typed fixture requires long network syntax".into());
    };
    Ok((
        attachments.first().ok_or("typed fixture attachment missing")?.clone(),
        networks.provenance().clone(),
    ))
}

#[test]
fn exact_native_sequence_corroborates_public_aliases_and_empty_alias_sets() -> TestResult {
    let original = project(51, BASE)?;
    let (network, _) = typed_network(&original)?;
    assert_eq!(
        corroborated_alias_sensitivities(&original, "service.main", &network),
        Some(vec![false, false])
    );
    let empty_text = BASE.replace(
        "        aliases:\n          - base-public\n          - other-public\n",
        "        aliases: []\n",
    );
    let empty = project(51, &empty_text)?;
    let (network, _) = typed_network(&empty)?;
    assert_eq!(
        corroborated_alias_sensitivities(&empty, "service.main", &network),
        Some(vec![])
    );
    Ok(())
}

#[test]
fn malformed_or_mismatched_metadata_protects_the_entire_alias_set_without_dropping_values() -> TestResult {
    let original = project(51, BASE)?;
    let (network, provenance) = typed_network(&original)?;
    let cases = vec![
        // Exact path, not a scan for an equal alias under another service/network.
        (51, BASE.replace("service.main", "other.main")),
        (51, BASE.replace("edge.shared", "edge.other")),
        (
            51,
            BASE.replace(
                "        aliases:\n          - base-public\n          - other-public\n",
                "",
            ),
        ),
        // A matching first item is insufficient when a later item/length disagrees.
        (
            51,
            BASE.replace("- other-public", "- other-public\n          - extra-public"),
        ),
        (51, BASE.replace("other-public", "wrong-public")),
        // Correct values with different original offsets or source identity are still mismatched.
        (51, format!("# shifted offsets\n{BASE}")),
        (52, BASE.to_owned()),
    ];
    let reordered = BASE
        .replace("base-public", "swap-public")
        .replace("other-public", "base-public")
        .replace("swap-public", "other-public");
    let mut metadata_cases = cases
        .into_iter()
        .map(|(id, text)| project(id, &text))
        .collect::<TestResult<Vec<_>>>()?;
    metadata_cases.push(project(51, &reordered)?);
    // Invalid native shapes can never fall back to aggregate public sensitivity. Require their
    // deliberate native model errors, rather than weakening validity checks for the other cases.
    for value in ["null", "{one: base-public, two: other-public}"] {
        let text = BASE.replace(
            "aliases:\n          - base-public\n          - other-public",
            &format!("aliases: {value}"),
        );
        metadata_cases.push(malformed_alias_project(
            51,
            &text,
            EXPECTED_SEQUENCE,
            "network aliases must be a sequence",
        )?);
    }
    metadata_cases.push(malformed_alias_project(
        51,
        &BASE.replace("- other-public", "- {broken: other-public}"),
        EXPECTED_SCALAR,
        "network aliases entries must be scalars",
    )?);
    for metadata in metadata_cases {
        assert!(
            corroborated_alias_sensitivities(&metadata, "service.main", &network).is_none(),
            "mismatched native tuple was accepted"
        );
        let source = ComposeSource::new(metadata, Identifier::new("alias-fault-fixture")?)?
            .with_source_id(ComposeSourceId::new(51), SourceId::new("original.yaml")?);
        let importer = ComposeImporter::new()?;
        let mut mapping = Mapping::new(&importer.codes, &source);
        let mut service = Service::new(Identifier::new("service.main")?);
        service.set_image(Sourced::generated(ImageReference::parse("example.invalid/app:1")?));
        mapping.map_service_network(
            "services.service.main",
            "service.main",
            &network,
            &provenance,
            &mut service,
        );
        let attachment = service
            .networks()
            .first()
            .ok_or("fault mapping dropped the attachment")?
            .value();
        assert_eq!(attachment.alias_sensitivities(), [true, true]);
        assert!(
            attachment
                .aliases()
                .iter()
                .map(String::as_str)
                .eq(["base-public", "other-public"]),
            "fault mapping dropped or replaced typed aliases"
        );
        let debug = format!("{service:?} {:?} {:?}", mapping.outcomes, mapping.diagnostics);
        assert!(
            !debug.contains("base-public") && !debug.contains("other-public"),
            "uncorroborated alias escaped default redaction"
        );
        assert_uncorroborated_aliases_refuse_export(service)?;
    }
    Ok(())
}

fn assert_uncorroborated_aliases_refuse_export(service: Service) -> TestResult {
    let mut application = Application::new(Identifier::new("uncorroborated-aliases")?);
    application.add_service(Sourced::generated(service))?;
    application.add_network(Sourced::generated(Network::new(
        Identifier::new("edge.shared")?,
        ResourceOwnership::Application,
    )))?;
    let target = TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?;
    for policy in [
        LossPolicy::ExactOnly,
        LossPolicy::AllowApproximate,
        LossPolicy::AllowPartial,
    ] {
        let plan = ComposeExporter::new()?.plan(&application, &target)?;
        assert!(plan.candidate().is_none());
        assert!(plan.outcomes().iter().any(|outcome| outcome.subject()
            == "services.service.main.networks.edge.shared.aliases[0]"
            && outcome.kind() == ConversionKind::Unsupported));
        let result = plan.authorize(policy);
        assert!(result.is_blocked() && result.output().is_none());
        assert!(!format!("{result:?}").contains("base-public"));
        assert!(!format!("{result:?}").contains("other-public"));
    }
    Ok(())
}
