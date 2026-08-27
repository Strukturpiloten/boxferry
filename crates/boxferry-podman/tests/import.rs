//! Adapter coverage for legacy Podman input evidence and safe diagnostics.

#![allow(clippy::expect_used, clippy::panic)] // Fixture transport failures should identify broken test data.

use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    sync::Mutex,
};

use boxferry_engine::{ConversionKind, ExportAdapter, ImportAdapter, PlatformVersion, TargetProfile};
use boxferry_model::{
    EnvironmentValue, HealthcheckCommand, Identifier, MountSource, ResourceOwnership, RestartPolicy, SelinuxRelabel,
};
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

fn modern_responses(container_inspect: &str) -> Result<Vec<LibpodResponse>, Box<dyn Error>> {
    let mut responses = legacy_responses(container_inspect)?;
    responses[0] = LibpodResponse::new(
        200,
        LibpodHeaders::new(vec![LibpodHeader::new("Libpod-API-Version", "6.0.2")?]),
        [],
    )?;
    responses[1] = json(r#"{"Components":[{"Name":"Podman Engine","Version":"6.0.2"}]}"#)?;
    responses.insert(7, json("[]")?);
    responses[9] = json(
        r#"{"id":"legacy-net","name":"legacy-net","internal":true,"subnets":[{"subnet":"10.88.0.0/16","gateway":"10.88.0.1","lease_range":{"start_ip":"10.88.1.0","end_ip":"10.88.1.255"}},{"subnet":"fd42::/64","gateway":"fd42::1","lease_range":{"start_ip":"fd42::100","end_ip":"fd42::1ff"}}]}"#,
    )?;
    Ok(responses)
}

fn grouped_selection_with_unrelated_responses() -> Result<Vec<LibpodResponse>, Box<dyn Error>> {
    let labels = |service: &str| {
        format!(
            r#""Labels":{{"com.docker.compose.project":"migration","com.docker.compose.service":"{service}","com.docker.compose.container-number":"1","io.boxferry.keep":"{service}"}}"#
        )
    };
    let selected = |id: &str, name: &str, service: &str| {
        format!(
            r#"{{"Id":"{id}","Name":"{name}","ImageName":"example.invalid/legacy:1","Pod":"","Config":{{"Entrypoint":"",{}}},"HostConfig":{{"RestartPolicy":{{"Name":""}}}},"NetworkSettings":{{"Networks":{{}}}},"Mounts":[]}}"#,
            labels(service)
        )
    };
    Ok(vec![
        LibpodResponse::new(
            200,
            LibpodHeaders::new(vec![LibpodHeader::new("Libpod-API-Version", "3.0.0")?]),
            [],
        )?,
        json(r#"{"Components":[{"Name":"Podman Engine","Version":"3.0.1"}]}"#)?,
        json(
            r#"[{"Id":"a-web","Names":["migration-web-1"]},{"Id":"b-worker","Names":["migration-worker-1"]},{"Id":"z-other","Names":["unrelated"]}]"#,
        )?,
        json("[]")?,
        json("[]")?,
        json("[]")?,
        json(r#"[{"Id":"sha256:legacy","Names":["example.invalid/legacy:1"]}]"#)?,
        json(&selected("a-web", "migration-web-1", "web"))?,
        json(&selected("b-worker", "migration-worker-1", "worker"))?,
        json(
            r#"{"Id":"z-other","Name":"unrelated","ImageName":"example.invalid/missing:1","Pod":"","Config":{"Entrypoint":"","Labels":{"com.docker.compose.project":"incomplete"}},"HostConfig":{"RestartPolicy":{"Name":""},"NetworkMode":"pasta:--map-gw"},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
        )?,
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
    legacy_source_with_options(responses, request, AcquisitionOptions::redacted())
}

fn legacy_source_with_options(
    responses: Vec<LibpodResponse>,
    request: &DiscoveryRequest,
    options: AcquisitionOptions,
) -> Result<PodmanSource, Box<dyn Error>> {
    let transport = FixtureTransport::new(responses);
    let inventory = block_on(acquire_inventory(&transport, options))?;
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
    assert_eq!(field("native_path_samples_shown"), Some("8"));
    assert_eq!(
        field("native_value_policy"),
        Some("field paths retained; native values not retained")
    );
    assert_eq!(
        field("native_path_samples"),
        Some("$.Unknown00, $.Unknown01, $.Unknown02, $.Unknown03, $.Unknown04, $.Unknown05, $.Unknown06, $.Unknown07")
    );
    let native = diagnostic.native_finding().ok_or("native finding")?;
    assert!(
        [
            "occurrence_count",
            "native_path_count",
            "native_path_samples",
            "native_path_samples_shown",
            "native_value_policy",
        ]
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
fn native_field_limits_explain_retained_paths_without_retaining_values() -> Result<(), Box<dyn Error>> {
    let mut inspect = serde_json::json!({
        "Id": "c-web",
        "Name": "web",
        "ImageName": "example.invalid/legacy:1",
        "Pod": "",
        "Config": {"Entrypoint": ""},
        "HostConfig": {"RestartPolicy": {"Name": ""}},
        "NetworkSettings": {"Networks": {}},
        "Mounts": []
    });
    let object = inspect.as_object_mut().ok_or("inspect object")?;
    for index in 0..=podman_lens::MAX_UNKNOWN_FIELDS_PER_RECORD {
        object.insert(format!("SyntheticUnknown{index:03}"), serde_json::Value::Bool(true));
    }
    let source = legacy_source(&serde_json::to_string(&inspect)?)?;
    let result = PodmanImporter::new()?.import(&source);
    let diagnostic = result
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.native_finding().is_some_and(|finding| {
                finding.code() == DiagnosticCode::UnknownFieldOverflow.as_str() && finding.stage() == "acquisition"
            })
        })
        .ok_or("bounded unknown-field diagnostic")?;
    let field = |name: &str| {
        diagnostic
            .fields()
            .iter()
            .find(|field| field.name() == name)
            .map(|field| field.value().redacted())
    };
    assert!(diagnostic.summary().contains("diagnostic list of unmapped field paths"));
    assert_eq!(field("retention_limit_per_resource"), Some("128"));
    assert_eq!(field("retention_limit_per_inventory"), Some("2048"));
    assert_eq!(field("retained_native_path_count"), Some("128"));
    assert_eq!(field("discarded_native_path_count_at_least"), Some("1"));
    assert_eq!(field("native_path_samples_shown"), Some("8"));
    assert_eq!(
        field("native_value_policy"),
        Some("field paths retained; native values not retained")
    );
    assert_eq!(
        field("path_descriptor_purpose"),
        Some("audit which Podman response fields were not typed or converted")
    );
    assert_eq!(
        field("conversion_impact"),
        Some("only the diagnostic path catalogue was truncated; typed observations used by conversion remain intact")
    );
    assert!(field("native_path_samples").is_some_and(|samples| samples.contains("$.SyntheticUnknown000")));
    assert!(!format!("{diagnostic:?}").contains("true"));
    Ok(())
}

#[test]
fn local_image_id_diagnostic_explains_that_configured_reference_is_retained() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"localhost/example/team-app:latest","Image":"sha256:local-only","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[]}"#,
    )?;
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert_eq!(
        service.image().map(|image| image.value().as_str()),
        Some("localhost/example/team-app:latest")
    );
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.fields().iter().any(|field| {
            field.name() == "reason"
                && field.value().redacted()
                    == "Podman local image ID is host-local resolution evidence; Image= was copied unchanged from Podman inspect $.ImageName"
        })
    }));
    Ok(())
}

#[test]
fn explicit_named_volume_promotion_accepts_configured_mount_identity() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[{"Type":"volume","Name":"legacy-data","Destination":"/data","RW":true,"Options":["z"]}]}"#,
    )?
    .with_promotion_policy(PodmanPromotionPolicy::conservative().with_effective_named_volume_mounts(true));
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert_eq!(service.mounts().len(), 1);
    assert_eq!(service.mounts()[0].value().target(), "/data");
    assert_eq!(
        service.mounts()[0].value().selinux_relabel(),
        Some(SelinuxRelabel::Shared)
    );
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0003" && diagnostic.summary().contains("portable named-volume mount promoted")
    }));
    Ok(())
}

#[test]
fn explicit_bind_mount_promotion_preserves_same_path_core_intent() -> Result<(), Box<dyn Error>> {
    let source = legacy_source(
        r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{}},"Mounts":[{"Type":"bind","Source":"/srv/example/config","Destination":"/etc/example","RW":false,"Options":["rbind","Z"],"Propagation":"rprivate"}]}"#,
    )?
    .with_promotion_policy(PodmanPromotionPolicy::conservative().with_effective_bind_mounts(true));

    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    let service = application.services().first().ok_or("legacy service")?.value();
    assert_eq!(service.mounts().len(), 1);
    assert!(matches!(
        service.mounts()[0].value().source(),
        MountSource::HostPath(path) if path == "/srv/example/config"
    ));
    assert_eq!(service.mounts()[0].value().target(), "/etc/example");
    assert!(service.mounts()[0].value().read_only());
    assert_eq!(
        service.mounts()[0].value().selinux_relabel(),
        Some(SelinuxRelabel::Private)
    );
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0003"
            && diagnostic.fields().iter().any(|field| {
                field.name() == "available_promotion"
                    && field.value().redacted() == "--promote-podman-effective-bind-mounts"
            })
    }));
    Ok(())
}

#[test]
fn combined_promotion_flags_model_network_ipam_and_keep_effective_empty_dns_unmodelled() -> Result<(), Box<dyn Error>> {
    let inspect = r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":"","Cmd":["nginx","-g","daemon off;"],"Env":["NC_davstorage.request_timeout=60 seconds"]},"HostConfig":{"RestartPolicy":{"Name":""},"Dns":[],"DnsSearch":[],"DnsOptions":[]},"NetworkSettings":{"Networks":{"legacy-net":{"NetworkID":"legacy-net"}}},"Mounts":[{"Type":"bind","Source":"/srv/example/config","Destination":"/etc/example","RW":true,"Propagation":"rprivate"}]}"#;
    let mut request = DiscoveryRequest::new();
    request.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    let source = legacy_source_with_options(
        modern_responses(inspect)?,
        &request,
        AcquisitionOptions::include_environment_values(),
    )?
    .with_promotion_policy(
        PodmanPromotionPolicy::conservative()
            .with_effective_bind_mounts(true)
            .with_effective_named_volume_mounts(true)
            .with_effective_named_networks(true)
            .with_portable_effective_settings(true),
    );

    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("legacy application")?;
    assert_eq!(application.networks().len(), 1);
    let network = application.networks()[0].value();
    assert_eq!(network.ownership(), ResourceOwnership::Application);
    assert_eq!(network.internal().map(|value| *value.value()), Some(true));
    assert_eq!(network.ipv6().map(|value| *value.value()), Some(true));
    let ipam = network.ipam_configs().ok_or("promoted network IPAM")?;
    assert_eq!(ipam.len(), 2);
    assert_eq!(ipam[0].value().subnet().value().expose(), "10.88.0.0/16");
    assert_eq!(
        ipam[0].value().gateway().map(|value| value.value().expose()),
        Some("10.88.0.1")
    );
    assert_eq!(
        ipam[0].value().ip_range().map(|value| value.value().expose()),
        Some("10.88.1.0-10.88.1.255")
    );
    assert_eq!(ipam[1].value().subnet().value().expose(), "fd42::/64");
    assert_eq!(
        ipam[1].value().gateway().map(|value| value.value().expose()),
        Some("fd42::1")
    );
    assert_eq!(
        ipam[1].value().ip_range().map(|value| value.value().expose()),
        Some("fd42::100-fd42::1ff")
    );
    let service = application.services().first().ok_or("legacy service")?.value();
    assert!(service.dns_servers().is_none());
    assert!(service.dns_search_domains().is_none());
    assert!(service.dns_options().is_none());
    assert_eq!(service.mounts().len(), 1);
    assert_eq!(service.environment().len(), 1);

    assert!(!result.diagnostics().iter().any(|diagnostic| {
        diagnostic.fields().iter().any(|field| {
            field.name() == "reason"
                && field.value().redacted()
                    == "effective network subnet and address-allocation settings need explicit promotion policy"
        })
    }));
    Ok(())
}

#[test]
fn arbitrary_network_lease_endpoints_are_preserved_as_an_ip_range() -> Result<(), Box<dyn Error>> {
    let inspect = r#"{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{"Entrypoint":""},"HostConfig":{"RestartPolicy":{"Name":""}},"NetworkSettings":{"Networks":{"legacy-net":{"NetworkID":"legacy-net"}}},"Mounts":[]}"#;
    let mut responses = modern_responses(inspect)?;
    responses[9] = json(
        r#"{"id":"legacy-net","name":"legacy-net","internal":false,"subnets":[{"subnet":"10.88.0.0/16","gateway":"10.88.0.1","lease_range":{"start_ip":"10.88.1.10","end_ip":"10.88.1.20"}}]}"#,
    )?;
    let mut request = DiscoveryRequest::new();
    request.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    let source = legacy_source_with_options(responses, &request, AcquisitionOptions::redacted())?
        .with_promotion_policy(
            PodmanPromotionPolicy::conservative()
                .with_effective_named_networks(true)
                .with_portable_effective_settings(true),
        );

    let result = PodmanImporter::new()?.import(&source);
    let network = result
        .application()
        .ok_or("legacy application")?
        .networks()
        .first()
        .ok_or("legacy network")?
        .value();
    let ipam = network
        .ipam_configs()
        .and_then(|rows| rows.first())
        .ok_or("network IPAM row")?
        .value();
    assert_eq!(
        ipam.ip_range().map(|range| range.value().expose()),
        Some("10.88.1.10-10.88.1.20")
    );
    Ok(())
}

#[test]
fn portable_effective_promotion_retains_reviewed_settings_and_redacts_evidence() -> Result<(), Box<dyn Error>> {
    const SECRET: &str = "portable-environment-canary-never-report";
    let inspect = format!(
        r#"{{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{{"Entrypoint":"","Env":["APP_SECRET={SECRET}"],"Labels":{{"com.docker.compose.project":"legacy","com.docker.compose.service":"web","io.boxferry.keep":"yes"}},"Healthcheck":{{"Test":["CMD","/bin/check"],"Interval":30000000000,"Timeout":5000000000,"Retries":3,"StartPeriod":1000000000}}}},"HostConfig":{{"RestartPolicy":{{"Name":"always","MaximumRetryCount":0}},"PortBindings":{{"8080/tcp":[{{"HostIp":"127.0.0.1","HostPort":"18080"}}]}},"Dns":["192.0.2.53"],"DnsSearch":["example.test"],"DnsOptions":["ndots:2"]}},"NetworkSettings":{{"Networks":{{}}}},"Mounts":[]}}"#
    );
    let mut request = DiscoveryRequest::new();
    request.add_root(ResourceSelector::exact(ResourceKind::Container, "web")?);
    let source = legacy_source_with_options(
        legacy_responses(&inspect)?,
        &request,
        AcquisitionOptions::include_environment_values(),
    )?
    .with_promotion_policy(PodmanPromotionPolicy::conservative().with_portable_effective_settings(true));

    let result = PodmanImporter::new()?.import(&source);
    let service = result
        .application()
        .ok_or("legacy application")?
        .services()
        .first()
        .ok_or("legacy service")?
        .value();
    assert_eq!(service.environment().len(), 1);
    let EnvironmentValue::Literal(value) = service.environment()[0].value().value() else {
        return Err("expected literal environment value".into());
    };
    assert!(value.is_sensitive());
    assert_eq!(value.expose(), SECRET);
    assert_eq!(
        service.restart_policy().map(|value| *value.value()),
        Some(RestartPolicy::Always)
    );
    let healthcheck = service.healthcheck().ok_or("promoted healthcheck")?.value();
    assert!(matches!(
        healthcheck.command().map(boxferry_model::Sourced::value),
        Some(HealthcheckCommand::Exec(arguments)) if arguments.len() == 1 && arguments[0].expose() == "/bin/check"
    ));
    assert_eq!(service.ports().len(), 1);
    assert_eq!(service.ports()[0].value().container(), 8080);
    assert_eq!(service.ports()[0].value().published(), Some(18_080));
    assert_eq!(service.ports()[0].value().host_address(), Some("127.0.0.1"));
    assert_eq!(
        service
            .dns_servers()
            .and_then(|values| values.first())
            .map(|value| value.value().expose()),
        Some("192.0.2.53")
    );
    assert_eq!(
        service
            .labels()
            .iter()
            .map(|label| label.value().name().as_str())
            .collect::<Vec<_>>(),
        ["io.boxferry.keep"]
    );
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0003"
            && diagnostic.fields().iter().any(|field| {
                field.name() == "available_promotion"
                    && field.value().redacted() == "--promote-podman-portable-effective-settings"
            })
    }));

    let inventory_snapshot = serde_json::to_string(&source.redacted_inventory_snapshot())?;
    let graph_snapshot = serde_json::to_string(&source.redacted_graph_snapshot())?;
    assert!(!inventory_snapshot.contains(SECRET));
    assert!(!graph_snapshot.contains(SECRET));
    assert!(!format!("{source:?}").contains(SECRET));
    Ok(())
}

#[test]
fn redacted_acquisition_never_promotes_environment_values() -> Result<(), Box<dyn Error>> {
    const SECRET: &str = "redacted-environment-canary-never-retain";
    let inspect = format!(
        r#"{{"Id":"c-web","Name":"web","ImageName":"example.invalid/legacy:1","Pod":"","Config":{{"Entrypoint":"","Env":["APP_SECRET={SECRET}"]}},"HostConfig":{{"RestartPolicy":{{"Name":""}}}},"NetworkSettings":{{"Networks":{{}}}},"Mounts":[]}}"#
    );
    let source = legacy_source(&inspect)?
        .with_promotion_policy(PodmanPromotionPolicy::conservative().with_portable_effective_settings(true));
    let result = PodmanImporter::new()?.import(&source);
    let service = result
        .application()
        .ok_or("legacy application")?
        .services()
        .first()
        .ok_or("legacy service")?
        .value();
    assert!(service.environment().is_empty());
    assert!(result.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().as_str() == "BFP0002"
            && diagnostic.fields().iter().any(|field| {
                field.name() == "reason" && field.value().redacted().contains("environment value remained redacted")
            })
    }));
    assert!(!serde_json::to_string(&source.redacted_inventory_snapshot())?.contains(SECRET));
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
fn selected_group_ignores_unrelated_discovery_findings_and_consumes_compose_labels() -> Result<(), Box<dyn Error>> {
    let mut request = DiscoveryRequest::new();
    request.add_root(ResourceSelector::exact(ResourceKind::Container, "migration-web-1")?);
    let source = legacy_source_with_request(grouped_selection_with_unrelated_responses()?, &request)?;
    let result = PodmanImporter::new()?.import(&source);
    let application = result.application().ok_or("selected application")?;
    assert_eq!(
        application.services().len(),
        2,
        "Compose ownership evidence expands the selected group"
    );
    for service in application.services() {
        assert_eq!(service.value().labels().len(), 1);
        assert_eq!(service.value().labels()[0].value().name().as_str(), "io.boxferry.keep");
    }
    assert!(result.diagnostics().iter().all(|diagnostic| {
        diagnostic
            .native_finding()
            .is_none_or(|finding| !matches!(finding.code(), "PLN0024" | "PLN0030" | "PLN0031"))
    }));
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
