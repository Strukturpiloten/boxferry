//! Adapter coverage for legacy Podman input evidence and safe diagnostics.

#![allow(clippy::expect_used, clippy::panic)] // Fixture transport failures should identify broken test data.

use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    sync::Mutex,
};

use boxferry_engine::{ConversionKind, ExportAdapter, ImportAdapter, PlatformVersion, TargetProfile};
use boxferry_model::Identifier;
use boxferry_podman::{
    PODMAN_TARGET, PodmanExporter, PodmanImporter, PodmanPromotionPolicy, PodmanSource, podman_lens,
};
use futures::executor::block_on;
use podman_lens::{
    AcquisitionOptions, DiagnosticCode, DiscoveryRequest, LabelSelector, LibpodHeader, LibpodHeaders, LibpodRequest,
    LibpodResponse, LibpodTransport, LibpodTransportFuture, ResourceKind, ResourceSelector, TransportError,
    acquire_inventory, discover,
};

struct FixtureTransport {
    responses: Mutex<VecDeque<LibpodResponse>>,
}

impl FixtureTransport {
    fn new(responses: Vec<LibpodResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
        }
    }
}

impl LibpodTransport for FixtureTransport {
    fn send<'a>(&'a self, _request: &'a LibpodRequest) -> LibpodTransportFuture<'a> {
        Box::pin(async move {
            self.responses
                .lock()
                .map_err(|_| TransportError::unavailable())?
                .pop_front()
                .ok_or_else(TransportError::unavailable)
        })
    }
}

fn json(body: &str) -> Result<LibpodResponse, Box<dyn Error>> {
    Ok(LibpodResponse::new(
        200,
        LibpodHeaders::new(vec![LibpodHeader::new("content-type", "application/json")?]),
        body.as_bytes(),
    )?)
}

fn legacy_responses(container_inspect: &str) -> Result<Vec<LibpodResponse>, Box<dyn Error>> {
    Ok(vec![
        LibpodResponse::new(
            200,
            LibpodHeaders::new(vec![LibpodHeader::new("Libpod-API-Version", "3.0.0")?]),
            [],
        )?,
        json(r#"{"Components":[{"Name":"Podman Engine","Version":"3.0.1"}]}"#)?,
        json(r#"[{"Id":"c-web","Names":["web"]}]"#)?,
        json("[]")?,
        json(r#"[{"Name":"legacy-net"}]"#)?,
        json(r#"[{"Name":"legacy-data"}]"#)?,
        json(r#"[{"Id":"sha256:legacy","Names":["example.invalid/legacy:1"]}]"#)?,
        json(container_inspect)?,
        json(
            r#"[{"cniVersion":"0.4.0","name":"legacy-net","plugins":[{"type":"bridge","ipam":{"type":"host-local","ranges":[[{"subnet":"10.88.0.0/16","gateway":"10.88.0.1"}]]}}]}]"#,
        )?,
        json(r#"{"Name":"legacy-data"}"#)?,
        json(r#"{"Id":"sha256:legacy","RepoTags":["example.invalid/legacy:1"]}"#)?,
    ])
}

fn legacy_source(container_inspect: &str) -> Result<PodmanSource, Box<dyn Error>> {
    let responses = legacy_responses(container_inspect)?;
    let mut request = DiscoveryRequest::new();
    request.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    legacy_source_with_request(responses, &request)
}

fn legacy_source_with_request(
    responses: Vec<LibpodResponse>,
    request: &DiscoveryRequest,
) -> Result<PodmanSource, Box<dyn Error>> {
    let transport = FixtureTransport::new(responses);
    let inventory = block_on(acquire_inventory(&transport, AcquisitionOptions::redacted()))?;
    let graph = discover(&inventory, request)?;
    Ok(PodmanSource::new(
        Identifier::new("legacy-migration")?,
        inventory,
        graph,
    ))
}

fn failed_container_list_responses() -> Result<Vec<LibpodResponse>, Box<dyn Error>> {
    Ok(vec![
        LibpodResponse::new(
            200,
            LibpodHeaders::new(vec![LibpodHeader::new("Libpod-API-Version", "3.0.0")?]),
            [],
        )?,
        json(r#"{"Components":[{"Name":"Podman Engine","Version":"3.0.1"}]}"#)?,
        LibpodResponse::new(500, LibpodHeaders::new(Vec::<LibpodHeader>::new()), [])?,
        json("[]")?,
        json(r#"[{"Name":"legacy-net"}]"#)?,
        json(r#"[{"Name":"legacy-data"}]"#)?,
        json(r#"[{"Id":"sha256:legacy","Names":["example.invalid/legacy:1"]}]"#)?,
        json(
            r#"[{"cniVersion":"0.4.0","name":"legacy-net","plugins":[{"type":"bridge","ipam":{"type":"host-local","ranges":[[{"subnet":"10.88.0.0/16","gateway":"10.88.0.1"}]]}}]}]"#,
        )?,
        json(r#"{"Name":"legacy-data"}"#)?,
        json(r#"{"Id":"sha256:legacy","RepoTags":["example.invalid/legacy:1"]}"#)?,
    ])
}

fn failed_unrelated_volume_list_responses() -> Result<Vec<LibpodResponse>, Box<dyn Error>> {
    Ok(vec![
        LibpodResponse::new(
            200,
            LibpodHeaders::new(vec![LibpodHeader::new("Libpod-API-Version", "3.0.0")?]),
            [],
        )?,
        json(r#"{"Components":[{"Name":"Podman Engine","Version":"3.0.1"}]}"#)?,
        json(r#"[{"Id":"c-web","Names":["web"]}]"#)?,
        json("[]")?,
        json("[]")?,
        LibpodResponse::new(500, LibpodHeaders::new(Vec::<LibpodHeader>::new()), [])?,
        json(r#"[{"Id":"sha256:legacy","Names":["example.invalid/legacy:1"]}]"#)?,
        json(
            r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":"","Labels":{"io.boxferry.test":"legacy"}},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
        )?,
        json(r#"{"Id":"sha256:legacy","RepoTags":["example.invalid/legacy:1"]}"#)?,
    ])
}

#[test]
fn input_only_podman_three_imports_through_the_neutral_model() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{"legacy-net":{}}},"Mounts":[{"Type":"volume","Name":"legacy-data","Destination":"/data","RW":true}]}"#,
    )?;
    assert_eq!(source.observed_engine_version(), "3.0.1");
    assert_eq!(source.observed_api_version(), "3.0.0");
    assert!(!source.input_capability().output_supported());

    let result = PodmanImporter::new()?.import(&source);
    let application = result
        .application()
        .expect("complete legacy source has a neutral application");
    assert_eq!(application.services().len(), 1);
    assert_eq!(application.networks().len(), 1);
    assert_eq!(application.volumes().len(), 1);
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() != "BFP0001")
    );
    let ordinary_native_findings = result
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFP0002")
        .filter_map(|diagnostic| diagnostic.native_finding())
        .filter(|finding| finding.stage() == "acquisition")
        .collect::<Vec<_>>();
    assert!(
        ordinary_native_findings.iter().all(|finding| [
            "subject",
            "reason",
            "resource_kind",
            "observation_origin",
            "source_engine",
            "source_api"
        ]
        .iter()
        .all(|expected| finding.fields().iter().any(|field| field.name() == *expected))),
        "ordinary native findings must retain safe actionable context"
    );
    let ordinary_codes = ordinary_native_findings
        .iter()
        .map(|finding| finding.code())
        .collect::<BTreeSet<_>>();
    assert!(
        ordinary_codes.len() < 4,
        "one small legacy application must not create one human diagnostic per native field"
    );
    for diagnostic in result.diagnostics().iter().filter(|diagnostic| {
        matches!(
            diagnostic.code().as_str(),
            "BFP0001" | "BFP0002" | "BFP0003" | "BFP0004" | "BFP0005"
        )
    }) {
        for expected in ["subject", "reason", "decision"] {
            assert!(
                diagnostic.fields().iter().any(|field| field.name() == expected),
                "{} omitted {expected}: {diagnostic:?}",
                diagnostic.code().as_str()
            );
        }
    }

    let output_target = TargetProfile::new(
        PODMAN_TARGET,
        PlatformVersion::new(6, 1, 0),
        Some(PlatformVersion::new(6, 1, 0)),
    )?;
    let output = PodmanExporter::new()?.plan(application, &output_target)?;
    assert!(
        output.candidate().is_some(),
        "an input-only source can migrate to an explicit modern target"
    );

    let inventory_snapshot = serde_json::to_value(source.redacted_inventory_snapshot())?;
    assert_eq!(inventory_snapshot["service"]["engine"], "3.0.1");
    assert!(inventory_snapshot.to_string().contains("[redacted]") || !inventory_snapshot.to_string().contains("Env"));
    let graph_snapshot = serde_json::to_value(source.redacted_graph_snapshot())?;
    assert_eq!(graph_snapshot["schema_version"], 1);
    Ok(())
}

#[test]
fn repeated_native_fields_are_reported_as_one_bounded_actionable_group() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[],"Unknown00":0,"Unknown01":1,"Unknown02":2,"Unknown03":3,"Unknown04":4,"Unknown05":5,"Unknown06":6,"Unknown07":7,"Unknown08":8,"Unknown09":9,"Unknown10":10,"Unknown11":11}"#,
    )?;
    let result = PodmanImporter::new()?.import(&source);
    let groups = result
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == "BFP0002")
        .filter(|diagnostic| {
            diagnostic.native_finding().is_some_and(|finding| {
                finding.code() == DiagnosticCode::NativeFieldUnsupported.as_str() && finding.stage() == "acquisition"
            })
        })
        .filter(|diagnostic| {
            diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "resource" && field.value().redacted() == "container:web")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        groups.len(),
        1,
        "one resource and native rule must produce one diagnostic"
    );

    let diagnostic = groups[0];
    let field = |name: &str| {
        diagnostic
            .fields()
            .iter()
            .find(|field| field.name() == name)
            .map(|field| field.value().redacted())
    };
    assert_eq!(field("occurrence_count"), Some("12"));
    assert_eq!(field("native_path_count"), Some("12"));
    assert_eq!(
        field("native_path_samples"),
        Some("$.Unknown00, $.Unknown01, $.Unknown02, $.Unknown03, $.Unknown04, $.Unknown05, $.Unknown06, $.Unknown07")
    );
    let native = diagnostic.native_finding().ok_or("native finding")?;
    assert!(
        ["occurrence_count", "native_path_count", "native_path_samples"]
            .iter()
            .all(|expected| native.fields().iter().any(|field| field.name() == *expected))
    );
    assert_eq!(
        result
            .outcomes()
            .iter()
            .filter(|outcome| outcome.subject() == "podman.acquisition.PLN0023")
            .count(),
        12,
        "aggregation must not change fidelity accounting"
    );
    Ok(())
}

#[test]
fn explicit_named_volume_promotion_accepts_configured_mount_identity() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[{"Type":"volume","Name":"legacy-data","Destination":"/data","RW":true}]}"#,
    )?
    .with_promotion_policy(PodmanPromotionPolicy::conservative().with_effective_named_volume_mounts(true));
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert_eq!(service.mounts().len(), 1);
    assert_eq!(service.mounts()[0].value().target(), "/data");
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0003" && diagnostic.summary().contains("portable named-volume mount promoted")
    }));
    Ok(())
}

#[test]
fn empty_user_and_id_derived_hostname_do_not_become_authored_intent() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Cmd":["sleep","3600"],"Entrypoint":"","User":"","WorkingDir":"/","Hostname":"c-web"},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
    )?;
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert!(service.user().is_none());
    assert!(service.hostname().is_none());
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/")
    );
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0002"
            && diagnostic.summary() == "Podman's ID-derived hostname is runtime-assigned evidence"
    }));

    let target = TargetProfile::new(
        PODMAN_TARGET,
        PlatformVersion::new(6, 1, 0),
        Some(PlatformVersion::new(6, 1, 0)),
    )?;
    let output = PodmanExporter::new()?.plan(application, &target)?;
    assert!(output.candidate().is_some());
    Ok(())
}

#[test]
fn hostname_observed_with_host_uts_namespace_does_not_become_authored_intent() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Cmd":["sleep","3600"],"Entrypoint":"","Hostname":"outer-host"},"HostConfig":{"RestartPolicy":{"Name":""},"UTSMode":"host"},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
    )?;
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert!(service.hostname().is_none());
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0002"
            && diagnostic.summary() == "a hostname observed with the host UTS namespace is not portable authored intent"
    }));

    let target = TargetProfile::new(
        PODMAN_TARGET,
        PlatformVersion::new(6, 1, 0),
        Some(PlatformVersion::new(6, 1, 0)),
    )?;
    let output = PodmanExporter::new()?.plan(application, &target)?;
    let plan = output.candidate().ok_or("deployment plan")?;
    assert!(!plan.deployment_json().contains("\"hostname\""));
    assert!(!plan.commands_shell().contains("'--hostname'"));
    Ok(())
}

#[test]
fn malformed_legacy_resource_reports_identity_path_and_version_evidence() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":17},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
    )?;
    let result = PodmanImporter::new()?.import(&source);
    let first_invalid = result
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "BFP0001")
        .expect("malformed inspect result must emit BFP0001");
    assert_eq!(
        first_invalid.summary(),
        "Podman source evidence is incomplete or malformed",
        "the causal native finding must precede generic invalid-observation wording"
    );
    let diagnostic = result
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.summary() == "Podman source evidence is incomplete or malformed")
        .expect("malformed inspect result must be actionable BFP0001");
    assert_eq!(
        diagnostic.summary(),
        "Podman source evidence is incomplete or malformed"
    );
    assert!(
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "source_engine" && field.value().redacted() == "3.0.1")
    );
    assert!(
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "resource" && field.value().redacted() == "container:web")
    );
    assert!(
        diagnostic
            .fields()
            .iter()
            .any(|field| { field.name() == "native_path" && field.value().redacted() == "$.Config.Entrypoint" })
    );
    let native = diagnostic.native_finding().expect("native provenance is retained");
    assert_eq!(native.code(), DiagnosticCode::ResourceMalformed.as_str());
    Ok(())
}

#[test]
fn input_only_source_cannot_be_mistaken_for_a_modern_output_target() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
    )?;
    assert!(source.input_capability().podman_minor_line().starts_with("3.0"));
    assert!(!source.input_capability().output_supported());
    assert!(source.inventory().service().output_target_profile().is_none());
    Ok(())
}

#[test]
fn failed_list_section_is_invalid_for_exact_and_all_selection() -> Result<(), Box<dyn Error>> {
    let mut exact = DiscoveryRequest::new();
    exact.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    let mut all = DiscoveryRequest::new();
    all.select_all();

    for request in [exact, all] {
        let source = legacy_source_with_request(failed_container_list_responses()?, &request)?;
        let result = PodmanImporter::new()?.import(&source);
        let diagnostic = result
            .diagnostics()
            .iter()
            .find(|diagnostic| {
                diagnostic.code().as_str() == "BFP0001"
                    && diagnostic
                        .native_finding()
                        .is_some_and(|finding| finding.code() == DiagnosticCode::InventoryHttpStatus.as_str())
            })
            .expect("a failed list section must remain a causal BFP0001");
        assert!(
            diagnostic
                .fields()
                .iter()
                .any(|field| { field.name() == "resource_kind" && field.value().redacted() == "container" })
        );
        assert!(
            result
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Invalid),
            "partial loss policy must never authorize an unavailable inventory section"
        );
    }
    Ok(())
}

#[test]
fn unrelated_failed_section_does_not_block_bounded_exact_or_prefix_selection() -> Result<(), Box<dyn Error>> {
    let mut exact = DiscoveryRequest::new();
    exact.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    let mut prefix = DiscoveryRequest::new();
    prefix.add_root(ResourceSelector::prefix(ResourceKind::Container, "we")?);

    for request in [exact, prefix] {
        let source = legacy_source_with_request(failed_unrelated_volume_list_responses()?, &request)?;
        let result = PodmanImporter::new()?.import(&source);
        assert_eq!(
            result
                .application()
                .expect("bounded selection remains usable")
                .services()
                .len(),
            1
        );
        assert!(
            result
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code().as_str() != "BFP0001"),
            "an unavailable unrelated volume section must not invalidate a container-only selection"
        );
    }
    Ok(())
}

#[test]
fn failed_section_blocks_label_selection_that_must_scan_all_resource_kinds() -> Result<(), Box<dyn Error>> {
    let mut request = DiscoveryRequest::new();
    request.add_label_root(LabelSelector::exact("io.boxferry.test", "legacy")?);
    let source = legacy_source_with_request(failed_unrelated_volume_list_responses()?, &request)?;
    let result = PodmanImporter::new()?.import(&source);
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0001"
            && diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "resource_kind" && field.value().redacted() == "volume")
    }));
    Ok(())
}
