//! Podman target resolution and reviewable export contracts.

use std::{collections::BTreeSet, error::Error};

use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile};
use boxferry_model::{
    Application, EnvironmentValue, EnvironmentVariable, Identifier, ImageReference, Mount, MountSource, Network,
    NetworkAttachment, NetworkIpamConfig, ProtectedString, Provenance, ResourceOwnership, RetainedNativeEvidence,
    RetainedNativeEvidenceEvent, RetainedNativeEvidenceSubject, SelinuxRelabel, Service, ServiceGroup, SourceId,
    SourceSpan, Sourced,
};
use boxferry_podman::{
    PODMAN_TARGET, PodmanExporter, PodmanTargetError, resolve_podman_target, reviewed_podman_versions,
};
use podman_lens::TargetExecutionContext;

#[test]
fn reviewed_catalogue_and_ceiling_resolution_are_exact() -> Result<(), Box<dyn Error>> {
    let reviewed = [
        version(5, 4, 0),
        version(5, 5, 0),
        version(5, 6, 0),
        version(5, 7, 0),
        version(5, 8, 6),
        version(6, 0, 0),
        version(6, 1, 0),
    ];
    assert_eq!(reviewed_podman_versions(), reviewed.as_slice());

    for exact in reviewed {
        let target = TargetProfile::new(PODMAN_TARGET, exact, Some(exact))?;
        assert_eq!(resolve_podman_target(&target)?.version(), exact);
    }

    let unbounded = TargetProfile::new(PODMAN_TARGET, version(5, 4, 0), None)?;
    assert_eq!(resolve_podman_target(&unbounded)?.version(), version(6, 1, 0));

    let between_reviewed_patches = TargetProfile::new(PODMAN_TARGET, version(5, 4, 0), Some(version(5, 8, 0)))?;
    assert_eq!(
        resolve_podman_target(&between_reviewed_patches)?.version(),
        version(5, 7, 0)
    );
    Ok(())
}

#[test]
fn target_resolution_fails_closed_outside_the_reviewed_catalogue() -> Result<(), Box<dyn Error>> {
    let too_old = TargetProfile::new(PODMAN_TARGET, version(5, 3, 0), Some(version(5, 3, 99)))?;
    assert_eq!(
        resolve_podman_target(&too_old),
        Err(PodmanTargetError::NoReviewedVersion)
    );

    let unrelated = TargetProfile::new("compose", version(5, 4, 0), None)?;
    assert_eq!(
        resolve_podman_target(&unrelated),
        Err(PodmanTargetError::ImplementationMismatch)
    );
    Ok(())
}

#[test]
fn caller_selected_execution_context_is_preserved_without_inference() -> Result<(), Box<dyn Error>> {
    let default = PodmanExporter::new()?;
    assert_eq!(default.execution_context(), TargetExecutionContext::Unknown);

    for context in [
        TargetExecutionContext::Unknown,
        TargetExecutionContext::Rootless,
        TargetExecutionContext::Rootful,
    ] {
        assert_eq!(
            PodmanExporter::new()?
                .with_execution_context(context)
                .execution_context(),
            context
        );
    }
    Ok(())
}

#[test]
fn invalid_targets_produce_bfp0006_and_no_candidate() -> Result<(), Box<dyn Error>> {
    let exporter = PodmanExporter::new()?;
    let target = TargetProfile::new(PODMAN_TARGET, version(5, 3, 0), Some(version(5, 3, 99)))?;
    let mut application = minimal_application()?;
    let source = SourceId::new("invalid-target.container")?;
    let event_origin = Provenance::spanned(source.clone(), SourceSpan::new(0, 9)?);
    let segment_origin = Provenance::spanned(source, SourceSpan::new(12, 36)?);
    application.add_retained_native_evidence(RetainedNativeEvidence::new(
        RetainedNativeEvidenceSubject::QuadletServicePodmanArgs(Identifier::new("web")?),
        Sourced::from_source(
            RetainedNativeEvidenceEvent::Value(vec![Sourced::from_source(
                ProtectedString::sensitive("private-invalid-target-evidence"),
                segment_origin.clone(),
            )]),
            event_origin.clone(),
        ),
    )?)?;
    let plan = exporter.plan(&application, &target)?;

    assert!(plan.candidate().is_none());
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "target.podman" && outcome.kind() == ConversionKind::Invalid })
    );
    let evidence_outcome = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "services.web.podman_args")
        .ok_or("missing retained-evidence loss on invalid target")?;
    assert_eq!(evidence_outcome.kind(), ConversionKind::Unsupported);
    assert_eq!(evidence_outcome.origins(), &[event_origin, segment_origin]);
    assert!(plan.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0007"
            && diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "subject" && field.value().redacted() == "services.web.podman_args")
    }));
    assert!(!format!("{plan:?}").contains("private-invalid-target-evidence"));
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFP0006" && diagnostic.severity() == Severity::Error)
        .ok_or("missing invalid Podman target diagnostic")?;
    assert_context_fields(
        diagnostic,
        &[
            "subject",
            "reason",
            "decision",
            "requested_minimum",
            "requested_maximum",
            "reviewed_targets",
        ],
    );
    assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());
    Ok(())
}

#[test]
fn podman_artifacts_are_deterministic_reviewable_and_semantically_versioned() -> Result<(), Box<dyn Error>> {
    let application = minimal_application()?;
    let target = TargetProfile::new(PODMAN_TARGET, version(5, 4, 0), None)?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);

    let first = exporter.plan(&application, &target)?.authorize(LossPolicy::ExactOnly);
    let second = exporter.plan(&application, &target)?.authorize(LossPolicy::ExactOnly);
    let first = first.output().ok_or("exact Podman output expected")?;
    let second = second.output().ok_or("repeated Podman output expected")?;

    assert_eq!(first, second);
    assert_eq!(first.target_version(), version(6, 1, 0));
    assert!(first.commands_shell().starts_with("#!/bin/sh\n"));
    assert!(first.commands_shell().contains("podman 'container' 'create'"));
    assert!(!first.commands_shell().contains("curl "));
    assert!(!format!("{first:?}").contains("example.invalid/web:1"));

    let deployment: serde_json::Value = serde_json::from_str(first.deployment_json())?;
    assert_eq!(deployment["schema_version"], 1);
    assert_eq!(deployment["status"], "exact");
    assert!(deployment["connection"].is_null());
    let operations = deployment["operations"]
        .as_array()
        .ok_or("deployment operations array expected")?;
    assert_eq!(operations.len(), 3);
    assert_eq!(operations[0]["action"], "ensure_image");
    assert_eq!(operations[1]["action"], "create");
    assert_eq!(operations[2]["action"], "start_container");
    assert!(operations.iter().all(|operation| {
        operation["libpod"]["path_and_query"]
            .as_str()
            .is_some_and(|path| path.starts_with("/v6.1.0/libpod/"))
    }));
    Ok(())
}

#[test]
fn uncertain_runtime_group_is_omitted_and_partial_podman_output_remains_authorizable() -> Result<(), Box<dyn Error>> {
    let mut application = minimal_application()?;
    let mut group = ServiceGroup::new(Identifier::new("runtime-pod")?, ResourceOwnership::Uncertain);
    group.add_member(Sourced::generated(Identifier::new("web")?))?;
    application.add_service_group(Sourced::generated(group))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);
    let plan = exporter.plan(&application, &target)?;

    assert!(plan.candidate().is_some(), "{:?}", plan.diagnostics());
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFP0007")
        .ok_or("missing unsupported Podman output diagnostic")?;
    assert_context_fields(diagnostic, &["subject", "reason", "decision", "required_loss_policy"]);
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial Podman output expected")?;
    assert!(!output.commands_shell().contains(" 'pod' "));
    assert!(output.commands_shell().contains("podman 'container' 'create'"));
    Ok(())
}

fn minimal_application() -> Result<Application, Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    let mut application = Application::new(Identifier::new("adapter-test")?);
    application.add_service(Sourced::generated(service))?;
    Ok(application)
}
#[test]
fn tmpfs_destination_is_a_semantic_podman_mount() -> Result<(), Box<dyn Error>> {
    let application = tmpfs_application(ProtectedString::plain("/run/cache"))?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);
    let result = exporter.plan(&application, &target)?.authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("exact tmpfs Podman output expected")?;
    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let operation = deployment["operations"]
        .as_array()
        .and_then(|operations| {
            operations
                .iter()
                .find(|operation| operation["action"] == "create" && operation["resource"]["kind"] == "container")
        })
        .ok_or("container create operation expected")?;
    let argv = operation["cli"]["argv"]
        .as_array()
        .ok_or("container CLI argv expected")?;
    assert!(
        argv.windows(2)
            .any(|arguments| { arguments[0] == "--mount" && arguments[1] == "type=tmpfs,target=/run/cache" })
    );
    assert_eq!(operation["libpod"]["body"]["json"]["mounts"][0]["type"], "tmpfs");
    assert_eq!(
        operation["libpod"]["body"]["json"]["mounts"][0]["destination"],
        "/run/cache"
    );
    Ok(())
}

#[test]
fn tmpfs_options_preserve_only_safe_destination_under_partial_policy() -> Result<(), Box<dyn Error>> {
    let application = tmpfs_application(ProtectedString::plain("/run/cache:mode=1777,uid=1000"))?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);
    let plan = exporter.plan(&application, &target)?;

    assert!(plan.candidate().is_some());
    assert!(
        plan.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFP0007")
    );
    let result = plan.authorize(LossPolicy::AllowPartial);
    let output = result.output().ok_or("partial tmpfs Podman output expected")?;
    assert!(output.commands_shell().contains("type=tmpfs,target=/run/cache"));
    assert!(!output.commands_shell().contains("mode=1777"));
    assert!(!output.deployment_json().contains("uid=1000"));
    Ok(())
}

#[test]
fn sensitive_or_malformed_tmpfs_destination_fails_without_leaking() -> Result<(), Box<dyn Error>> {
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);
    let sensitive = "never-print-private-tmpfs";
    let sensitive_application = tmpfs_application(ProtectedString::sensitive(sensitive))?;
    let sensitive_plan = exporter.plan(&sensitive_application, &target)?;

    assert!(sensitive_plan.candidate().is_none());
    assert!(
        sensitive_plan
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFP0008")
    );
    assert!(!format!("{sensitive_plan:?}").contains(sensitive));

    let malformed_application = tmpfs_application(ProtectedString::plain("relative/path"))?;
    let malformed_plan = exporter.plan(&malformed_application, &target)?;
    assert!(malformed_plan.candidate().is_none());
    assert!(
        malformed_plan
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFP0008")
    );
    Ok(())
}

#[test]
fn invalid_network_alias_reports_its_exact_mapping_subject() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("alias-test")?);
    application.add_network(Sourced::generated(Network::new(
        Identifier::new("frontend")?,
        ResourceOwnership::Application,
    )))?;
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.add_network(Sourced::generated(NetworkAttachment::new(
        Identifier::new("frontend")?,
        vec![Sourced::generated(ProtectedString::plain("a".repeat(254)))],
    )));
    application.add_service(Sourced::generated(service))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    assert!(plan.candidate().is_none());
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFP0008")
        .ok_or("missing Podman mapping diagnostic")?;
    assert_context_fields(diagnostic, &["subject", "reason", "decision"]);
    assert!(
        diagnostic.fields().iter().any(|field| {
            field.name() == "subject" && field.value().expose() == "services.web.networks[0].aliases[0]"
        })
    );
    Ok(())
}

#[test]
fn local_image_portability_failure_reports_resource_and_field() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("localhost/example:1")?));
    let mut application = Application::new(Identifier::new("local-image-test")?);
    application.add_service(Sourced::generated(service))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    assert!(plan.candidate().is_none());
    let finding = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic
                .native_finding()
                .is_some_and(|finding| finding.code() == "PLN0048")
        })
        .ok_or("missing local image portability diagnostic")?;
    assert!(
        finding
            .fields()
            .iter()
            .any(|field| { field.name() == "resource_name" && field.value().expose() == "web-image" })
    );
    assert!(
        finding
            .fields()
            .iter()
            .any(|field| { field.name() == "intent_field" && field.value().expose() == "source.portability" })
    );
    Ok(())
}

#[test]
fn neutral_network_losses_are_reported_per_present_field() -> Result<(), Box<dyn Error>> {
    let mut application = minimal_application()?;
    let mut network = Network::new(Identifier::new("front")?, ResourceOwnership::Application);
    network.set_runtime_name(Sourced::generated(ProtectedString::plain("runtime-front")));
    network.set_driver(Sourced::generated(ProtectedString::plain("bridge")));
    network.set_driver_options(Vec::new());
    network.set_labels(Vec::new());
    network.set_internal(Sourced::generated(true));
    network.set_ipv6(Sourced::generated(true));
    network.set_ipam_driver(Sourced::generated(ProtectedString::plain("host-local")));
    network.add_ipam_config(Sourced::generated(NetworkIpamConfig::new(Sourced::generated(
        ProtectedString::plain("10.88.0.0/16"),
    ))?));
    application.add_network(Sourced::generated(network))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    let expected = BTreeSet::from([
        "networks.front.driver".to_owned(),
        "networks.front.internal".to_owned(),
        "networks.front.ipam_configs".to_owned(),
        "networks.front.ipam_driver".to_owned(),
        "networks.front.ipv6".to_owned(),
        "networks.front.runtime_name".to_owned(),
    ]);
    let actual = plan
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFP0007")
        .filter_map(|diagnostic| {
            diagnostic.fields().iter().find_map(|field| {
                (field.name() == "subject" && field.value().redacted().starts_with("networks.front."))
                    .then(|| field.value().redacted().to_owned())
            })
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    assert!(!actual.contains("networks.front.settings"));
    for diagnostic in plan.diagnostics().iter().filter(|diagnostic| {
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "subject" && expected.contains(field.value().redacted()))
    }) {
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|field| { field.name() == "decision" && field.value().redacted() == "omitted" })
        );
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|field| { field.name() == "available_promotion" && field.value().redacted() == "none" })
        );
        assert!(diagnostic.fields().iter().any(|field| {
            field.name() == "remediation" && field.value().redacted().contains("no automatic promotion exists")
        }));
    }
    assert!(plan.authorize(LossPolicy::AllowPartial).output().is_some());
    Ok(())
}

#[test]
fn host_bind_and_selinux_losses_name_exact_fields() -> Result<(), Box<dyn Error>> {
    const PRIVATE_SOURCE: &str = "/private/source/never-render";
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    let mut mount = Mount::new(MountSource::HostPath(PRIVATE_SOURCE.to_owned()), "/etc/example", true)?;
    mount.set_selinux_relabel(SelinuxRelabel::Private);
    service.add_mount(Sourced::generated(mount));
    let mut application = Application::new(Identifier::new("mount-fields")?);
    application.add_service(Sourced::generated(service))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    let expected = BTreeSet::from([
        "services.web.mounts[0].selinux_relabel".to_owned(),
        "services.web.mounts[0].source".to_owned(),
    ]);
    let actual = plan
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFP0007")
        .filter_map(|diagnostic| {
            diagnostic.fields().iter().find_map(|field| {
                (field.name() == "subject" && field.value().redacted().starts_with("services.web.mounts[0]."))
                    .then(|| field.value().redacted().to_owned())
            })
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    for diagnostic in plan.diagnostics().iter().filter(|diagnostic| {
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "subject" && expected.contains(field.value().redacted()))
    }) {
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|field| { field.name() == "decision" && field.value().redacted() == "omitted" })
        );
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|field| { field.name() == "available_promotion" && field.value().redacted() == "none" })
        );
        assert!(diagnostic.fields().iter().any(|field| {
            field.name() == "remediation" && field.value().redacted().contains("no automatic promotion exists")
        }));
    }
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial Podman output")?;
    assert!(!output.deployment_json().contains(PRIVATE_SOURCE));
    assert!(!output.commands_shell().contains(PRIVATE_SOURCE));
    Ok(())
}

#[test]
fn generated_podman_environment_is_key_sorted() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    for (name, value) in [("ZETA", "z"), ("ALPHA", "a"), ("MIDDLE", "m")] {
        service.add_environment(Sourced::generated(EnvironmentVariable::new(
            Identifier::new(name)?,
            EnvironmentValue::Literal(ProtectedString::plain(value)),
        )));
    }
    let mut application = Application::new(Identifier::new("environment-order")?);
    application.add_service(Sourced::generated(service))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let result = PodmanExporter::new()?
        .plan(&application, &target)?
        .authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("exact Podman output")?;
    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let argv = deployment["operations"]
        .as_array()
        .and_then(|operations| {
            operations.iter().find_map(|operation| {
                (operation["action"] == "create" && operation["resource"]["kind"] == "container")
                    .then(|| operation["cli"]["argv"].as_array())
                    .flatten()
            })
        })
        .ok_or("container create argv")?;
    let assignments = argv
        .windows(2)
        .filter_map(|arguments| {
            (arguments[0] == "--env")
                .then(|| arguments[1].as_str().map(ToOwned::to_owned))
                .flatten()
        })
        .collect::<Vec<_>>();

    assert_eq!(assignments, ["ALPHA=a", "MIDDLE=m", "ZETA=z"]);
    Ok(())
}

fn tmpfs_application(value: ProtectedString) -> Result<Application, Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.set_tmpfs(vec![Sourced::generated(value)]);
    let mut application = Application::new(Identifier::new("tmpfs-test")?);
    application.add_service(Sourced::generated(service))?;
    Ok(application)
}

fn assert_context_fields(diagnostic: &boxferry_engine::Diagnostic, expected: &[&str]) {
    for name in expected {
        assert!(
            diagnostic.fields().iter().any(|field| field.name() == *name),
            "{} omitted {name}: {diagnostic:?}",
            diagnostic.code().as_str()
        );
    }
}

const fn version(major: u64, minor: u64, patch: u64) -> PlatformVersion {
    PlatformVersion::new(major, minor, patch)
}
