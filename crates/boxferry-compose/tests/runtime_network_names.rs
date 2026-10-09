//! Independent neutral-to-Compose contracts for observed network identities.

use std::error::Error;

use boxferry_compose::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeImporter,
    ComposeSource, DOCKER_COMPOSE_TARGET,
};
use boxferry_engine::{ConversionKind, ExportAdapter, ImportAdapter, LossPolicy, PlatformVersion, TargetProfile};
use boxferry_model::{
    Application, Identifier, ImageReference, Network, NetworkAttachment, ProtectedString, Provenance, ProvenanceKind,
    ResourceOwnership, Service, SourceId, Sourced,
};
use compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const OWNERS: [ResourceOwnership; 4] = [
    ResourceOwnership::Application,
    ResourceOwnership::External,
    ResourceOwnership::Implicit,
    ResourceOwnership::Uncertain,
];

fn target() -> TestResult<TargetProfile> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?)
}

fn application(ownership: ResourceOwnership, name: Option<ProtectedString>, observed: bool) -> TestResult<Application> {
    let id = SourceId::new("network-evidence")?;
    let origin = if observed {
        Provenance::runtime_observation(id)
    } else {
        Provenance::source(id)
    };
    let mut network = Network::new(Identifier::new("edge")?, ownership);
    if let Some(name) = name {
        network.set_runtime_name(Sourced::from_source(name, name_origin()?));
    }
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.add_network(Sourced::generated(NetworkAttachment::new(
        Identifier::new("edge")?,
        Vec::new(),
    )));
    let mut application = Application::new(Identifier::new("runtime-network")?);
    application.add_service(Sourced::generated(service))?;
    application.add_network(Sourced::from_source(network, origin))?;
    Ok(application)
}

fn name_origin() -> TestResult<Provenance> {
    Ok(Provenance::runtime_observation(SourceId::new(
        "explicit-name-evidence",
    )?))
}

fn reimport(text: &str) -> TestResult<Application> {
    let id = ComposeSourceId::new(436);
    let loaded = LoadedProject::load([DocumentInput::new(
        id,
        DocumentOrigin::new("generated.compose.yaml", "/runtime-network-offline"),
        text,
    )])?;
    assert!(loaded.is_valid(), "{:?}", loaded.diagnostics());
    let merged = merge_project(&loaded, None);
    assert!(merged.is_valid(), "{:?}", merged.diagnostics());
    let source = ComposeSource::new(
        merged.project().ok_or("merged project")?.clone(),
        Identifier::new("fallback")?,
    )?
    .with_source_id(id, SourceId::new("generated.compose.yaml")?);
    let imported = ComposeImporter::new()?.import(&source);
    assert!(
        imported
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(imported.application().ok_or("reimported application")?.clone())
}

fn assert_reimport(text: &str, ownership: ResourceOwnership, runtime_name: Option<&str>) -> TestResult {
    let imported = reimport(text)?;
    assert_eq!(imported.networks().len(), 1);
    let network = imported.networks()[0].value();
    assert_eq!(network.name().as_str(), "edge");
    assert_eq!(
        network.ownership(),
        if ownership == ResourceOwnership::Application {
            ResourceOwnership::Application
        } else {
            ResourceOwnership::External
        }
    );
    assert_eq!(network.runtime_name().map(|name| name.value().expose()), runtime_name);
    if let Some(name) = network.runtime_name() {
        assert!(!name.value().is_sensitive());
        assert!(
            name.origins()
                .iter()
                .all(|origin| origin.kind() == ProvenanceKind::SourceDocument)
        );
    }
    assert_eq!(
        imported.services()[0].value().networks()[0].value().network().as_str(),
        "edge"
    );
    Ok(())
}

#[test]
fn explicit_observed_network_names_take_precedence_once() -> TestResult {
    let docker_version = PlatformVersion::new(2, 30, 0);
    for profile in [
        target()?,
        TargetProfile::new(DOCKER_COMPOSE_TARGET, docker_version, Some(docker_version))?,
    ] {
        for ownership in OWNERS {
            for native_name in ["edge", "platform-edge"] {
                let application = application(ownership, Some(ProtectedString::plain(native_name)), true)?;
                let plan = ComposeExporter::new()?.plan(&application, &profile)?;
                assert!(plan.candidate().is_some(), "{ownership:?}: {:?}", plan.diagnostics());
                assert!(
                    plan.diagnostics()
                        .iter()
                        .all(|diagnostic| diagnostic.code().as_str() != "BFC0008")
                );
                let name_decisions: Vec<_> = plan
                    .outcomes()
                    .iter()
                    .filter(|outcome| outcome.subject() == "networks.edge.name")
                    .collect();
                assert_eq!(name_decisions.len(), 1);
                assert_eq!(name_decisions[0].kind(), ConversionKind::Exact);
                assert_eq!(name_decisions[0].origins(), [name_origin()?]);
                let lifecycle: Vec<_> = plan
                    .outcomes()
                    .iter()
                    .filter(|outcome| outcome.subject() == "networks.edge")
                    .collect();
                match ownership {
                    ResourceOwnership::Application => assert!(lifecycle.is_empty()),
                    ResourceOwnership::External => {
                        assert_eq!(lifecycle.len(), 1);
                        assert_eq!(lifecycle[0].kind(), ConversionKind::Exact);
                    }
                    _ => {
                        assert_eq!(lifecycle.len(), 1);
                        assert_eq!(lifecycle[0].kind(), ConversionKind::Unsupported);
                        assert_eq!(
                            lifecycle[0].diagnostic().map(boxferry_engine::DiagnosticCode::as_str),
                            Some("BFC0007")
                        );
                        assert_eq!(lifecycle[0].origins(), application.networks()[0].origins());
                        assert!(plan.clone().authorize(LossPolicy::ExactOnly).output().is_none());
                    }
                }
                let policy = if matches!(ownership, ResourceOwnership::Application | ResourceOwnership::External) {
                    LossPolicy::ExactOnly
                } else {
                    LossPolicy::AllowPartial
                };
                let authorized = plan.authorize(policy);
                let document = authorized.output().ok_or("network candidate")?;
                assert_eq!(
                    document.text().matches(&format!("    name: {native_name}\n")).count(),
                    1
                );
                assert_reimport(document.text(), ownership, Some(native_name))?;
            }
        }
    }
    Ok(())
}

#[test]
fn observed_network_fallback_requires_absent_runtime_name_and_external_lifecycle() -> TestResult {
    for ownership in OWNERS {
        for observed in [true, false] {
            let application = application(ownership, None, observed)?;
            let plan = ComposeExporter::new()?.plan(&application, &target()?)?;
            assert!(plan.candidate().is_some(), "{:?}", plan.diagnostics());
            assert!(
                plan.outcomes()
                    .iter()
                    .all(|outcome| outcome.subject() != "networks.edge.name")
            );
            let lifecycle: Vec<_> = plan
                .outcomes()
                .iter()
                .filter(|outcome| outcome.subject() == "networks.edge")
                .collect();
            let expected_decisions = match ownership {
                ResourceOwnership::Application => Vec::new(),
                ResourceOwnership::External => vec![ConversionKind::Exact],
                _ => vec![ConversionKind::Unsupported],
            };
            assert_eq!(
                lifecycle.iter().map(|outcome| outcome.kind()).collect::<Vec<_>>(),
                expected_decisions
            );
            let authorized = plan.authorize(LossPolicy::AllowPartial);
            let document = authorized.output().ok_or("fallback candidate")?;
            let expected_name = (observed && ownership != ResourceOwnership::Application).then_some("edge");
            assert_eq!(document.text().contains("    name: edge\n"), expected_name.is_some());
            assert_reimport(document.text(), ownership, expected_name)?;
        }
    }
    Ok(())
}

#[test]
fn sensitive_runtime_network_names_are_refused_without_fallback_or_disclosure() -> TestResult {
    const PRIVATE_NAME: &str = "private-network-name-canary";
    for ownership in OWNERS {
        let application = application(ownership, Some(ProtectedString::sensitive(PRIVATE_NAME)), true)?;
        let plan = ComposeExporter::new()?.plan(&application, &target()?)?;
        let decisions: Vec<_> = plan
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == "networks.edge.name")
            .collect();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].kind(), ConversionKind::Unsupported);
        assert_eq!(
            decisions[0].diagnostic().map(boxferry_engine::DiagnosticCode::as_str),
            Some("BFC0007")
        );
        assert_eq!(decisions[0].origins(), [name_origin()?]);
        assert!(
            plan.diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code().as_str() != "BFC0008")
        );
        assert!(!format!("{plan:?}").contains(PRIVATE_NAME));
        assert!(plan.clone().authorize(LossPolicy::ExactOnly).output().is_none());
        let authorized = plan.authorize(LossPolicy::AllowPartial);
        let document = authorized.output().ok_or("existing partial candidate behavior")?;
        assert!(!document.text().contains(PRIVATE_NAME));
        assert!(!document.text().contains("    name:"));
        assert_reimport(document.text(), ownership, None)?;
        let retained = application.networks()[0]
            .value()
            .runtime_name()
            .ok_or("retained private name")?;
        assert!(retained.value().is_sensitive());
        assert_eq!(retained.value().expose(), PRIVATE_NAME);
    }
    Ok(())
}

#[test]
fn invalid_explicit_runtime_network_names_still_fail_generation() -> TestResult {
    for ownership in OWNERS {
        for invalid_name in ["", "invalid\0name"] {
            let application = application(ownership, Some(ProtectedString::plain(invalid_name)), true)?;
            let plan = ComposeExporter::new()?.plan(&application, &target()?)?;
            let expected_origin = name_origin()?;
            assert!(plan.candidate().is_none());
            assert!(plan.outcomes().iter().any(|outcome| {
                outcome.subject() == "networks.edge.name"
                    && outcome.kind() == ConversionKind::Invalid
                    && outcome.diagnostic().is_some_and(|code| code.as_str() == "BFC0008")
                    && outcome.origins() == [expected_origin.clone()]
            }));
            assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());
        }
    }
    Ok(())
}
