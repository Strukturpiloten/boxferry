//! Podman target resolution and reviewable export contracts.

use std::{collections::BTreeSet, error::Error};

use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy, PlatformVersion, Severity, TargetProfile};
use boxferry_model::{
    Application, EnvironmentValue, EnvironmentVariable, Identifier, ImageReference, MetadataLabel, Mount, MountSource,
    Network, NetworkAttachment, NetworkIpamConfig, ProtectedString, Provenance, ResourceOwnership,
    RetainedNativeEvidence, RetainedNativeEvidenceEvent, RetainedNativeEvidenceSubject, SelinuxRelabel, Service,
    ServiceDependency, ServiceGroup, SourceId, SourceSpan, Sourced, Volume,
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

#[test]
fn volume_runtime_names_map_to_exact_declarations_and_mounts_across_services() -> Result<(), Box<dyn Error>> {
    let application = volume_mapping_application(&[("data", Some("runtime-data")), ("cache", None)])?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("exact volume output expected")?;

    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let operations = deployment["operations"]
        .as_array()
        .ok_or("deployment operations array expected")?;
    assert_eq!(operations.len(), 8);
    assert!(operations.iter().all(|operation| operation["status"] == "exact"));
    let operation_roster = operations
        .iter()
        .map(|op| {
            Ok((
                op["action"].as_str().ok_or("operation action expected")?.to_owned(),
                op["resource"]["kind"]
                    .as_str()
                    .ok_or("resource kind expected")?
                    .to_owned(),
                op["resource"]["name"]
                    .as_str()
                    .ok_or("resource name expected")?
                    .to_owned(),
            ))
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    assert_eq!(
        operation_roster,
        BTreeSet::from([
            ("ensure_image".to_owned(), "image".to_owned(), "web-image".to_owned()),
            ("ensure_image".to_owned(), "image".to_owned(), "worker-image".to_owned()),
            ("create".to_owned(), "volume".to_owned(), "cache".to_owned()),
            ("create".to_owned(), "volume".to_owned(), "runtime-data".to_owned()),
            ("create".to_owned(), "container".to_owned(), "web".to_owned()),
            ("create".to_owned(), "container".to_owned(), "worker".to_owned()),
            ("start_container".to_owned(), "container".to_owned(), "web".to_owned()),
            (
                "start_container".to_owned(),
                "container".to_owned(),
                "worker".to_owned()
            ),
        ])
    );

    let volume_creates = operations
        .iter()
        .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
        .collect::<Vec<_>>();
    assert_eq!(volume_creates.len(), 2);
    let volume_names = volume_creates
        .iter()
        .map(|op| op["resource"]["name"].as_str().ok_or("volume resource name expected"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert_eq!(volume_names, BTreeSet::from(["runtime-data", "cache"]));
    for operation in volume_creates {
        let name = operation["resource"]["name"]
            .as_str()
            .ok_or("volume resource name expected")?;
        assert_volume_create(operation, name, "6.1.0");
    }

    let container_creates = operations
        .iter()
        .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "container")
        .collect::<Vec<_>>();
    assert_eq!(container_creates.len(), 2);
    let web = container_creates
        .iter()
        .find(|op| op["resource"]["name"] == "web")
        .ok_or("web container create expected")?;
    assert_container_create(
        web,
        "web",
        "6.1.0",
        &[
            ("runtime-data", "/data/0", &["rw", "copy"]),
            ("cache", "/data/1", &["ro", "copy"]),
        ],
    );
    let worker = container_creates
        .iter()
        .find(|op| op["resource"]["name"] == "worker")
        .ok_or("worker container create expected")?;
    assert_container_create(
        worker,
        "worker",
        "6.1.0",
        &[("runtime-data", "/worker/data", &["rw", "copy"])],
    );
    assert!(output.commands_shell().contains("'volume' 'create' 'runtime-data'"));
    assert!(output.commands_shell().contains("'volume' 'create' 'cache'"));
    Ok(())
}

#[test]
fn equal_runtime_and_fallback_names_are_exact_at_reviewed_version_boundaries() -> Result<(), Box<dyn Error>> {
    for (target_version, context) in [
        (version(5, 4, 0), TargetExecutionContext::Unknown),
        (version(6, 1, 0), TargetExecutionContext::Rootless),
        (version(6, 1, 0), TargetExecutionContext::Rootful),
    ] {
        let application = volume_declarations_application(&[("data", Some("data")), ("cache", None)])?;
        let target = TargetProfile::new(PODMAN_TARGET, target_version, Some(target_version))?;
        let plan = PodmanExporter::new()?
            .with_execution_context(context)
            .plan(&application, &target)?;
        let failure_context = format!(
            "reviewed-boundary target_version={target_version}, context={context:?}, diagnostics={:?}, outcomes={:?}",
            plan.diagnostics(),
            plan.outcomes()
        );
        let authorized = plan.authorize(LossPolicy::ExactOnly);
        let output = authorized.output().ok_or(failure_context)?;
        let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
        let operations = deployment["operations"]
            .as_array()
            .ok_or("deployment operations array expected")?;
        assert_eq!(operations.len(), 2);
        let version = target_version.to_string();
        let volume_creates = operations
            .iter()
            .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
            .collect::<Vec<_>>();
        assert_eq!(volume_creates.len(), 2);
        let volume_names = volume_creates
            .iter()
            .map(|op| op["resource"]["name"].as_str().ok_or("volume resource name expected"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        assert_eq!(volume_names, BTreeSet::from(["data", "cache"]));
        for operation in volume_creates {
            let name = operation["resource"]["name"]
                .as_str()
                .ok_or("volume resource name expected")?;
            assert_volume_create(operation, name, &version);
        }
    }
    Ok(())
}

#[test]
fn equal_runtime_names_with_mounts_are_exact_for_rootless_and_rootful_targets() -> Result<(), Box<dyn Error>> {
    for context in [TargetExecutionContext::Rootless, TargetExecutionContext::Rootful] {
        let application = volume_mapping_application(&[("data", Some("data"))])?;
        let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
        let plan = PodmanExporter::new()?
            .with_execution_context(context)
            .plan(&application, &target)?;
        let failure_context = format!(
            "mounted equal-name target_version=6.1.0, context={context:?}, diagnostics={:?}, outcomes={:?}",
            plan.diagnostics(),
            plan.outcomes()
        );
        let authorized = plan.authorize(LossPolicy::ExactOnly);
        let output = authorized.output().ok_or(failure_context)?;
        let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
        let operations = deployment["operations"]
            .as_array()
            .ok_or("deployment operations array expected")?;
        let volume_create = operations
            .iter()
            .find(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
            .ok_or("equal-name volume create expected")?;
        assert_volume_create(volume_create, "data", "6.1.0");
        for (container_name, destination) in [("web", "/data/0"), ("worker", "/worker/data")] {
            let container_create = operations
                .iter()
                .find(|op| op["action"] == "create" && op["resource"]["name"] == container_name)
                .ok_or("container create expected")?;
            assert_container_create(
                container_create,
                container_name,
                "6.1.0",
                &[("data", destination, &["rw", "copy"])],
            );
        }
    }
    Ok(())
}

#[test]
fn minimum_reviewed_unknown_context_retains_native_pull_policy_refusal() -> Result<(), Box<dyn Error>> {
    let application = volume_mapping_application(&[("data", Some("data"))])?;
    let target = TargetProfile::new(PODMAN_TARGET, version(5, 4, 0), Some(version(5, 4, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;

    assert!(plan.candidate().is_none());
    let volume_outcome = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "volumes.data")
        .ok_or("exact equal-name volume mapping outcome expected")?;
    assert_eq!(volume_outcome.kind(), ConversionKind::Exact);

    let invalid_images = plan
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Invalid)
        .map(boxferry_engine::ConversionOutcome::subject)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        invalid_images,
        BTreeSet::from(["image.web-image", "image.worker-image"])
    );

    let refusals = plan
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.code().as_str() == "BFP0008"
                && diagnostic
                    .native_finding()
                    .is_some_and(|finding| finding.code() == "PLN0046")
        })
        .collect::<Vec<_>>();
    assert_eq!(refusals.len(), 2);
    let refused_images = refusals
        .iter()
        .map(|diagnostic| {
            let finding = diagnostic.native_finding().ok_or("native rendering refusal expected")?;
            let intent_field = finding
                .fields()
                .iter()
                .find(|field| field.name() == "intent_field")
                .ok_or("native intent field expected")?;
            assert_eq!(intent_field.value().expose(), "pull_policy.target_version");
            finding
                .fields()
                .iter()
                .find(|field| field.name() == "resource_name")
                .map(|field| field.value().expose())
                .ok_or("refused image name expected")
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert_eq!(refused_images, BTreeSet::from(["web-image", "worker-image"]));

    for policy in [
        LossPolicy::ExactOnly,
        LossPolicy::AllowApproximate,
        LossPolicy::AllowPartial,
    ] {
        assert!(
            plan.clone().authorize(policy).output().is_none(),
            "{policy:?} must not authorize invalid output"
        );
    }
    Ok(())
}

#[test]
fn container_runtime_names_preserve_create_start_and_logical_image_identities() -> Result<(), Box<dyn Error>> {
    for names in [
        vec![("web", Some("runtime-web")), ("worker", None)],
        vec![("web", Some("web")), ("worker", Some("worker"))],
        vec![("web", Some("worker")), ("worker", Some("third"))],
        vec![("web", Some("worker")), ("worker", Some("web"))],
        vec![("web", None), ("worker", None)],
    ] {
        let application = container_identity_application(&names, None)?;
        for reviewed in reviewed_podman_versions() {
            let target = TargetProfile::new(PODMAN_TARGET, *reviewed, Some(*reviewed))?;
            let plan = PodmanExporter::new()?.plan(&application, &target)?;
            if *reviewed < version(5, 6, 0) {
                // Container identities are mapped exactly; the existing native image-policy
                // version refusal still prevents rendering at these reviewed targets.
                assert!(plan.candidate().is_none());
                for (logical, runtime) in &names {
                    if runtime.is_some() {
                        assert!(plan.outcomes().iter().any(|outcome| {
                            outcome.subject() == format!("services.{logical}.runtime_name")
                                && outcome.kind() == ConversionKind::Exact
                        }));
                    }
                }
                assert!(plan.diagnostics().iter().all(|diagnostic| {
                    diagnostic.code().as_str() == "BFP0008"
                        && diagnostic
                            .native_finding()
                            .is_some_and(|finding| finding.code() == "PLN0046")
                }));
                assert!(plan.authorize(LossPolicy::AllowPartial).output().is_none());
                continue;
            }
            assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
            assert!(
                plan.outcomes()
                    .iter()
                    .all(|outcome| outcome.kind() == ConversionKind::Exact)
            );
            for (logical, runtime) in &names {
                let subject = format!("services.{logical}.runtime_name");
                assert_eq!(
                    plan.outcomes()
                        .iter()
                        .filter(|outcome| outcome.subject() == subject)
                        .count(),
                    usize::from(runtime.is_some())
                );
            }
            let authorized = plan.authorize(LossPolicy::ExactOnly);
            let output = authorized.output().ok_or("exact container identity output expected")?;
            let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
            let operations = deployment["operations"].as_array().ok_or("operations expected")?;
            assert_eq!(operations.len(), names.len() * 3);
            for (logical, runtime) in &names {
                let native = runtime.unwrap_or(logical);
                assert_container_identity_operations(operations, logical, native, &reviewed.to_string());
                assert!(output.commands_shell().contains(&format!("'--name' '{native}'")));
                assert!(
                    output
                        .commands_shell()
                        .contains(&format!("'container' 'start' '{native}'"))
                );
            }
        }
        assert_eq!(
            application
                .services()
                .iter()
                .map(|service| service.value().name().as_str())
                .collect::<Vec<_>>(),
            vec!["web", "worker"]
        );
    }
    Ok(())
}

fn assert_container_identity_operations(operations: &[serde_json::Value], logical: &str, native: &str, version: &str) {
    let creates = operations
        .iter()
        .filter(|op| {
            op["action"] == "create" && op["resource"]["kind"] == "container" && op["resource"]["name"] == native
        })
        .collect::<Vec<_>>();
    assert_eq!(creates.len(), 1);
    assert_container_create(creates[0], native, version, &[]);
    assert_eq!(
        operations
            .iter()
            .filter(|op| op["action"] == "ensure_image" && op["resource"]["name"] == format!("{logical}-image"))
            .count(),
        1
    );
    let starts = operations
        .iter()
        .filter(|op| op["action"] == "start_container" && op["resource"]["name"] == native)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    assert_eq!(
        starts[0]["cli"]["argv"],
        serde_json::json!(["container", "start", native])
    );
    assert_eq!(starts[0]["libpod"]["method"], "POST");
    assert_eq!(
        starts[0]["libpod"]["path_and_query"],
        format!("/v{version}/libpod/containers/{native}/start")
    );
}

#[test]
fn renamed_container_dependencies_order_native_starts_and_reject_undeclared_aliases() -> Result<(), Box<dyn Error>> {
    let names = [("web", Some("z-native")), ("worker", Some("a-native"))];
    let application = container_identity_application(&names, Some("web"))?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    assert!(plan.outcomes().iter().any(
        |outcome| outcome.subject() == "services.worker.dependencies[0]" && outcome.kind() == ConversionKind::Exact
    ));
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("ordered container output expected")?;
    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let starts = deployment["operations"]
        .as_array()
        .ok_or("operations expected")?
        .iter()
        .filter(|op| op["action"] == "start_container")
        .map(|op| op["resource"]["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(starts, vec![Some("z-native"), Some("a-native")]);
    for missing in ["z-native", "undeclared"] {
        let application = container_identity_application(&names, Some(missing))?;
        assert_container_identity_rejected(&application, "services.worker.dependencies[0]", None)?;
    }
    Ok(())
}

#[test]
fn pod_members_resolve_logical_services_to_native_containers() -> Result<(), Box<dyn Error>> {
    let mut application =
        container_identity_application(&[("web", Some("worker")), ("worker", Some("native-worker"))], None)?;
    let mut group = ServiceGroup::new(Identifier::new("app-pod")?, ResourceOwnership::Application);
    for member in ["web", "worker"] {
        group.add_member(Sourced::generated(Identifier::new(member)?))?;
    }
    application.add_service_group(Sourced::generated(group))?;
    for context in [TargetExecutionContext::Rootless, TargetExecutionContext::Rootful] {
        let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
        let plan = PodmanExporter::new()?
            .with_execution_context(context)
            .plan(&application, &target)?;
        assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
        let authorized = plan.authorize(LossPolicy::ExactOnly);
        let output = authorized
            .output()
            .ok_or("consistent bidirectional pod membership expected")?;
        let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
        let operations = deployment["operations"].as_array().ok_or("operations expected")?;
        let members = operations
            .iter()
            .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "container")
            .collect::<Vec<_>>();
        assert_eq!(members.len(), 2);
        assert_eq!(
            members
                .iter()
                .map(|op| op["resource"]["name"].as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([Some("worker"), Some("native-worker")])
        );
        for member in members {
            assert_eq!(member["libpod"]["body"]["json"]["pod"], "app-pod");
            let argv = member["cli"]["argv"].as_array().ok_or("container argv expected")?;
            assert!(
                argv.windows(2)
                    .any(|pair| pair == [serde_json::json!("--pod"), serde_json::json!("app-pod")])
            );
        }
        assert_eq!(operations.last().ok_or("pod start expected")?["action"], "start_pod");
    }
    // Neutral membership validation rejects native aliases without logical declarations.
    for missing in ["native-worker", "undeclared"] {
        let mut group = ServiceGroup::new(Identifier::new("invalid-pod")?, ResourceOwnership::Application);
        group.add_member(Sourced::generated(Identifier::new(missing)?))?;
        assert!(application.add_service_group(Sourced::generated(group)).is_err());
    }
    Ok(())
}

#[test]
fn invalid_or_colliding_container_runtime_names_never_authorize_output() -> Result<(), Box<dyn Error>> {
    let oversized = "é".repeat(129);
    for name in ["", "bad\nname", "bad\0name", oversized.as_str()] {
        let application = container_identity_application(&[("web", Some(name))], None)?;
        assert_container_identity_rejected(&application, "services.web.runtime_name", None)?;
    }
    for names in [
        vec![("web", Some("same")), ("worker", Some("same"))],
        vec![("web", Some("worker")), ("worker", None)],
        vec![("web", None), ("worker", Some("web"))],
    ] {
        let application = container_identity_application(&names, None)?;
        assert_container_identity_rejected(&application, "services.worker.runtime_name", None)?;
    }
    Ok(())
}

#[test]
fn protected_container_runtime_names_are_rejected_before_native_construction() -> Result<(), Box<dyn Error>> {
    for canary in ["private-container-name-39b760e4", "private\ncontainer-name-39b760e4"] {
        let mut service = Service::new(Identifier::new("web")?);
        service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
        service.set_runtime_name(Sourced::generated(ProtectedString::sensitive(canary)));
        let mut application = Application::new(Identifier::new("protected-container-test")?);
        application.add_service(Sourced::generated(service))?;
        assert_container_identity_rejected(&application, "services.web.runtime_name", Some(canary))?;
    }
    Ok(())
}

fn container_identity_application(
    names: &[(&str, Option<&str>)],
    dependency: Option<&str>,
) -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("container-identity-test")?);
    for (logical, runtime) in names {
        let mut service = Service::new(Identifier::new(*logical)?);
        service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
        if let Some(runtime) = runtime {
            service.set_runtime_name(Sourced::generated(ProtectedString::plain(*runtime)));
        }
        if *logical == "worker"
            && let Some(dependency) = dependency
        {
            service.add_dependency(Sourced::generated(ServiceDependency::new(Identifier::new(dependency)?)));
        }
        application.add_service(Sourced::generated(service))?;
    }
    Ok(application)
}

fn assert_container_identity_rejected(
    application: &Application,
    subject: &str,
    canary: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(application, &target)?;
    assert!(plan.candidate().is_none());
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Invalid)
    );
    let diagnostic = plan
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code().as_str() == "BFP0008"
                && diagnostic.severity() == Severity::Error
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().expose() == subject)
        })
        .ok_or("identity refusal diagnostic expected")?;
    assert_context_fields(diagnostic, &["subject", "reason", "decision"]);
    if let Some(canary) = canary {
        assert!(!format!("{plan:?}").contains(canary));
        assert!(!format!("{diagnostic:?}").contains(canary));
        assert!(
            plan.diagnostics()
                .iter()
                .flat_map(boxferry_engine::Diagnostic::fields)
                .all(|field| !field.value().expose().contains(canary))
        );
        assert!(diagnostic.fields().iter().any(|field| field.name() == "reason"
            && field.value().expose()
                == "Protected Podman container runtime names cannot be emitted as target resource identities"));
    }
    for policy in [
        LossPolicy::ExactOnly,
        LossPolicy::AllowApproximate,
        LossPolicy::AllowPartial,
    ] {
        let authorized = plan.clone().authorize(policy);
        assert!(authorized.output().is_none());
        if let Some(canary) = canary {
            assert!(!format!("{authorized:?}").contains(canary));
        }
    }
    Ok(())
}

#[test]
fn duplicate_resolved_volume_identities_are_rejected_but_chained_renames_are_valid() -> Result<(), Box<dyn Error>> {
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    for volumes in [
        vec![("first", Some("same")), ("second", Some("same"))],
        vec![("first", Some("second")), ("second", None)],
    ] {
        let application = volume_mapping_application(&volumes)?;
        let plan = PodmanExporter::new()?.plan(&application, &target)?;
        assert!(
            plan.candidate().is_none(),
            "duplicate target volume identities must refuse planning"
        );
        assert!(
            plan.outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Invalid)
        );
        assert!(
            plan.diagnostics().iter().any(|diagnostic| {
                diagnostic.code().as_str() == "BFP0008" && diagnostic.severity() == Severity::Error
            })
        );
        assert!(plan.clone().authorize(LossPolicy::AllowPartial).output().is_none());
    }
    let application = volume_mapping_application(&[("first", Some("second")), ("second", Some("third"))])?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    let authorized = plan.authorize(LossPolicy::ExactOnly);
    let output = authorized.output().ok_or("distinct volume identities expected")?;
    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let volume_names = deployment["operations"]
        .as_array()
        .ok_or("operations array expected")?
        .iter()
        .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
        .map(|op| op["resource"]["name"].as_str().ok_or("volume resource name expected"))
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert_eq!(volume_names, BTreeSet::from(["second", "third"]));
    let container = deployment["operations"]
        .as_array()
        .and_then(|operations| {
            operations
                .iter()
                .find(|op| op["action"] == "create" && op["resource"]["name"] == "web")
        })
        .ok_or("web container create expected")?;
    assert_container_create(
        container,
        "web",
        "6.1.0",
        &[
            ("second", "/data/0", &["rw", "copy"]),
            ("third", "/data/1", &["ro", "copy"]),
        ],
    );
    Ok(())
}

#[test]
fn external_implicit_and_uncertain_volumes_remain_preconditions() -> Result<(), Box<dyn Error>> {
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    for ownership in [
        ResourceOwnership::External,
        ResourceOwnership::Implicit,
        ResourceOwnership::Uncertain,
    ] {
        let application =
            volume_mapping_application_with_ownership(&[("logical-data", Some("runtime-data"))], ownership)?;
        let plan = PodmanExporter::new()?.plan(&application, &target)?;
        let authorized = plan.authorize(LossPolicy::ExactOnly);
        let output = authorized
            .output()
            .ok_or("external volume precondition output expected")?;
        let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
        let prerequisites = deployment["external_preconditions"]
            .as_array()
            .ok_or("preconditions array expected")?;
        assert_eq!(
            prerequisites
                .iter()
                .filter(|resource| resource["kind"] == "volume")
                .map(|resource| resource["name"].as_str())
                .collect::<Vec<_>>(),
            vec![Some("runtime-data")]
        );
        assert_eq!(
            deployment["operations"]
                .as_array()
                .ok_or("operations array expected")?
                .iter()
                .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
                .count(),
            0
        );
        assert!(
            output
                .commands_shell()
                .contains("Requires external volume: 'runtime-data'")
        );
        let operations = deployment["operations"].as_array().ok_or("operations array expected")?;
        let container_creates = operations
            .iter()
            .filter(|op| op["action"] == "create" && op["resource"]["kind"] == "container")
            .collect::<Vec<_>>();
        assert_eq!(container_creates.len(), 2);
        let web = container_creates
            .iter()
            .find(|op| op["resource"]["name"] == "web")
            .ok_or("web container create expected")?;
        assert_container_create(web, "web", "6.1.0", &[("runtime-data", "/data/0", &["rw", "copy"])]);
        let worker = container_creates
            .iter()
            .find(|op| op["resource"]["name"] == "worker")
            .ok_or("worker container create expected")?;
        assert_container_create(
            worker,
            "worker",
            "6.1.0",
            &[("runtime-data", "/worker/data", &["rw", "copy"])],
        );
    }
    Ok(())
}

#[test]
fn protected_runtime_volume_names_fail_closed_without_disclosure_under_every_loss_policy() -> Result<(), Box<dyn Error>>
{
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    for canary in [
        "private-volume-runtime-name-cf20c31a",
        "private\nvolume-runtime-name-cf20c31a",
    ] {
        let mut volume = Volume::new(Identifier::new("logical-data")?, ResourceOwnership::Application);
        volume.set_runtime_name(Sourced::generated(ProtectedString::sensitive(canary)));
        let application = volume_application_with_mounts(
            vec![volume],
            &[("logical-data", "/data", false)],
            &[("logical-data", "/worker/data", false)],
        )?;
        let plan = PodmanExporter::new()?.plan(&application, &target)?;
        assert!(plan.candidate().is_none());
        let diagnostic = plan
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code().as_str() == "BFP0008")
            .ok_or("protected runtime name must produce BFP0008")?;
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert_context_fields(diagnostic, &["subject", "reason", "decision"]);
        assert!(diagnostic.fields().iter().any(|field| {
            field.name() == "subject" && field.value().expose() == "volumes.logical-data.runtime_name"
        }));
        assert!(diagnostic.fields().iter().any(|field| {
            field.name() == "reason"
                && field.value().expose()
                    == "Protected Podman volume runtime names cannot be emitted as target resource identities"
        }));
        assert!(plan.outcomes().iter().any(|outcome| {
            outcome.kind() == ConversionKind::Invalid && outcome.subject() == "volumes.logical-data.runtime_name"
        }));
        assert!(!format!("{plan:?}").contains(canary));
        assert!(!format!("{diagnostic:?}").contains(canary));
        assert!(
            diagnostic
                .fields()
                .iter()
                .all(|field| !field.value().expose().contains(canary))
        );
        assert!(
            diagnostic
                .fields()
                .iter()
                .all(|field| field.value().expose() != "[REDACTED]")
        );
        for policy in [
            LossPolicy::ExactOnly,
            LossPolicy::AllowApproximate,
            LossPolicy::AllowPartial,
        ] {
            let result = plan.clone().authorize(policy);
            assert!(result.output().is_none());
            assert!(!format!("{result:?}").contains(canary));
        }
    }
    Ok(())
}

#[test]
fn invalid_runtime_names_and_undeclared_or_aliased_mounts_are_rejected() -> Result<(), Box<dyn Error>> {
    let mut invalid = Volume::new(Identifier::new("logical-data")?, ResourceOwnership::Application);
    invalid.set_runtime_name(Sourced::generated(ProtectedString::plain("bad\nname")));
    let invalid_application = volume_application_with_mounts(vec![invalid], &[("logical-data", "/data", false)], &[])?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let invalid_plan = PodmanExporter::new()?.plan(&invalid_application, &target)?;
    assert!(invalid_plan.candidate().is_none());
    assert!(
        invalid_plan
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "BFP0008")
    );
    assert!(
        invalid_plan
            .clone()
            .authorize(LossPolicy::AllowPartial)
            .output()
            .is_none()
    );

    for mount_name in ["runtime-data", "undeclared"] {
        let volume_name = "logical-data";
        let mut volume = Volume::new(Identifier::new(volume_name)?, ResourceOwnership::Application);
        volume.set_runtime_name(Sourced::generated(ProtectedString::plain("runtime-data")));
        let application = volume_application_with_mounts(vec![volume], &[(mount_name, "/data", false)], &[])?;
        let plan = PodmanExporter::new()?.plan(&application, &target)?;
        assert!(plan.candidate().is_none());
        assert!(plan.clone().authorize(LossPolicy::AllowPartial).output().is_none());
        assert!(plan.diagnostics().iter().any(|diagnostic| {
            diagnostic.code().as_str() == "BFP0008"
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().expose() == "services.web.mounts[0].source")
        }));
    }
    Ok(())
}

#[test]
fn unrelated_volume_settings_remain_partial_with_bfp0007() -> Result<(), Box<dyn Error>> {
    let mut volume = Volume::new(Identifier::new("logical-data")?, ResourceOwnership::Application);
    volume.set_runtime_name(Sourced::generated(ProtectedString::plain("runtime-data")));
    volume.set_driver(Sourced::generated(ProtectedString::plain("local")));
    let application = volume_application_with_mounts(vec![volume], &[("logical-data", "/data", false)], &[])?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    assert!(plan.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0007"
            && diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "subject" && field.value().expose() == "volumes.logical-data.settings")
    }));
    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized
        .output()
        .ok_or("partial output for unrelated volume setting expected")?;
    let deployment: serde_json::Value = serde_json::from_str(output.deployment_json())?;
    let volume_create = deployment["operations"]
        .as_array()
        .and_then(|operations| {
            operations
                .iter()
                .find(|op| op["action"] == "create" && op["resource"]["kind"] == "volume")
        })
        .ok_or("runtime-named volume create expected under partial policy")?;
    assert_eq!(volume_create["resource"]["name"], "runtime-data");
    Ok(())
}

fn volume_declarations_application(volumes: &[(&str, Option<&str>)]) -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("volume-declarations-test")?);
    for (logical, runtime) in volumes {
        let mut volume = Volume::new(Identifier::new(*logical)?, ResourceOwnership::Application);
        if let Some(runtime) = runtime {
            volume.set_runtime_name(Sourced::generated(ProtectedString::plain(*runtime)));
        }
        application.add_volume(Sourced::generated(volume))?;
    }
    Ok(application)
}

fn volume_mapping_application(volumes: &[(&str, Option<&str>)]) -> Result<Application, Box<dyn Error>> {
    volume_mapping_application_with_ownership(volumes, ResourceOwnership::Application)
}

fn volume_mapping_application_with_ownership(
    volumes: &[(&str, Option<&str>)],
    ownership: ResourceOwnership,
) -> Result<Application, Box<dyn Error>> {
    let values = volumes
        .iter()
        .map(|(logical, runtime)| {
            let mut volume = Volume::new(Identifier::new(*logical)?, ownership);
            if let Some(runtime) = runtime {
                volume.set_runtime_name(Sourced::generated(ProtectedString::plain(*runtime)));
            }
            Ok(volume)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let web_mounts = volumes
        .iter()
        .enumerate()
        .map(|(index, (logical, _))| (*logical, if index == 0 { "/data/0" } else { "/data/1" }, index == 1))
        .collect::<Vec<_>>();
    let worker_mounts = volumes
        .first()
        .map(|(logical, _)| vec![(*logical, "/worker/data", false)])
        .unwrap_or_default();
    volume_application_with_mounts(values, &web_mounts, &worker_mounts)
}

fn volume_application_with_mounts(
    volumes: Vec<Volume>,
    web_mounts: &[(&str, &str, bool)],
    worker_mounts: &[(&str, &str, bool)],
) -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("adapter-test")?);
    for volume in volumes {
        application.add_volume(Sourced::generated(volume))?;
    }
    add_volume_mounting_service(&mut application, "web", web_mounts)?;
    if !worker_mounts.is_empty() {
        add_volume_mounting_service(&mut application, "worker", worker_mounts)?;
    }
    Ok(application)
}

fn add_volume_mounting_service(
    application: &mut Application,
    service_name: &str,
    mounts: &[(&str, &str, bool)],
) -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new(service_name)?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    for (logical_name, destination, read_only) in mounts {
        service.add_mount(Sourced::generated(Mount::new(
            MountSource::Volume(Identifier::new(*logical_name)?),
            *destination,
            *read_only,
        )?));
    }
    application.add_service(Sourced::generated(service))?;
    Ok(())
}

fn assert_volume_create(operation: &serde_json::Value, name: &str, version: &str) {
    assert_eq!(operation["status"], "exact");
    assert_eq!(operation["action"], "create");
    assert_eq!(
        operation["resource"],
        serde_json::json!({"kind": "volume", "name": name})
    );
    assert_eq!(operation["cli"]["program"], "podman");
    assert_eq!(operation["cli"]["argv"], serde_json::json!(["volume", "create", name]));
    assert_eq!(operation["cli"]["external_sensitive_input_required"], false);
    assert_eq!(operation["libpod"]["method"], "POST");
    assert_eq!(
        operation["libpod"]["path_and_query"],
        format!("/v{version}/libpod/volumes/create")
    );
    assert_eq!(operation["libpod"]["body"]["kind"], "json");
    assert_eq!(operation["libpod"]["body"]["json"], serde_json::json!({"Name": name}));
}

fn assert_container_create(operation: &serde_json::Value, name: &str, version: &str, mounts: &[(&str, &str, &[&str])]) {
    assert_eq!(operation["status"], "exact");
    assert_eq!(operation["action"], "create");
    assert_eq!(
        operation["resource"],
        serde_json::json!({"kind": "container", "name": name})
    );
    assert_eq!(operation["cli"]["program"], "podman");
    let mut argv = vec![
        "container".to_owned(),
        "create".to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--pull=never".to_owned(),
    ];
    for (source, destination, options) in mounts {
        argv.extend([
            "--volume".to_owned(),
            format!("{source}:{destination}:{},copy", options[0]),
        ]);
    }
    argv.push("example.invalid/web:1".to_owned());
    assert_eq!(operation["cli"]["argv"], serde_json::json!(argv));
    assert_eq!(operation["cli"]["external_sensitive_input_required"], false);
    assert_eq!(operation["libpod"]["method"], "POST");
    assert_eq!(
        operation["libpod"]["path_and_query"],
        format!("/v{version}/libpod/containers/create?name={name}")
    );
    assert_eq!(operation["libpod"]["body"]["kind"], "json");
    assert_eq!(operation["libpod"]["body"]["json"]["image"], "example.invalid/web:1");
    let expected_volumes = mounts
        .iter()
        .map(|(source, destination, options)| {
            serde_json::json!({"Name": source, "Dest": destination, "Options": options})
        })
        .collect::<Vec<_>>();
    if mounts.is_empty() {
        assert!(operation["libpod"]["body"]["json"].get("volumes").is_none());
    } else {
        assert_eq!(
            operation["libpod"]["body"]["json"]["volumes"],
            serde_json::json!(expected_volumes)
        );
    }
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
fn duplicate_authored_network_alias_remains_invalid() -> Result<(), Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("duplicate-alias-test")?);
    application.add_network(Sourced::generated(Network::new(
        Identifier::new("frontend")?,
        ResourceOwnership::Application,
    )))?;
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.add_network(Sourced::generated(NetworkAttachment::new(
        Identifier::new("frontend")?,
        vec![
            Sourced::generated(ProtectedString::plain("web")),
            Sourced::generated(ProtectedString::plain("web")),
        ],
    )));
    application.add_service(Sourced::generated(service))?;

    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let plan = PodmanExporter::new()?.plan(&application, &target)?;
    assert!(plan.candidate().is_none());
    assert!(plan.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0008"
            && diagnostic.fields().iter().any(|field| {
                field.name() == "subject" && field.value().expose() == "services.web.networks[0].aliases[1]"
            })
    }));
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

#[test]
fn image_defaults_are_normalized_or_reported_as_partial_losses() -> Result<(), Box<dyn Error>> {
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    service.set_working_directory(Sourced::generated(ProtectedString::plain("/srv/app/")));
    service.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new("org.opencontainers.image.description")?,
        ProtectedString::sensitive("private-multiline-label-canary\nsecond line"),
    )));
    service.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new("org.opencontainers.image.title")?,
        ProtectedString::plain("web"),
    )));

    let mut application = Application::new(Identifier::new("image-defaults")?);
    application.add_service(Sourced::generated(service))?;
    let target = TargetProfile::new(PODMAN_TARGET, version(6, 1, 0), Some(version(6, 1, 0)))?;
    let exporter = PodmanExporter::new()?.with_execution_context(TargetExecutionContext::Rootless);
    assert!(
        exporter
            .plan(&application, &target)?
            .authorize(LossPolicy::ExactOnly)
            .output()
            .is_none()
    );
    let plan = exporter.plan(&application, &target)?;

    assert!(plan.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0007"
            && diagnostic.fields().iter().any(|field| {
                field.name() == "subject"
                    && field.value().redacted() == "services.web.labels.org.opencontainers.image.description"
            })
    }));
    assert!(!format!("{plan:?}").contains("private-multiline-label-canary"));

    let authorized = plan.authorize(LossPolicy::AllowPartial);
    let output = authorized.output().ok_or("partial Podman output expected")?;
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

    assert!(
        argv.windows(2)
            .any(|arguments| { arguments[0] == "--workdir" && arguments[1].as_str() == Some("/srv/app") })
    );
    for rendered in [output.deployment_json(), output.commands_shell()] {
        assert!(!rendered.contains("/srv/app/"));
        assert!(!rendered.contains("org.opencontainers.image.description"));
        assert!(!rendered.contains("private-multiline-label-canary"));
        assert!(!rendered.contains("second line"));
        assert!(rendered.contains("org.opencontainers.image.title"));
    }
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
