//! Podman target resolution and reviewable export contracts.

use std::error::Error;

use boxferry_engine::{ExportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile};
use boxferry_model::{
    Application, Identifier, ImageReference, Network, NetworkAttachment, ProtectedString, ResourceOwnership, Service,
    ServiceGroup, Sourced,
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
    let plan = exporter.plan(&minimal_application()?, &target)?;

    assert!(plan.candidate().is_none());
    assert!(
        plan.diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.code().as_str() == "BFP0006" && diagnostic.severity() == Severity::Error })
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
    assert!(
        plan.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFP0007")
    );
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

fn tmpfs_application(value: ProtectedString) -> Result<Application, Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.set_tmpfs(vec![Sourced::generated(value)]);
    let mut application = Application::new(Identifier::new("tmpfs-test")?);
    application.add_service(Sourced::generated(service))?;
    Ok(application)
}

const fn version(major: u64, minor: u64, patch: u64) -> PlatformVersion {
    PlatformVersion::new(major, minor, patch)
}
