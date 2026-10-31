use super::*;
use boxferry_engine::{ConversionKind, ExportAdapter, LossPolicy};
use boxferry_model::{
    EnvironmentVariable, ExposedPort, Healthcheck as NeutralHealthcheck, HealthcheckDuration, HealthcheckRetries,
    Identifier, ImageReference as NeutralImageReference, MetadataLabel, Mount as NeutralMount, Network,
    NetworkAttachment, Port as NeutralPort, ProtectedString, Provenance, PullPolicy, SelinuxRelabel, ServiceDependency,
    SourceId, Sourced, Volume,
};
use docker_lens::version::{
    ApiVersion, CapabilityEvidenceKey, DaemonMode, DebianPackageRevision, TargetProfile as NativeTargetProfile,
    TargetProfileIdentity,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn reviewed_profile() -> TestResult<NativeTargetProfile> {
    Ok(TargetCapabilityCatalog::reviewed()
        .profiles()
        .find(|profile| {
            matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootful
        })
        .ok_or("reviewed upstream rootful target is missing")?
        .clone())
}

fn target(profile: &NativeTargetProfile) -> TestResult<TargetProfile> {
    let version = numeric_engine_release(profile).ok_or("reviewed release has no numeric compatibility version")?;
    Ok(TargetProfile::new(DOCKER_TARGET, version, Some(version))?)
}

fn service(name: &str, image: Option<&str>) -> TestResult<Service> {
    let mut service = Service::new(Identifier::new(name)?);
    if let Some(image) = image {
        service.set_image(Sourced::generated(NeutralImageReference::parse(image)?));
    }
    Ok(service)
}

fn application(service: Service) -> TestResult<Application> {
    let mut application = Application::new(Identifier::new("example")?);
    application.add_service(Sourced::generated(service))?;
    Ok(application)
}

fn forged_api_profile(
    profile: &NativeTargetProfile,
    advertised: ApiVersion,
    acquisition: ApiVersion,
    rendering: ApiVersion,
) -> TestResult<NativeTargetProfile> {
    let identity = TargetProfileIdentity::new(
        profile.identity().build().clone(),
        profile.release().clone(),
        advertised,
        acquisition,
        rendering,
        profile.mode(),
    )
    .map_err(|error| format!("{error:?}"))?;
    Ok(NativeTargetProfile::new(identity, profile.evidence_key().clone()))
}

#[test]
fn exact_minimal_request_retains_full_reviewed_profile() -> TestResult {
    let profile = reviewed_profile()?;
    let exporter = DockerExporter::new(profile.clone())?;
    let planned = exporter.plan(&application(service("web", Some("busybox:1"))?)?, &target(&profile)?)?;
    let output = planned.candidate().ok_or("reviewed standalone request missing")?;
    let api = profile.rendering_api_version();
    let expected = format!(
        "{{\"method\":\"POST\",\"path\":\"/v{}.{}/containers/create?name=web\",\"body\":{{\"Image\":\"busybox:1\",\"HostConfig\":{{}}}}}}\n",
        api.major.get(),
        api.minor,
    );
    assert_eq!(output.bytes(), expected.as_bytes());
    assert_eq!(output.native_profile(), &profile);
    assert_eq!(output.evidence().record_key(), profile.evidence_key());
    assert!(output.network_prerequisites().is_empty());
    assert!(output.volume_prerequisites().is_empty());
    let complete = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
    let complete = std::str::from_utf8(&complete)?;
    assert!(complete.contains("\"schema_version\":1"));
    assert!(complete.contains("\"kind\":\"target\""));
    assert!(complete.contains(&format!("\"engine_release\":\"{}\"", profile.release().as_str())));
    assert!(complete.contains("\"requests\":[{"));
    assert!(complete.contains("\"prerequisites\":[]"));
    assert_eq!(
        output.evidence().lane().artifact_name(),
        output.evidence().artifact_name()
    );
    assert!(
        planned
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    assert!(planned.authorize(LossPolicy::ExactOnly).output().is_some());
    Ok(())
}

#[test]
fn reviewed_standalone_volume_port_exec_health_and_restart_are_inert() -> TestResult {
    let profile = reviewed_profile()?;
    let mut service = service("web", Some("busybox:1"))?;
    service.set_command(Sourced::generated(Command::Exec(vec![ProtectedString::plain(
        "--serve",
    )])));
    service.set_entrypoint(Sourced::generated(Entrypoint::Exec(vec![ProtectedString::plain(
        "/bin/app",
    )])));
    service.add_port(Sourced::generated(NeutralPort::new(
        80,
        Some(8080),
        None,
        Protocol::Tcp,
    )?));
    service.add_mount(Sourced::generated(NeutralMount::new(
        MountSource::Volume(Identifier::new("data")?),
        "/data",
        true,
    )?));
    service.set_restart_policy(Sourced::generated(NeutralRestartPolicy::Always));
    let mut health = NeutralHealthcheck::new();
    health.set_command(Sourced::generated(HealthcheckCommand::Exec(vec![
        ProtectedString::plain("/bin/check"),
    ])));
    health.set_interval(Sourced::generated(HealthcheckDuration::new("1s")?));
    health.set_timeout(Sourced::generated(HealthcheckDuration::new("500ms")?));
    health.set_retries(Sourced::generated(HealthcheckRetries::new("3")?));
    service.set_healthcheck(Sourced::generated(health));
    let mut app = application(service)?;
    app.add_volume(Sourced::generated(Volume::new(
        Identifier::new("data")?,
        ResourceOwnership::Application,
    )))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    let output = planned.candidate().ok_or("reviewed plan missing")?;
    let text = std::str::from_utf8(output.bytes())?;
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("/volumes/create"));
    assert!(lines[1].contains("/containers/create?name=web"));
    assert!(lines[1].contains("\"PortBindings\":{\"80/tcp\":[{\"HostPort\":\"8080\"}]"));
    assert!(lines[1].contains("\"Type\":\"volume\",\"Source\":\"data\""));
    assert!(lines[1].contains("\"Cmd\":[\"--serve\"]"));
    assert!(lines[1].contains("\"Entrypoint\":[\"/bin/app\"]"));
    assert!(lines[1].contains("\"Test\":[\"CMD\",\"/bin/check\"]"));
    assert!(lines[1].contains("\"RestartPolicy\":{\"Name\":\"always\",\"MaximumRetryCount\":0}"));
    assert!(
        planned
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(())
}

#[test]
fn missing_health_start_interval_reports_exact_feature_without_values() -> TestResult {
    let catalogue = TargetCapabilityCatalog::reviewed();
    let profiles: Vec<_> = catalogue
        .profiles()
        .filter(|profile| {
            profile.rendering_api_version().major.get() == 1 && profile.rendering_api_version().minor == 41
        })
        .collect();
    assert!(!profiles.is_empty(), "reviewed API1.41 negative boundary is missing");
    for profile in profiles {
        for duration in ["0s", "1s"] {
            let mut web = service("private-resource-canary", Some("busybox:1"))?;
            let mut health = NeutralHealthcheck::new();
            health.set_command(Sourced::generated(HealthcheckCommand::Exec(vec![
                ProtectedString::plain("private-command-canary"),
            ])));
            web.set_healthcheck(Sourced::generated(health.clone()));
            let exporter = DockerExporter::new(profile.clone())?;
            let baseline = exporter.plan(&application(web.clone())?, &target(profile)?)?;
            assert!(
                baseline.candidate().is_some(),
                "ordinary healthcheck must remain admitted"
            );
            health.set_start_interval(Sourced::generated(HealthcheckDuration::new(duration)?));
            web.set_healthcheck(Sourced::generated(health));
            let app = application(web)?;
            let planned = exporter.plan(&app, &target(profile)?)?;
            assert!(planned.candidate().is_none());
            let features: Vec<_> = planned
                .diagnostics()
                .iter()
                .flat_map(Diagnostic::fields)
                .filter(|field| field.name() == "native_feature")
                .map(|field| field.value().expose())
                .collect();
            assert_eq!(features, ["HealthStartInterval"]);
            assert!(planned.outcomes().iter().any(|outcome| outcome.subject()
                == "services.private-resource-canary.healthcheck"
                && outcome.kind() == ConversionKind::Invalid));
            assert!(!format!("{:?}", planned.diagnostics()).contains("private-command-canary"));
            for policy in [
                LossPolicy::ExactOnly,
                LossPolicy::AllowApproximate,
                LossPolicy::AllowPartial,
            ] {
                assert!(exporter.plan(&app, &target(profile)?)?.authorize(policy).is_blocked());
            }
        }
    }
    Ok(())
}

#[test]
fn explicit_volume_destination_controls_create_or_prerequisite_and_exact_remount() -> TestResult {
    let profile = reviewed_profile()?;
    for ownership in [ResourceOwnership::Application, ResourceOwnership::External] {
        let mut web = service("web", Some("busybox:1"))?;
        web.add_mount(Sourced::generated(NeutralMount::new(
            MountSource::Volume(Identifier::new("docker-volume-1")?),
            "/data",
            false,
        )?));
        let mut app = application(web)?;
        let mut volume = Volume::new(Identifier::new("docker-volume-1")?, ownership);
        volume.set_runtime_name(Sourced::generated(ProtectedString::plain("destination-volume")));
        app.add_volume(Sourced::generated(volume))?;
        let plan = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
        let output = plan.candidate().ok_or("explicit destination plan missing")?;
        let complete = String::from_utf8(output.complete_bytes().map_err(|error| format!("{error:?}"))?)?;
        let expected_creates = usize::from(ownership == ResourceOwnership::Application);
        assert_eq!(complete.matches("/volumes/create").count(), expected_creates);
        if ownership == ResourceOwnership::Application {
            assert!(complete.contains("\"Name\":\"destination-volume\""));
            assert!(output.volume_prerequisites().is_empty());
        } else {
            assert_eq!(output.volume_prerequisites().len(), 1);
            assert!(complete.contains("\"identity\":\"destination-volume\""));
        }
        assert!(complete.contains("\"Source\":\"destination-volume\""));
        assert!(!complete.contains("docker-volume-1"));
    }
    Ok(())
}

#[test]
fn reviewed_catalogue_retains_configured_bind_relabel_with_unverified_obligations() -> TestResult {
    use docker_lens::target::{BindRelabelCondition, BindSourceCondition};

    let catalogue = TargetCapabilityCatalog::reviewed();
    assert_eq!(catalogue.profiles().count(), 4);
    for profile in catalogue.profiles() {
        for (read_only, relabel, native_relabel, mode) in [
            (false, SelinuxRelabel::Shared, BindRelabel::Shared, "rw,z"),
            (true, SelinuxRelabel::Shared, BindRelabel::Shared, "ro,z"),
            (false, SelinuxRelabel::Private, BindRelabel::Private, "rw,Z"),
            (true, SelinuxRelabel::Private, BindRelabel::Private, "ro,Z"),
        ] {
            let mut service = service("web", Some("busybox:1"))?;
            let mut mount = NeutralMount::new(MountSource::HostPath("/srv/data".into()), "/data", read_only)?;
            mount.set_selinux_relabel(relabel);
            service.add_mount(Sourced::generated(mount));
            let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(profile)?)?;
            // Admission proves configured retention only, never source suitability or real
            // SELinux effects. The caller still owes every serialized source/SELinux obligation.
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .all(|outcome| outcome.kind() == ConversionKind::Exact)
            );
            assert!(planned.outcomes().iter().any(|outcome| {
                outcome.subject() == "services.web.mounts.0.selinux_relabel" && outcome.kind() == ConversionKind::Exact
            }));
            let output = planned.candidate().ok_or("reviewed bind artifact missing")?;
            assert_eq!(output.native_profile(), profile);
            assert_eq!(output.evidence().record_key(), profile.evidence_key());
            assert!(output.network_prerequisites().is_empty());
            assert!(output.volume_prerequisites().is_empty());
            let prerequisites = output.artifact.bind_source_prerequisites();
            assert_eq!(prerequisites.len(), 1);
            let prerequisite = &prerequisites[0];
            assert_eq!(prerequisite.identity(), b"web");
            assert_eq!(prerequisite.mount_index, 0);
            assert_eq!(prerequisite.source(), b"/srv/data");
            assert_eq!(prerequisite.target(), b"/data");
            assert_eq!(prerequisite.read_only(), read_only);
            assert_eq!(prerequisite.relabel(), native_relabel);
            assert_eq!(
                prerequisite.source_conditions(),
                &[
                    BindSourceCondition::Exists,
                    BindSourceCondition::TypeReviewed,
                    BindSourceCondition::ContentsReviewed,
                    BindSourceCondition::OwnershipReviewed,
                    BindSourceCondition::PermissionsReviewed
                ]
            );
            assert_eq!(
                prerequisite.selinux_conditions(),
                &[
                    BindRelabelCondition::DaemonSelinuxEnabled,
                    BindRelabelCondition::ContainerMountLabelPresent,
                    BindRelabelCondition::PolicyFilesystemSupport,
                    BindRelabelCondition::RelabelAuthority
                ]
            );
            let api = profile.rendering_api_version();
            let request = format!(
                "{{\"method\":\"POST\",\"path\":\"/v{}.{}/containers/create?name=web\",\"body\":{{\"Image\":\"busybox:1\",\"HostConfig\":{{\"Binds\":[\"/srv/data:/data:{mode}\"]}}}}}}",
                api.major.get(),
                api.minor
            );
            assert_eq!(output.bytes(), format!("{request}\n").as_bytes());
            let complete = output
                .complete_bytes()
                .map_err(|_| "complete bind artifact unavailable")?;
            let complete = std::str::from_utf8(&complete)?;
            assert!(complete.starts_with("{\"schema_version\":2,\"context\":{\"kind\":\"target\","));
            assert!(complete.contains(&format!("\"requests\":[{request}],\"prerequisites\":[")));
            // Use the opaque typed reference, not an assumed request index or allocation order.
            let reference = prerequisite.reference.local_index();
            let relabel = if native_relabel == BindRelabel::Private {
                "private"
            } else {
                "shared"
            };
            let row = format!(
                "{{\"kind\":\"bind_source\",\"reference\":\"{reference}\",\"identity\":\"web\",\"mount_index\":\"0\",\"source\":\"/srv/data\",\"target\":\"/data\",\"read_only\":{read_only},\"relabel\":\"{relabel}\",\"source_conditions\":[\"exists\",\"type_reviewed\",\"contents_reviewed\",\"ownership_reviewed\",\"permissions_reviewed\"],\"selinux_effect\":\"unverified\",\"selinux_conditions\":[\"daemon_selinux_enabled\",\"container_mount_label_present\",\"policy_filesystem_support\",\"relabel_authority\"]}}"
            );
            assert!(complete.ends_with(&format!("\"prerequisites\":[{row}]}}\n")));
            let presentation = format!(
                "{output:?} {prerequisite:?} {:?} {:?}",
                planned.outcomes(),
                planned.diagnostics()
            );
            assert!(!presentation.contains("/srv/data"));
            assert!(!presentation.contains("/data"));
            assert!(!planned.authorize(LossPolicy::ExactOnly).is_blocked());
        }
    }
    Ok(())
}

#[test]
fn typed_bind_mapping_preserves_access_relabel_and_private_native_paths() -> TestResult {
    for (read_only, relabel, expected) in [
        (true, SelinuxRelabel::Shared, BindRelabel::Shared),
        (false, SelinuxRelabel::Shared, BindRelabel::Shared),
        (true, SelinuxRelabel::Private, BindRelabel::Private),
        (false, SelinuxRelabel::Private, BindRelabel::Private),
    ] {
        let mut mount = NeutralMount::new(
            MountSource::HostPath("/fake/private-source-canary".into()),
            "/fake/private-target-canary",
            read_only,
        )?;
        mount.set_selinux_relabel(relabel);
        let native = native_bind_mount(&mount).map_err(|error| format!("{error:?}"))?;
        assert_eq!(native.read_only(), read_only);
        assert_eq!(native.bind_relabel(), Some(expected));
        assert_eq!(native.target(), b"/fake/private-target-canary");
        assert!(
            matches!(native.source(), docker_lens::target::MountSource::Bind(source) if source.as_bytes() == b"/fake/private-source-canary")
        );
        assert!(!format!("{native:?}").contains("canary"));
    }
    Ok(())
}

#[test]
fn relabel_mapping_refuses_non_binds_and_legacy_colon_paths() -> TestResult {
    let profile = reviewed_profile()?;
    for source in [MountSource::Volume(Identifier::new("data")?), MountSource::Anonymous] {
        let mut mount = NeutralMount::new(source, "/data", false)?;
        mount.set_selinux_relabel(SelinuxRelabel::Shared);
        assert!(native_bind_mount(&mount).is_err());
        let mut service = service("web", Some("busybox:1"))?;
        service.add_mount(Sourced::generated(mount));
        let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(&profile)?)?;
        assert!(planned.candidate().is_none());
        assert!(
            planned
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "services.web.mounts.0.selinux_relabel"
                    && outcome.kind() == ConversionKind::Invalid)
        );
    }
    for (source, container_target) in [("/fake/source:canary", "/data"), ("/fake/source", "/data:canary")] {
        let mut mount = NeutralMount::new(MountSource::HostPath(source.into()), container_target, false)?;
        // Structured plain binds still admit colons; only legacy relabel syntax cannot.
        assert!(native_bind_mount(&mount).is_ok());
        mount.set_selinux_relabel(SelinuxRelabel::Private);
        assert!(native_bind_mount(&mount).is_err());
        let mut service = service("web", Some("busybox:1"))?;
        service.add_mount(Sourced::generated(mount));
        let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(&profile)?)?;
        assert!(planned.candidate().is_none());
        assert!(
            planned.outcomes().iter().any(
                |outcome| outcome.subject() == "services.web.mounts.0" && outcome.kind() == ConversionKind::Invalid
            )
        );
        assert!(!format!("{:?}", planned.diagnostics()).contains("canary"));
    }
    Ok(())
}

#[test]
fn literal_environment_requires_independent_output_opt_in_and_debug_stays_redacted() -> TestResult {
    let profile = reviewed_profile()?;
    let mut service = service("web", Some("busybox:1"))?;
    service.add_environment(Sourced::generated(EnvironmentVariable::new(
        Identifier::new("TOKEN")?,
        EnvironmentValue::Literal(ProtectedString::sensitive("needle-value")),
    )));
    let app = application(service)?;
    let withheld = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(
        withheld
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.environment.TOKEN"
                && outcome.kind() == ConversionKind::Unsupported)
    );
    assert!(
        !withheld
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web" && outcome.kind() == ConversionKind::Exact)
    );
    assert!(
        withheld.candidate().is_none(),
        "withheld value must not leave a complete-looking request"
    );
    assert!(withheld.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.container" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(!format!("{:?}", withheld.diagnostics()).contains("needle-value"));
    assert!(withheld.authorize(LossPolicy::ExactOnly).is_blocked());
    let partial = DockerExporter::new(profile.clone())?
        .plan(&app, &target(&profile)?)?
        .authorize(LossPolicy::AllowPartial);
    assert!(
        partial.output().is_none(),
        "partial policy cannot publish a missing required assignment"
    );
    let included = DockerExporter::new(profile.clone())?
        .with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral)
        .plan(&app, &target(&profile)?)?;
    let output = included.candidate().ok_or("included candidate missing")?;
    assert!(std::str::from_utf8(output.bytes())?.contains("TOKEN=needle-value"));
    assert!(!format!("{output:?}").contains("needle-value"));
    assert!(included.authorize(LossPolicy::ExactOnly).output().is_some());

    let mut required_service = self::service("required", Some("busybox:1"))?;
    required_service.add_environment(Sourced::generated(EnvironmentVariable::new(
        Identifier::new("TOKEN")?,
        EnvironmentValue::Required,
    )));
    let required = DockerExporter::new(profile.clone())?
        .with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral)
        .plan(&application(required_service)?, &target(&profile)?)?;
    assert!(
        required.candidate().is_none(),
        "target value consent cannot resolve an absent source value"
    );
    assert!(required.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.required.environment.TOKEN" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn missing_image_and_uncertain_volume_ownership_cannot_emit_incomplete_service() -> TestResult {
    let profile = reviewed_profile()?;
    let exporter = DockerExporter::new(profile.clone())?;
    let missing = exporter.plan(&application(service("web", None)?)?, &target(&profile)?)?;
    assert!(missing.candidate().is_none());
    assert!(
        missing
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.image")
    );

    for ownership in [ResourceOwnership::External, ResourceOwnership::Uncertain] {
        let mut service = service("web", Some("busybox:1"))?;
        service.add_mount(Sourced::generated(NeutralMount::new(
            MountSource::Volume(Identifier::new("data")?),
            "/data",
            false,
        )?));
        let mut app = application(service)?;
        app.add_volume(Sourced::generated(Volume::new(Identifier::new("data")?, ownership)))?;
        let planned = exporter.plan(&app, &target(&profile)?)?;
        if ownership == ResourceOwnership::External {
            let output = planned.candidate().ok_or("external volume plan missing")?;
            assert_eq!(output.volume_prerequisites().len(), 1);
            assert!(planned.outcomes().iter().any(|outcome| {
                outcome.subject() == "services.web.mounts.0" && outcome.kind() == ConversionKind::Exact
            }));
        } else {
            assert!(planned.candidate().is_none());
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "volumes.data.ownership")
            );
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "services.web.mounts.0")
            );
        }
    }
    Ok(())
}

#[test]
fn unresolved_parameterized_image_cannot_become_a_native_create_request() -> TestResult {
    let profile = reviewed_profile()?;
    let exporter = DockerExporter::new(profile.clone())?;
    let target = target(&profile)?;
    for image in [
        "${IMAGE}",
        "$IMAGE",
        "$_IMAGE",
        "registry.example/${REPOSITORY}:1",
        "registry.example/team/app:$TAG",
        "app:${TAG:-stable}",
    ] {
        let origin = Provenance::source(SourceId::new("image-source")?);
        let mut web = service("web", None)?;
        web.set_image(Sourced::from_source(
            NeutralImageReference::parse(image)?,
            origin.clone(),
        ));
        let mut app = application(web)?;
        app.add_service(Sourced::generated(service("unrelated", Some("busybox:1"))?))?;
        let planned = exporter.plan(&app, &target)?;
        assert!(
            planned.candidate().is_none(),
            "unresolved image must not become create intent"
        );
        let rejected = planned
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == "services.web.image")
            .ok_or("unresolved image rejection missing")?;
        assert_eq!(rejected.kind(), ConversionKind::Invalid);
        assert_eq!(rejected.origins(), &[origin]);
        assert!(
            !planned.outcomes().iter().any(|outcome| {
                outcome.subject() == "services.web.image" && outcome.kind() == ConversionKind::Exact
            })
        );
        assert!(planned.diagnostics().iter().any(|diagnostic| {
            Some(diagnostic.code()) == rejected.diagnostic()
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().redacted() == "services.web.image")
        }));
        assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());
    }

    let app = application(service("web", Some("registry.example/team/app:1.2"))?)?;
    let resolved = exporter.plan(&app, &target)?;
    let output = resolved.candidate().ok_or("resolved literal image request missing")?;
    assert!(std::str::from_utf8(output.bytes())?.contains("\"Image\":\"registry.example/team/app:1.2\""));
    assert!(
        resolved
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "services.web.image" && outcome.kind() == ConversionKind::Exact })
    );
    assert!(resolved.authorize(LossPolicy::ExactOnly).output().is_some());
    Ok(())
}

#[test]
fn authored_volume_labels_are_native_create_intent_not_external_mutation() -> TestResult {
    let profile = reviewed_profile()?;
    let mut owned = Volume::new(Identifier::new("data")?, ResourceOwnership::Application);
    owned.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new("owner")?,
        ProtectedString::plain("example"),
    )));
    let mut app = application(service("web", Some("busybox:1"))?)?;
    app.add_volume(Sourced::generated(owned))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    let output = planned.candidate().ok_or("labelled volume plan missing")?;
    assert!(std::str::from_utf8(output.bytes())?.contains("\"Labels\":{\"owner\":\"example\"}"));
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "volumes.data.labels" && outcome.kind() == ConversionKind::Exact })
    );

    let exporter = DockerExporter::new(profile.clone())?;
    let target = target(&profile)?;
    for (ownership, value, subject) in [
        (
            ResourceOwnership::External,
            ProtectedString::plain("example"),
            "volumes.data.labels",
        ),
        (
            ResourceOwnership::Application,
            ProtectedString::sensitive("private"),
            "volumes.data.labels.0",
        ),
    ] {
        let mut volume = Volume::new(Identifier::new("data")?, ownership);
        volume.add_label(Sourced::generated(MetadataLabel::new(Identifier::new("owner")?, value)));
        let mut app = application(service("web", Some("busybox:1"))?)?;
        app.add_volume(Sourced::generated(volume))?;
        let planned = exporter.plan(&app, &target)?;
        // An unrelated container is a safe preview, not permission to ignore this loss.
        let output = planned.candidate().ok_or("unrelated safe container preview missing")?;
        assert!(output.volume_prerequisites().is_empty());
        let requests = std::str::from_utf8(output.bytes())?;
        assert!(requests.contains("/containers/create"));
        assert!(!requests.contains("/volumes/create"));
        assert!(!requests.contains("private"));
        let unsupported = planned
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == subject)
            .ok_or("volume label loss missing")?;
        assert_eq!(unsupported.kind(), ConversionKind::Unsupported);
        assert!(planned.diagnostics().iter().any(|diagnostic| {
            Some(diagnostic.code()) == unsupported.diagnostic()
                && diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().redacted() == subject)
        }));
        assert!(!format!("{:?}", planned.diagnostics()).contains("private"));
        for policy in [LossPolicy::ExactOnly, LossPolicy::AllowApproximate] {
            assert!(exporter.plan(&app, &target)?.authorize(policy).is_blocked());
        }
        assert!(planned.authorize(LossPolicy::AllowPartial).output().is_some());

        let mut dependent = service("dependent", Some("busybox:1"))?;
        dependent.add_mount(Sourced::generated(NeutralMount::new(
            MountSource::Volume(Identifier::new("data")?),
            "/data",
            false,
        )?));
        app.add_service(Sourced::generated(dependent))?;
        let attached = exporter.plan(&app, &target)?;
        assert!(attached.candidate().is_none());
        assert!(attached.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.dependent.mounts.0" && outcome.kind() == ConversionKind::Invalid
        }));
        for policy in [
            LossPolicy::ExactOnly,
            LossPolicy::AllowApproximate,
            LossPolicy::AllowPartial,
        ] {
            assert!(exporter.plan(&app, &target)?.authorize(policy).is_blocked());
        }
    }
    Ok(())
}

#[test]
fn no_pull_policy_matches_inert_create_but_acquisition_policy_is_not_invented() -> TestResult {
    let profile = reviewed_profile()?;
    let mut no_pull = service("web", Some("busybox:1"))?;
    no_pull.set_pull_policy(Sourced::generated(PullPolicy::Never));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(no_pull)?, &target(&profile)?)?;
    assert!(planned.candidate().is_some());
    assert!(
        planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.pull_policy" && outcome.kind() == ConversionKind::Exact
        })
    );
    let mut acquire = service("web", Some("busybox:1"))?;
    acquire.set_pull_policy(Sourced::generated(PullPolicy::Always));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(acquire)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.pull_policy" && outcome.kind() == ConversionKind::Unsupported
    }));
    Ok(())
}

#[test]
fn protected_non_environment_values_cannot_enter_native_requests() -> TestResult {
    let profile = reviewed_profile()?;
    let mut command = service("web", Some("busybox:1"))?;
    command.set_command(Sourced::generated(Command::Exec(vec![ProtectedString::sensitive(
        "private-command",
    )])));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(command)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.command")
    );

    let mut label = service("web", Some("busybox:1"))?;
    label.add_label(Sourced::generated(MetadataLabel::new(
        Identifier::new("private")?,
        ProtectedString::sensitive("private-label"),
    )));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(label)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.labels.0")
    );

    let mut setting = service("web", Some("busybox:1"))?;
    setting.set_user(Sourced::generated(ProtectedString::sensitive("private-user")));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(setting)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.user")
    );
    Ok(())
}

#[test]
fn clearing_command_without_explicit_exec_entrypoint_is_a_field_level_rejection() -> TestResult {
    let profile = reviewed_profile()?;
    for entrypoint in [None, Some(Entrypoint::Empty)] {
        let mut web = service("web", Some("private.example/image-canary:1"))?;
        web.set_command(Sourced::generated(Command::Empty));
        if let Some(entrypoint) = entrypoint {
            web.set_entrypoint(Sourced::generated(entrypoint));
        }
        let planned = DockerExporter::new(profile.clone())?.plan(&application(web)?, &target(&profile)?)?;
        assert!(planned.candidate().is_none());
        assert!(planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.command" && outcome.kind() == ConversionKind::Invalid
        }));
        assert!(
            !planned.outcomes().iter().any(|outcome| {
                outcome.subject() == "services.web.command" && outcome.kind() == ConversionKind::Exact
            })
        );
        let diagnostics = format!("{:?}", planned.diagnostics());
        assert!(diagnostics.contains("explicit nonempty exec entrypoint"));
        assert!(!diagnostics.contains("private.example") && !diagnostics.contains("target.docker.intent"));
    }
    Ok(())
}

#[test]
fn ambiguous_command_clear_error_does_not_blame_a_valid_service() -> TestResult {
    let profile = reviewed_profile()?;
    let mut valid = service("valid", Some("busybox:1"))?;
    valid.set_command(Sourced::generated(Command::Empty));
    valid.set_entrypoint(Sourced::generated(Entrypoint::Exec(vec![ProtectedString::plain(
        "/bin/entry",
    )])));
    let mut invalid = service("invalid", Some("busybox:1"))?;
    invalid.set_command(Sourced::generated(Command::Empty));
    let mut app = application(valid)?;
    app.add_service(Sourced::generated(invalid))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "target.docker.command_entrypoint" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(
        !planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.valid.command" && outcome.kind() == ConversionKind::Invalid
        })
    );
    assert!(
        planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.valid.command" && outcome.kind() == ConversionKind::Exact
        })
    );
    Ok(())
}

#[test]
fn guarded_command_clear_retains_cmd_and_denies_entrypoint_clear_on_all_profiles() -> TestResult {
    let catalogue = TargetCapabilityCatalog::reviewed();
    assert_eq!(catalogue.profiles().count(), 4);
    for profile in catalogue.profiles() {
        let mut web = service("web", Some("busybox:1"))?;
        web.set_command(Sourced::generated(Command::Empty));
        web.set_entrypoint(Sourced::generated(Entrypoint::Exec(vec![ProtectedString::plain(
            "/bin/entry",
        )])));
        let planned = DockerExporter::new(profile.clone())?.plan(&application(web)?, &target(profile)?)?;
        let output = planned.candidate().ok_or("guarded CommandClear candidate missing")?;
        let requests = std::str::from_utf8(output.bytes())?;
        assert!(requests.contains("\"Entrypoint\":[\"/bin/entry\"]"));
        assert!(requests.contains("\"Cmd\":[]"));
        assert!(planned.authorize(LossPolicy::ExactOnly).output().is_some());

        let mut web = service("web", Some("busybox:1"))?;
        web.set_entrypoint(Sourced::generated(Entrypoint::Empty));
        web.set_command(Sourced::generated(Command::Exec(vec![ProtectedString::plain(
            "--serve",
        )])));
        let planned = DockerExporter::new(profile.clone())?.plan(&application(web)?, &target(profile)?)?;
        assert!(planned.candidate().is_none(), "EntrypointClear must remain denied");
        assert!(planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.entrypoint" && outcome.kind() == ConversionKind::Invalid
        }));
        assert!(format!("{:?}", planned.diagnostics()).contains("missing_capability"));
    }
    Ok(())
}

#[test]
fn exact_native_identity_evidence_mode_and_numeric_range_are_independent_gates() -> TestResult {
    let profile = reviewed_profile()?;
    let app = application(service("web", Some("busybox:1"))?)?;
    let wrong_range = TargetProfile::new(
        DOCKER_TARGET,
        PlatformVersion::new(1, 0, 0),
        Some(PlatformVersion::new(1, 0, 0)),
    )?;
    let wrong_implementation = TargetProfile::new(
        "podman",
        numeric_engine_release(&profile).ok_or("numeric release missing")?,
        None,
    )?;
    let forged_key = CapabilityEvidenceKey::sha256([7; 32]).map_err(|error| format!("{error:?}"))?;
    let forged = NativeTargetProfile::new(profile.identity().clone(), forged_key);
    let wrong_mode = TargetProfileIdentity::new(
        profile.identity().build().clone(),
        profile.release().clone(),
        profile.identity().advertised_api_version(),
        profile.identity().acquisition_api_version(),
        profile.identity().rendering_api_version(),
        DaemonMode::Rootless,
    )
    .map_err(|error| format!("{error:?}"))?;
    let forged_mode = NativeTargetProfile::new(wrong_mode, profile.evidence_key().clone());
    let identity = profile.identity();
    let advertised = identity.advertised_api_version();
    let acquisition = identity.acquisition_api_version();
    let rendering = identity.rendering_api_version();
    let forged_advertised = forged_api_profile(
        &profile,
        ApiVersion::new(advertised.major, advertised.minor.saturating_add(1)),
        acquisition,
        rendering,
    )?;
    let forged_acquisition = forged_api_profile(
        &profile,
        advertised,
        ApiVersion::new(acquisition.major, acquisition.minor.saturating_add(1)),
        rendering,
    )?;
    let forged_rendering = forged_api_profile(
        &profile,
        advertised,
        acquisition,
        ApiVersion::new(rendering.major, rendering.minor.saturating_sub(1)),
    )?;
    for (exporter, box_target) in [
        (DockerExporter::new(profile.clone())?, wrong_range),
        (DockerExporter::new(profile.clone())?, wrong_implementation),
        (DockerExporter::new(forged)?, target(&profile)?),
        (DockerExporter::new(forged_mode)?, target(&profile)?),
        (DockerExporter::new(forged_advertised)?, target(&profile)?),
        (DockerExporter::new(forged_acquisition)?, target(&profile)?),
        (DockerExporter::new(forged_rendering)?, target(&profile)?),
    ] {
        let planned = exporter.plan(&app, &box_target)?;
        assert!(planned.candidate().is_none());
        assert!(
            planned
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == "target.docker")
        );
        assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());
    }
    Ok(())
}

#[test]
fn debian_package_revision_and_all_api_dimensions_survive_in_output() -> TestResult {
    let profile = TargetCapabilityCatalog::reviewed()
        .profiles()
        .find(|profile| matches!(profile.identity().build(), EngineBuild::DebianPackage(_)))
        .ok_or("reviewed Debian package target missing")?
        .clone();
    let planned = DockerExporter::new(profile.clone())?
        .plan(&application(service("web", Some("busybox:1"))?)?, &target(&profile)?)?;
    let output = planned.candidate().ok_or("reviewed package target missing")?;
    let identity = output.native_profile().identity();
    assert_eq!(identity.build(), profile.identity().build());
    assert_eq!(
        identity.advertised_api_version(),
        profile.identity().advertised_api_version()
    );
    assert_eq!(
        identity.acquisition_api_version(),
        profile.identity().acquisition_api_version()
    );
    assert_eq!(
        identity.rendering_api_version(),
        profile.identity().rendering_api_version()
    );
    assert_eq!(identity.mode(), profile.identity().mode());
    assert_eq!(output.evidence().record_key(), profile.evidence_key());

    let changed_package = TargetProfileIdentity::new(
        EngineBuild::DebianPackage(
            DebianPackageRevision::new("20.10.5+unreviewed".into()).map_err(|error| format!("{error:?}"))?,
        ),
        profile.release().clone(),
        identity.advertised_api_version(),
        identity.acquisition_api_version(),
        identity.rendering_api_version(),
        identity.mode(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let forged = NativeTargetProfile::new(changed_package, profile.evidence_key().clone());
    assert!(
        DockerExporter::new(forged)?
            .plan(&application(service("web", Some("busybox:1"))?)?, &target(&profile)?)?
            .candidate()
            .is_none()
    );
    Ok(())
}

#[test]
fn unsupported_neutral_field_is_a_subject_loss_and_suppresses_unsafe_service() -> TestResult {
    let profile = reviewed_profile()?;
    let mut service = service("web", Some("busybox:1"))?;
    service.set_hostname(Sourced::generated(ProtectedString::sensitive("private-hostname")));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned
            .outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "services.web.hostname")
    );
    assert!(!format!("{:?}", planned.diagnostics()).contains("private-hostname"));
    Ok(())
}

#[test]
fn duplicate_fixed_publication_reports_the_offending_service_port() -> TestResult {
    let profile = reviewed_profile()?;
    let mut first = service("one", Some("busybox:1"))?;
    first.add_port(Sourced::generated(NeutralPort::new(
        80,
        Some(8080),
        None,
        Protocol::Tcp,
    )?));
    let mut second = service("two", Some("busybox:1"))?;
    second.add_port(Sourced::generated(NeutralPort::new(
        81,
        Some(8080),
        None,
        Protocol::Tcp,
    )?));
    let mut app = application(first)?;
    app.add_service(Sourced::generated(second))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.two.ports.0" && outcome.kind() == ConversionKind::Invalid
        })
    );
    assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());
    Ok(())
}

#[test]
fn duplicate_environment_name_reports_the_offending_service_field() -> TestResult {
    let profile = reviewed_profile()?;
    let mut service = service("web", Some("busybox:1"))?;
    for value in ["first", "second"] {
        service.add_environment(Sourced::generated(EnvironmentVariable::new(
            Identifier::new("TOKEN")?,
            EnvironmentValue::Literal(ProtectedString::sensitive(value)),
        )));
    }
    let planned = DockerExporter::new(profile.clone())?
        .with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral)
        .plan(&application(service)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.environment.TOKEN" && outcome.kind() == ConversionKind::Invalid
    }));
    assert!(!format!("{:?}", planned.diagnostics()).contains("second"));
    Ok(())
}

#[test]
fn rootless_privileged_host_port_reports_indexed_invalid_loss() -> TestResult {
    let profile = TargetCapabilityCatalog::reviewed()
        .profiles()
        .find(|profile| {
            matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootless
        })
        .ok_or("reviewed upstream rootless target is missing")?
        .clone();
    let mut service = service("web", Some("busybox:1"))?;
    service.add_port(Sourced::generated(NeutralPort::new(80, Some(80), None, Protocol::Tcp)?));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.ports.0" && outcome.kind() == ConversionKind::Invalid
        })
    );
    assert!(!planned.outcomes().iter().any(|outcome| {
        (outcome.subject() == "services.web.ports.0" || outcome.subject() == "services.web")
            && outcome.kind() == ConversionKind::Exact
    }));
    assert!(format!("{:?}", planned.diagnostics()).contains("restricted_port"));
    assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());
    Ok(())
}

#[test]
fn rootless_mixed_ephemeral_fixed_privileged_and_high_ports_attribute_only_fixed_privileged_loss() -> TestResult {
    let profile = TargetCapabilityCatalog::reviewed()
        .profiles()
        .find(|profile| {
            matches!(profile.identity().build(), EngineBuild::Upstream) && profile.mode() == DaemonMode::Rootless
        })
        .ok_or("reviewed upstream rootless target is missing")?
        .clone();
    let mut web = service("web", Some("busybox:1"))?;
    web.add_port(Sourced::generated(NeutralPort::new(
        8080,
        Some(0),
        Some("127.0.0.1".into()),
        Protocol::Tcp,
    )?));
    web.add_port(Sourced::generated(NeutralPort::new(80, Some(80), None, Protocol::Tcp)?));
    web.add_port(Sourced::generated(NeutralPort::new(
        8081,
        Some(18080),
        None,
        Protocol::Tcp,
    )?));
    let app = application(web)?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert_eq!(app.services()[0].value().ports()[0].value().published(), Some(0));
    assert!(
        planned
            .diagnostics()
            .iter()
            .flat_map(Diagnostic::fields)
            .all(|field| field.name() != "native_feature")
    );
    assert_eq!(
        app.services()[0].value().ports()[0].value().host_address(),
        Some("127.0.0.1")
    );
    let invalid_ports = planned
        .outcomes()
        .iter()
        .filter(|outcome| {
            outcome.subject().starts_with("services.web.ports.") && outcome.kind() == ConversionKind::Invalid
        })
        .map(ConversionOutcome::subject)
        .collect::<Vec<_>>();
    assert_eq!(invalid_ports, vec!["services.web.ports.1"]);
    for subject in ["services.web.ports.0", "services.web.ports.2"] {
        assert!(
            planned
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == subject && outcome.kind() == ConversionKind::Exact)
        );
        assert!(!planned.diagnostics().iter().any(|diagnostic| {
            diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "subject" && field.value().expose() == subject)
        }));
    }
    assert!(planned.diagnostics().iter().any(|diagnostic| {
        diagnostic
            .fields()
            .iter()
            .any(|field| field.name() == "native_category" && field.value().expose() == "restricted_port")
            && diagnostic
                .fields()
                .iter()
                .any(|field| field.name() == "subject" && field.value().expose() == "services.web.ports.1")
    }));
    assert!(planned.authorize(LossPolicy::AllowPartial).is_blocked());

    let mut web = service("web", Some("busybox:1"))?;
    web.add_port(Sourced::generated(NeutralPort::new(
        8080,
        Some(0),
        Some("127.0.0.1".into()),
        Protocol::Tcp,
    )?));
    web.add_port(Sourced::generated(NeutralPort::new(
        8081,
        Some(18080),
        None,
        Protocol::Tcp,
    )?));
    let app = application(web)?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(
        planned.candidate().is_some(),
        "synthetic rootless publication diagnostics: {:?}",
        planned.diagnostics()
    );
    assert!(
        planned
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    assert_eq!(app.services()[0].value().ports()[0].value().published(), Some(0));
    assert_eq!(app.services()[0].value().ports()[1].value().published(), Some(18080));
    assert!(!format!("{:?}", planned.diagnostics()).contains("restricted_port"));
    Ok(())
}

#[test]
fn duplicate_named_volume_destination_reports_indexed_invalid_loss() -> TestResult {
    let profile = reviewed_profile()?;
    let mut service = service("web", Some("busybox:1"))?;
    for name in ["first", "second"] {
        service.add_mount(Sourced::generated(NeutralMount::new(
            MountSource::Volume(Identifier::new(name)?),
            "/data",
            false,
        )?));
    }
    let mut app = application(service)?;
    for name in ["first", "second"] {
        app.add_volume(Sourced::generated(Volume::new(
            Identifier::new(name)?,
            ResourceOwnership::Application,
        )))?;
    }
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(
        planned.outcomes().iter().any(|outcome| {
            outcome.subject() == "services.web.mounts.1" && outcome.kind() == ConversionKind::Invalid
        })
    );
    Ok(())
}

#[test]
fn empty_first_exec_arguments_report_their_source_fields() -> TestResult {
    let profile = reviewed_profile()?;
    for field in ["command", "entrypoint"] {
        let mut service = service("web", Some("busybox:1"))?;
        match field {
            "command" => service.set_command(Sourced::generated(Command::Exec(vec![ProtectedString::plain("")]))),
            _ => service.set_entrypoint(Sourced::generated(Entrypoint::Exec(vec![ProtectedString::plain("")]))),
        }
        let planned = DockerExporter::new(profile.clone())?.plan(&application(service)?, &target(&profile)?)?;
        assert!(planned.candidate().is_none());
        assert!(planned.outcomes().iter().any(|outcome| {
            outcome.subject() == format!("services.web.{field}") && outcome.kind() == ConversionKind::Invalid
        }));
    }
    Ok(())
}

#[test]
fn external_network_without_driver_is_adjusted_but_keeps_exact_prerequisite() -> TestResult {
    let profile = reviewed_profile()?;
    let mut web = service("web", Some("busybox:1"))?;
    web.add_network(Sourced::generated(NetworkAttachment::new(
        Identifier::new("edge")?,
        Vec::new(),
    )));
    let mut app = application(web)?;
    let mut edge = Network::new(Identifier::new("edge")?, ResourceOwnership::External);
    edge.set_runtime_name(Sourced::generated(ProtectedString::plain("shared-edge")));
    let origin = Provenance::source(SourceId::new("private/compose.yaml")?);
    app.add_network(Sourced::from_source(edge, origin.clone()))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    let output = planned
        .candidate()
        .ok_or("reviewed external network candidate missing")?;
    assert_eq!(output.network_prerequisites().len(), 1);
    let adjusted = planned
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "networks.edge.driver")
        .ok_or("missing external driver adjustment")?;
    assert_eq!(adjusted.kind(), ConversionKind::Approximate);
    assert_eq!(adjusted.origins(), &[origin]);
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "networks.edge.prerequisite" && outcome.kind() == ConversionKind::Exact
    }));
    assert!(planned.authorize(LossPolicy::ExactOnly).is_blocked());
    let approved = DockerExporter::new(profile.clone())?
        .plan(&app, &target(&profile)?)?
        .authorize(LossPolicy::AllowApproximate);
    assert!(approved.output().is_some());
    Ok(())
}

#[test]
fn incompatible_external_network_driver_and_missing_reference_reject() -> TestResult {
    let profile = reviewed_profile()?;
    let mut web = service("web", Some("busybox:1"))?;
    web.add_network(Sourced::generated(NetworkAttachment::new(
        Identifier::new("edge")?,
        Vec::new(),
    )));
    let mut app = application(web)?;
    let mut edge = Network::new(Identifier::new("edge")?, ResourceOwnership::External);
    edge.set_driver(Sourced::generated(ProtectedString::plain("overlay")));
    app.add_network(Sourced::generated(edge))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "networks.edge.driver" && outcome.kind() == ConversionKind::Unsupported
    }));
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.networks.0" && outcome.kind() == ConversionKind::Invalid
    }));
    Ok(())
}

#[test]
fn external_network_never_clears_explicit_effective_constraints() -> TestResult {
    let profile = reviewed_profile()?;
    for field in ["internal-false", "ipv6-true"] {
        let mut web = service("web", Some("busybox:1"))?;
        web.add_network(Sourced::generated(NetworkAttachment::new(
            Identifier::new("docker-network-1")?,
            Vec::new(),
        )));
        let mut app = application(web)?;
        let mut network = Network::new(Identifier::new("docker-network-1")?, ResourceOwnership::External);
        network.set_runtime_name(Sourced::generated(ProtectedString::plain("destination-network")));
        match field {
            "internal-false" => network.set_internal(Sourced::generated(false)),
            "ipv6-true" => network.set_ipv6(Sourced::generated(true)),
            _ => unreachable!("closed constraint fixture"),
        }
        app.add_network(Sourced::generated(network))?;
        let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
        if field == "internal-false" {
            let output = planned
                .candidate()
                .ok_or("admitted explicit external false was omitted")?;
            assert_eq!(output.network_prerequisites()[0].expected_internal, Some(false));
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "networks.docker-network-1.internal"
                        && outcome.kind() == ConversionKind::Exact)
            );
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "networks.docker-network-1.driver"
                        && outcome.kind() == ConversionKind::Approximate)
            );
            assert!(planned.authorize(LossPolicy::ExactOnly).is_blocked());
            assert!(
                DockerExporter::new(profile.clone())?
                    .plan(&app, &target(&profile)?)?
                    .authorize(LossPolicy::AllowApproximate)
                    .output()
                    .is_some()
            );
        } else {
            assert!(
                planned.candidate().is_none(),
                "unsupported external IPv6 must still block"
            );
            assert!(
                planned
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.subject() == "networks.docker-network-1.ipv6"
                        && outcome.kind() == ConversionKind::Unsupported)
            );
            for policy in [
                LossPolicy::ExactOnly,
                LossPolicy::AllowApproximate,
                LossPolicy::AllowPartial,
            ] {
                let result = DockerExporter::new(profile.clone())?
                    .plan(&app, &target(&profile)?)?
                    .authorize(policy);
                assert!(result.is_blocked() && result.output().is_none());
            }
        }
    }
    Ok(())
}

#[test]
fn admitted_external_network_internal_expectations_remain_distinct_on_every_profile_and_policy() -> TestResult {
    let profiles = TargetCapabilityCatalog::reviewed();
    assert_eq!(profiles.profiles().count(), 4);
    for profile in profiles.profiles() {
        for expected_internal in [None, Some(false), Some(true)] {
            let outer = Provenance::source(SourceId::new("private/network-origin-canary.yaml")?);
            let field = Provenance::source(SourceId::new("private/internal-origin-canary.yaml")?);
            let mut network = Network::new(Identifier::new("edge")?, ResourceOwnership::External);
            network.set_driver(Sourced::generated(ProtectedString::plain("bridge")));
            if let Some(internal) = expected_internal {
                network.set_internal(Sourced::from_source(internal, field.clone()));
            }
            let mut app = Application::new(Identifier::new("external-constraint")?);
            app.add_network(Sourced::from_source(network, outer.clone()))?;
            for policy in [
                LossPolicy::ExactOnly,
                LossPolicy::AllowApproximate,
                LossPolicy::AllowPartial,
            ] {
                let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(profile)?)?;
                let output = planned.candidate().ok_or("admitted external prerequisite missing")?;
                assert_external_only_artifact(output, profile, expected_internal, "edge")?;
                if expected_internal.is_some() {
                    let outcome = planned
                        .outcomes()
                        .iter()
                        .find(|outcome| outcome.subject() == "networks.edge.internal")
                        .ok_or("missing retained external isolation decision")?;
                    assert_eq!(outcome.kind(), ConversionKind::Exact);
                    assert!(outcome.diagnostic().is_none());
                    assert!(outcome.origins().contains(&outer) && outcome.origins().contains(&field));
                } else {
                    assert!(
                        !planned
                            .outcomes()
                            .iter()
                            .any(|outcome| outcome.subject() == "networks.edge.internal")
                    );
                }
                assert!(
                    planned
                        .outcomes()
                        .iter()
                        .all(|outcome| outcome.kind() == ConversionKind::Exact)
                );
                assert!(
                    planned
                        .outcomes()
                        .iter()
                        .any(|outcome| outcome.subject() == "networks.edge.prerequisite")
                );
                assert!(planned.diagnostics().is_empty());
                assert!(!format!("{output:?} {:?}", planned.diagnostics()).contains("canary"));
                let result = planned.authorize(policy);
                assert!(!result.is_blocked() && result.output().is_some());
            }
        }
    }
    Ok(())
}

fn expected_native_context(profile: &NativeTargetProfile) -> TestResult<String> {
    use std::fmt::Write;
    let identity = profile.identity();
    let build = match identity.build() {
        EngineBuild::Upstream => "{\"kind\":\"upstream\"}".to_owned(),
        EngineBuild::DebianPackage(revision) => {
            format!("{{\"kind\":\"debian_package\",\"revision\":\"{}\"}}", revision.as_str())
        }
    };
    let mode = match profile.mode() {
        DaemonMode::Rootful => "rootful",
        DaemonMode::Rootless => "rootless",
        DaemonMode::Unknown => return Err("unreviewed daemon mode in admitted context".into()),
    };
    let api = |version: ApiVersion| format!("{}.{}", version.major.get(), version.minor);
    let mut evidence = String::new();
    for byte in profile.evidence_key().as_sha256_bytes() {
        write!(evidence, "{byte:02x}")?;
    }
    Ok(format!(
        "{{\"kind\":\"target\",\"build\":{build},\"engine_release\":\"{}\",\"advertised_api_version\":\"{}\",\"acquisition_api_version\":\"{}\",\"rendering_api_version\":\"{}\",\"daemon_mode\":\"{mode}\",\"evidence_sha256\":\"{evidence}\"}}",
        profile.release().as_str(),
        api(identity.advertised_api_version()),
        api(identity.acquisition_api_version()),
        api(identity.rendering_api_version())
    ))
}

fn assert_external_only_artifact(
    output: &DockerOutput,
    profile: &NativeTargetProfile,
    internal: Option<bool>,
    identity: &str,
) -> TestResult {
    assert_eq!(output.native_profile(), profile);
    assert_eq!(output.evidence().record_key(), profile.evidence_key());
    assert!(
        output.bytes().is_empty(),
        "external intent must not create or modify the network"
    );
    assert!(output.volume_prerequisites().is_empty());
    let [network] = output.network_prerequisites() else {
        return Err("external prerequisite inventory changed".into());
    };
    assert_eq!(network.identity(), identity.as_bytes());
    assert_eq!(network.expected_driver, NetworkDriver::Bridge);
    assert_eq!(network.expected_internal, internal);
    let schema = if internal.is_some() { 3 } else { 1 };
    let internal_field = internal.map_or_else(String::new, |value| format!(",\"expected_internal\":{value}"));
    // Opaque references are correlated to typed prerequisites, never request positions or host allocation.
    let reference = network.reference.local_index();
    let context = expected_native_context(profile)?;
    let expected = format!(
        "{{\"schema_version\":{schema},\"context\":{context},\"requests\":[],\"prerequisites\":[{{\"kind\":\"network\",\"reference\":\"{reference}\",\"identity\":\"{identity}\",\"expected_driver\":\"bridge\"{internal_field}}}]}}"
    );
    let complete = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
    assert!(
        complete == expected.as_bytes() || complete == format!("{expected}\n").as_bytes(),
        "complete external artifact context, closed fields, schema or boolean differs"
    );
    Ok(())
}

#[test]
fn admitted_external_constraints_and_relabel_binds_keep_schema_precedence_and_all_obligations() -> TestResult {
    for profile in TargetCapabilityCatalog::reviewed().profiles() {
        for internal in [None, Some(false), Some(true)] {
            let mut web = service("web", Some("busybox:1"))?;
            web.add_network(Sourced::generated(NetworkAttachment::new(
                Identifier::new("edge")?,
                Vec::new(),
            )));
            let mut mount =
                NeutralMount::new(MountSource::HostPath("/fake/private-bind-canary".into()), "/data", true)?;
            mount.set_selinux_relabel(SelinuxRelabel::Private);
            web.add_mount(Sourced::generated(mount));
            let mut app = application(web)?;
            let mut network = Network::new(Identifier::new("edge")?, ResourceOwnership::External);
            network.set_driver(Sourced::generated(ProtectedString::plain("bridge")));
            if let Some(value) = internal {
                network.set_internal(Sourced::generated(value));
            }
            app.add_network(Sourced::generated(network))?;
            for policy in [
                LossPolicy::ExactOnly,
                LossPolicy::AllowApproximate,
                LossPolicy::AllowPartial,
            ] {
                let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(profile)?)?;
                let output = planned
                    .candidate()
                    .ok_or("admitted combined network/bind artifact missing")?;
                assert_eq!(output.native_profile(), profile);
                assert_eq!(output.network_prerequisites().len(), 1);
                assert_eq!(output.network_prerequisites()[0].expected_internal, internal);
                let [bind] = output.artifact.bind_source_prerequisites() else {
                    return Err("combined bind prerequisite inventory differs".into());
                };
                assert_eq!(bind.identity(), b"web");
                assert_eq!(bind.mount_index, 0);
                assert_eq!(bind.source(), b"/fake/private-bind-canary");
                assert_eq!(bind.target(), b"/data");
                assert!(bind.read_only());
                assert_eq!(bind.relabel(), BindRelabel::Private);
                let complete = output.complete_bytes().map_err(|error| format!("{error:?}"))?;
                let text = std::str::from_utf8(&complete)?;
                let schema = if internal.is_some() { 3 } else { 2 };
                assert!(text.starts_with(&format!(
                    "{{\"schema_version\":{schema},\"context\":{},\"requests\":[",
                    expected_native_context(profile)?
                )));
                assert!(text.contains("\"Binds\":[\"/fake/private-bind-canary:/data:ro,Z\"]"));
                assert!(text.contains("\"source_conditions\":[\"exists\",\"type_reviewed\",\"contents_reviewed\",\"ownership_reviewed\",\"permissions_reviewed\"]"));
                assert!(text.contains("\"selinux_conditions\":[\"daemon_selinux_enabled\",\"container_mount_label_present\",\"policy_filesystem_support\",\"relabel_authority\"]"));
                assert!(text.contains("\"selinux_effect\":\"unverified\""));
                assert_eq!(text.matches("\"kind\":\"bind_source\"").count(), 1);
                if let Some(value) = internal {
                    assert!(text.contains(&format!("\"expected_internal\":{value}")));
                } else {
                    assert!(!text.contains("\"expected_internal\""));
                }
                assert!(
                    planned
                        .outcomes()
                        .iter()
                        .all(|outcome| outcome.kind() == ConversionKind::Exact)
                );
                assert!(planned.diagnostics().is_empty());
                assert!(!format!("{output:?}").contains("canary"));
                let authorized = planned.authorize(policy);
                assert!(!authorized.is_blocked() && authorized.output().is_some());
            }
        }
    }
    Ok(())
}

#[test]
fn external_constraint_mapping_keeps_bool_and_avoids_exact_claims_for_omitted_siblings() -> TestResult {
    let profile = reviewed_profile()?;
    let exporter = DockerExporter::new(profile.clone())?;
    for internal in [false, true] {
        for unsupported in ["none", "ipv6", "labels"] {
            let mut network = Network::new(Identifier::new("edge")?, ResourceOwnership::External);
            network.set_driver(Sourced::generated(ProtectedString::plain("bridge")));
            network.set_internal(Sourced::generated(internal));
            match unsupported {
                "ipv6" => network.set_ipv6(Sourced::generated(true)),
                "labels" => network.set_labels(vec![Sourced::generated(MetadataLabel::new(
                    Identifier::new("public")?,
                    ProtectedString::plain("reviewed"),
                ))]),
                _ => (),
            }
            let mut app = Application::new(Identifier::new("external-constraint")?);
            app.add_network(Sourced::generated(network))?;
            let mut mapping = Mapping {
                exporter: &exporter,
                application: &app,
                resources: Vec::new(),
                networks: BTreeMap::new(),
                volumes: BTreeMap::new(),
                published_ports: BTreeMap::new(),
                service_refs: Vec::new(),
                dependency_decisions: Vec::new(),
                unsafe_service: false,
                outcomes: Vec::new(),
                diagnostics: Vec::new(),
            };
            mapping.map_application()?;
            if unsupported == "none" {
                let [TargetResource::Network(network)] = mapping.resources.as_slice() else {
                    return Err("external typed network mapping missing".into());
                };
                assert!(
                    matches!(&network.source, NetworkSource::External { expected_internal: Some(value), .. } if *value == internal)
                );
            } else {
                assert!(mapping.resources.is_empty());
                assert!(
                    !mapping
                        .outcomes
                        .iter()
                        .any(|outcome| outcome.kind() == ConversionKind::Exact
                            && matches!(
                                outcome.subject(),
                                "networks.edge.internal" | "networks.edge.prerequisite" | "networks.edge"
                            ))
                );
                assert!(
                    mapping
                        .outcomes
                        .iter()
                        .any(|outcome| outcome.subject() == "networks.edge.internal"
                            && outcome.kind() == ConversionKind::Unsupported)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn repeated_host_ip_bindings_and_exposed_ports_keep_independent_shapes() -> TestResult {
    let profile = reviewed_profile()?;
    let mut web = service("web", Some("busybox:1"))?;
    for address in ["127.0.0.1", "127.0.0.2"] {
        web.add_port(Sourced::generated(NeutralPort::new(
            80,
            Some(8080),
            Some(address.into()),
            Protocol::Tcp,
        )?));
    }
    web.add_port(Sourced::generated(NeutralPort::new(81, Some(0), None, Protocol::Tcp)?));
    web.add_exposed_port(Sourced::generated(ExposedPort::new(9000, Protocol::Udp)?));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(web)?, &target(&profile)?)?;
    let text = std::str::from_utf8(planned.candidate().ok_or("port candidate missing")?.bytes())?;
    assert!(text.contains("\"80/tcp\""));
    assert!(text.contains("\"HostIp\":\"127.0.0.1\""));
    assert!(text.contains("\"HostIp\":\"127.0.0.2\""));
    assert!(text.contains("\"9000/udp\""));
    assert!(text.contains("\"81/tcp\""));
    assert!(
        planned
            .outcomes()
            .iter()
            .all(|outcome| outcome.kind() == ConversionKind::Exact)
    );
    Ok(())
}

#[test]
fn supplied_runtime_name_and_dependency_decision_remain_separate_from_engine_requests() -> TestResult {
    let profile = reviewed_profile()?;
    let mut database = service("database", Some("busybox:1"))?;
    database.set_runtime_name(Sourced::generated(ProtectedString::plain("db-live")));
    let mut web = service("web", Some("busybox:1"))?;
    web.set_runtime_name(Sourced::generated(ProtectedString::plain("web-live")));
    let mut dependency = ServiceDependency::new(Identifier::new("database")?);
    let origin = Provenance::source(SourceId::new("private/source.yaml")?);
    dependency.set_condition(Sourced::from_source(
        ServiceDependencyCondition::Healthy,
        origin.clone(),
    ));
    dependency.set_required(Sourced::generated(true));
    dependency.set_restart(Sourced::generated(false));
    web.add_dependency(Sourced::from_source(dependency, origin.clone()));
    let mut app = application(database)?;
    app.add_service(Sourced::generated(web))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    let output = planned.candidate().ok_or("dependency candidate missing")?;
    assert_eq!(output.dependency_decisions().len(), 1);
    let decision = &output.dependency_decisions()[0];
    assert_eq!(decision.service_identity(), b"web-live");
    assert_eq!(decision.dependency_identity(), b"db-live");
    assert_eq!(decision.condition(), Some(&ServiceDependencyCondition::Healthy));
    assert_eq!(decision.required(), Some(true));
    assert_eq!(decision.restart(), Some(false));
    assert_eq!(decision.reference_origins(), std::slice::from_ref(&origin));
    assert_eq!(decision.condition_origins(), &[origin]);
    assert!(!std::str::from_utf8(output.bytes())?.contains("depends_on"));
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.dependencies.0" && outcome.kind() == ConversionKind::Approximate
    }));
    assert!(planned.authorize(LossPolicy::ExactOnly).is_blocked());
    Ok(())
}

#[test]
fn dependency_defaults_and_completed_successfully_choices_remain_typed() -> TestResult {
    let profile = reviewed_profile()?;
    let database = service("database", Some("busybox:1"))?;
    let mut web = service("web", Some("busybox:1"))?;
    let source = Provenance::source(SourceId::new("private/source.yaml")?);
    web.add_dependency(Sourced::from_source(
        ServiceDependency::new(Identifier::new("database")?),
        source.clone(),
    ));
    let mut job = service("job", Some("busybox:1"))?;
    let mut completed = ServiceDependency::new(Identifier::new("database")?);
    completed.set_condition(Sourced::from_source(
        ServiceDependencyCondition::CompletedSuccessfully,
        source.clone(),
    ));
    completed.set_required(Sourced::from_source(false, source.clone()));
    completed.set_restart(Sourced::from_source(true, source.clone()));
    job.add_dependency(Sourced::from_source(completed, source.clone()));
    let mut app = application(database)?;
    app.add_service(Sourced::generated(web))?;
    app.add_service(Sourced::generated(job))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    let decisions = planned
        .candidate()
        .ok_or("dependency candidate missing")?
        .dependency_decisions();
    let started = decisions
        .iter()
        .find(|decision| decision.service() == "web")
        .ok_or("started decision missing")?;
    assert_eq!(started.condition(), None);
    assert_eq!(started.required(), None);
    assert_eq!(started.restart(), None);
    assert!(started.condition_origins().is_empty());
    assert_eq!(started.reference_origins(), std::slice::from_ref(&source));
    let completed = decisions
        .iter()
        .find(|decision| decision.service() == "job")
        .ok_or("completed decision missing")?;
    assert_eq!(
        completed.condition(),
        Some(&ServiceDependencyCondition::CompletedSuccessfully)
    );
    assert_eq!(completed.required(), Some(false));
    assert_eq!(completed.restart(), Some(true));
    assert_eq!(completed.condition_origins(), std::slice::from_ref(&source));
    assert_eq!(completed.required_origins(), std::slice::from_ref(&source));
    assert_eq!(completed.restart_origins(), &[source]);
    Ok(())
}

#[test]
fn dependency_missing_target_and_cycle_are_invalid() -> TestResult {
    let profile = reviewed_profile()?;
    let mut web = service("web", Some("busybox:1"))?;
    web.add_dependency(Sourced::generated(ServiceDependency::new(Identifier::new("missing")?)));
    let planned = DockerExporter::new(profile.clone())?.plan(&application(web)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "services.web.dependencies.0" && outcome.kind() == ConversionKind::Invalid
    }));
    let mut one = service("one", Some("busybox:1"))?;
    one.add_dependency(Sourced::generated(ServiceDependency::new(Identifier::new("two")?)));
    let mut two = service("two", Some("busybox:1"))?;
    two.add_dependency(Sourced::generated(ServiceDependency::new(Identifier::new("one")?)));
    let mut app = application(one)?;
    app.add_service(Sourced::generated(two))?;
    let planned = DockerExporter::new(profile.clone())?.plan(&app, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(|outcome| {
        outcome.subject() == "target.docker.dependencies" && outcome.kind() == ConversionKind::Invalid
    }));
    Ok(())
}

#[test]
fn inert_vendor_metadata_has_one_private_origin_bearing_loss_per_item() -> TestResult {
    use boxferry_model::{MetadataNode, MetadataScope, MetadataValue, VendorMetadata};
    let profile = reviewed_profile()?;
    let origin = Provenance::source(SourceId::new("metadata-source")?);
    let mut app = application(service("web", Some("busybox:1"))?)?;
    for (scope, namespace, protected) in [
        (MetadataScope::Application, "x-public", false),
        (
            MetadataScope::Service(Identifier::new("web")?),
            "x-private-namespace-canary",
            true,
        ),
    ] {
        let namespace = if protected {
            ProtectedString::sensitive(namespace)
        } else {
            ProtectedString::plain(namespace)
        };
        app.add_vendor_metadata(VendorMetadata::new(
            scope,
            Sourced::from_source(namespace, origin.clone()),
            Sourced::from_source(
                MetadataNode::new(
                    MetadataValue::String(ProtectedString::sensitive("metadata-value-canary")),
                    true,
                    false,
                )?,
                origin.clone(),
            ),
        )?)?;
    }
    let exporter = DockerExporter::new(profile.clone())?;
    let planned = exporter.plan(&app, &target(&profile)?)?;
    let losses = planned
        .outcomes()
        .iter()
        .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
        .collect::<Vec<_>>();
    assert_eq!(losses.len(), 2);
    for (index, loss) in losses.iter().enumerate() {
        assert_eq!(loss.subject(), format!("application.vendor_metadata[{index}]"));
        assert_eq!(loss.diagnostic().map(DiagnosticCode::as_str), Some("BFD0006"));
        assert!(loss.origins().contains(&origin));
    }
    let bytes = planned
        .candidate()
        .ok_or("inert metadata should leave a reviewable partial candidate")?
        .bytes();
    assert!(!String::from_utf8_lossy(bytes).contains("Labels"));
    let debug = format!("{:?} {:?}", planned.outcomes(), planned.diagnostics());
    for canary in ["x-private-namespace-canary", "metadata-value-canary"] {
        assert!(!debug.contains(canary));
        assert!(!String::from_utf8_lossy(bytes).contains(canary));
    }
    assert!(planned.authorize(LossPolicy::ExactOnly).is_blocked());
    assert!(
        exporter
            .plan(&app, &target(&profile)?)?
            .authorize(LossPolicy::AllowPartial)
            .output()
            .is_some()
    );
    Ok(())
}

#[test]
fn deferred_expressions_and_provider_plans_never_produce_incomplete_docker_requests() -> TestResult {
    use boxferry_model::{
        DeferredExpression, DeferredSegment, ServiceEnvironmentAssignment, ServiceEnvironmentFile,
        ServiceEnvironmentName, ServiceExecutionEnvironment,
    };
    let profile = reviewed_profile()?;
    let origin = Provenance::source(SourceId::new("deferred-source")?);
    for bind in [false, true] {
        let mut web = service("web", Some("busybox:1"))?;
        let name = ServiceEnvironmentName::new("PRIVATE_PROVIDER_NAME")?;
        let expression = DeferredExpression::new(vec![Sourced::from_source(
            DeferredSegment::Reference(name.clone()),
            origin.clone(),
        )])?;
        if bind {
            web.add_mount(Sourced::from_source(
                NeutralMount::new(MountSource::DeferredHostPath(expression), "/data", false)?,
                origin.clone(),
            ));
        } else {
            web.add_environment(Sourced::from_source(
                EnvironmentVariable::new(
                    Identifier::new("CONTAINER_VALUE")?,
                    EnvironmentValue::Deferred(expression),
                ),
                origin.clone(),
            ));
        }
        web.set_execution_environment(Sourced::from_source(
            ServiceExecutionEnvironment::new(
                vec![Sourced::generated(name.clone())],
                vec![Sourced::generated(ServiceEnvironmentAssignment::new(
                    name,
                    "/srv/provider-private-value-canary",
                )?)],
                vec![Sourced::generated(ServiceEnvironmentFile::new(
                    "/nonexistent/provider-private-file-canary.env",
                )?)],
            )?,
            origin.clone(),
        ));
        let app = application(web)?;
        let planned = DockerExporter::new(profile.clone())?
            .with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral)
            .plan(&app, &target(&profile)?)?;
        assert!(planned.candidate().is_none());
        let provider = planned
            .outcomes()
            .iter()
            .find(|outcome| outcome.subject() == "services.web.execution_environment")
            .ok_or("provider plan was silently omitted")?;
        assert_eq!(provider.kind(), ConversionKind::Unsupported);
        assert_eq!(provider.diagnostic().map(DiagnosticCode::as_str), Some("BFD0006"));
        assert!(provider.origins().contains(&origin));
        let expression_subject = if bind {
            "services.web.mounts.0"
        } else {
            "services.web.environment.CONTAINER_VALUE"
        };
        assert!(planned.outcomes().iter().any(|outcome| outcome.subject() == expression_subject
            && outcome.kind() == ConversionKind::Unsupported));
        let debug = format!("{app:?} {:?} {:?}", planned.outcomes(), planned.diagnostics());
        for canary in [
            "PRIVATE_PROVIDER_NAME",
            "provider-private-value-canary",
            "provider-private-file-canary",
        ] {
            assert!(!debug.contains(canary));
        }
        assert!(planned.authorize(LossPolicy::AllowPartial).output().is_none());
    }
    Ok(())
}

#[test]
fn unresolved_source_environment_is_not_reinterpreted_under_literal_output_consent() -> TestResult {
    let profile = reviewed_profile()?;
    let mut web = service("web", Some("busybox:1"))?;
    web.add_environment(Sourced::generated(EnvironmentVariable::new(
        Identifier::new("CONTAINER_VALUE")?,
        EnvironmentValue::UnresolvedSource {
            source_format: Identifier::new("compose")?,
            expression: ProtectedString::sensitive("${PRIVATE_SOURCE_CANARY}"),
        },
    )));
    let planned = DockerExporter::new(profile.clone())?
        .with_environment_policy(DockerEnvironmentPolicy::IncludeLiteral)
        .plan(&application(web)?, &target(&profile)?)?;
    assert!(planned.candidate().is_none());
    assert!(planned.outcomes().iter().any(
        |outcome| outcome.subject() == "services.web.environment.CONTAINER_VALUE"
            && outcome.kind() == ConversionKind::Unsupported
    ));
    assert!(!format!("{:?} {:?}", planned.outcomes(), planned.diagnostics()).contains("PRIVATE_SOURCE_CANARY"));
    assert!(planned.authorize(LossPolicy::AllowPartial).output().is_none());
    Ok(())
}
