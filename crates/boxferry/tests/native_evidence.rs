//! Public cross-route regression coverage for the opaque native-evidence boundary.

#![cfg(all(feature = "compose", feature = "podman", feature = "quadlet"))]

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    Application, COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ConversionKind,
    ExportAdapter, Identifier, ImportAdapter, LossPolicy, ModelError, PODMAN_TARGET, PlatformVersion, PodmanExporter,
    ProtectedString, Provenance, QuadletDocumentInput, QuadletExporter, QuadletImporter, QuadletSource,
    ResourceOwnership, RetainedNativeEvidence, RetainedNativeEvidenceEvent, RetainedNativeEvidenceSubject, SourceId,
    SourceSpan, Sourced, TargetProfile, Volume,
};

const SENTINEL: &str = "opaque-private-sentinel";

#[test]
fn evidence_construction_requires_provenance_and_export_loss_keeps_every_origin() -> Result<(), Box<dyn Error>> {
    let source = SourceId::new("ordered.volume")?;
    let event_origin = Provenance::spanned(source.clone(), SourceSpan::new(0, 8)?);
    let primary_origin = Provenance::spanned(source.clone(), SourceSpan::new(12, 18)?);
    let continuation_origin = Provenance::spanned(source, SourceSpan::new(22, 28)?);
    let subject = RetainedNativeEvidenceSubject::QuadletVolumePodmanArgs(Identifier::new("data")?);

    let sourced_segment = Sourced::from_source(ProtectedString::sensitive(SENTINEL), primary_origin.clone());
    assert!(matches!(
        RetainedNativeEvidence::new(
            subject.clone(),
            Sourced::generated(RetainedNativeEvidenceEvent::Value(vec![sourced_segment.clone()])),
        ),
        Err(ModelError::MissingNativeEvidenceProvenance { component: "event" })
    ));
    assert!(matches!(
        RetainedNativeEvidence::new(
            subject.clone(),
            Sourced::from_source(
                RetainedNativeEvidenceEvent::Value(vec![Sourced::generated(ProtectedString::sensitive(SENTINEL))]),
                event_origin.clone(),
            ),
        ),
        Err(ModelError::MissingNativeEvidenceProvenance {
            component: "physical segment"
        })
    ));
    assert!(matches!(
        RetainedNativeEvidence::new(
            subject.clone(),
            Sourced::from_source(RetainedNativeEvidenceEvent::Value(Vec::new()), event_origin.clone()),
        ),
        Err(ModelError::EmptyNativeEvidenceEvent)
    ));
    let unprotected_evidence = RetainedNativeEvidence::new(
        subject.clone(),
        Sourced::from_source(
            RetainedNativeEvidenceEvent::Value(vec![Sourced::from_source(
                ProtectedString::plain(SENTINEL),
                primary_origin.clone(),
            )]),
            event_origin.clone(),
        ),
    );
    assert_eq!(unprotected_evidence, Err(ModelError::UnprotectedNativeEvidenceSegment));
    assert!(!format!("{unprotected_evidence:?}").contains(SENTINEL));

    let evidence = RetainedNativeEvidence::new(
        subject,
        Sourced::from_source(
            RetainedNativeEvidenceEvent::Value(vec![
                sourced_segment,
                Sourced::from_source(ProtectedString::sensitive("continued"), continuation_origin.clone()),
            ]),
            event_origin.clone(),
        ),
    )?;
    let mut application = Application::new(Identifier::new("ordered")?);
    assert!(matches!(
        application.add_retained_native_evidence(evidence.clone()),
        Err(ModelError::UnknownNativeEvidenceOwner { kind: "volume", .. })
    ));
    application.add_volume(Sourced::generated(Volume::new(
        Identifier::new("data")?,
        ResourceOwnership::Application,
    )))?;
    application.add_retained_native_evidence(evidence)?;

    let plan = ComposeExporter::new()?.plan(&application, &compose_target()?)?;
    let outcome = plan
        .outcomes()
        .iter()
        .find(|outcome| outcome.subject() == "volumes.data.podman_args")
        .ok_or("retained-evidence outcome")?;
    assert_eq!(outcome.kind(), ConversionKind::Unsupported);
    assert_eq!(outcome.origins(), &[event_origin, primary_origin, continuation_origin]);
    assert!(!format!("{application:?}{plan:?}").contains(SENTINEL));
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one route contract deliberately verifies all three exporter artifacts"
)]
fn quadlet_source_keeps_portable_intent_and_reports_opaque_evidence_on_every_exporter() -> Result<(), Box<dyn Error>> {
    const VOLUME_SOURCE: &str = concat!(
        "[Volume]\n",
        "VolumeName=data-runtime\n",
        "Driver=local\n",
        "Copy=true\n",
        "User=1000\n",
        "ContainersConfModule=base.conf\n",
        "ContainersConfModule=   \n",
        "GlobalArgs= \\\n",
        "  continued-global\n",
        "PodmanArgs=first \\\n",
        "  second\n",
    );
    const CONTAINER_SOURCE: &str = concat!(
        "[Container]\n",
        "Image=example.invalid/web:1\n",
        "Notify=healthy\n",
        "PodmanArgs=--secret=opaque-private-sentinel\n",
    );

    let source = QuadletSource::parse(
        Identifier::new("native-evidence")?,
        [
            QuadletDocumentInput::new("data.volume", QuadletSourceId::new(41), VOLUME_SOURCE),
            QuadletDocumentInput::new("web.container", QuadletSourceId::new(42), CONTAINER_SOURCE),
        ],
    )?
    .into_source();
    let imported = QuadletImporter::new()?.import(&source);
    let application = imported.application().ok_or("application expected")?;
    let evidence = application.retained_native_evidence();
    assert_eq!(evidence.len(), 5);
    assert!(matches!(
        evidence[1].event().value(),
        RetainedNativeEvidenceEvent::Reset(_)
    ));
    assert!(matches!(
        evidence[2].event().value(),
        RetainedNativeEvidenceEvent::Value(_)
    ));
    assert_eq!(evidence[2].event().value().physical_segments().len(), 2);
    assert_eq!(evidence[3].event().value().physical_segments().len(), 2);

    let podman_segments = evidence[3].event().value().physical_segments();
    let expected_spans = podman_segments
        .iter()
        .map(|segment| {
            let text = segment.value().expose();
            let start = VOLUME_SOURCE.find(text).ok_or("physical segment in source")?;
            Ok(Some(SourceSpan::new(start, start + text.len())?))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    assert_eq!(
        podman_segments
            .iter()
            .map(|segment| segment.origins()[0].span())
            .collect::<Vec<_>>(),
        expected_spans
    );
    assert_eq!(
        evidence[3]
            .event()
            .origins()
            .iter()
            .map(Provenance::span)
            .collect::<Vec<_>>(),
        expected_spans
    );
    assert!(!format!("{application:?}{imported:?}").contains(SENTINEL));

    let expected = [
        "services.web.podman_args",
        "volumes.data.containers_conf_modules",
        "volumes.data.global_args",
        "volumes.data.podman_args",
    ];

    let compose = ComposeExporter::new()?.plan(application, &compose_target()?)?;
    assert_evidence_losses(&compose, &expected);
    let compose_result = compose.authorize(LossPolicy::AllowPartial);
    let compose_text = compose_result.output().ok_or("Compose output expected")?.text();
    assert!(compose_text.contains("example.invalid/web:1"));
    assert_no_opaque_native_values(compose_text);
    assert_private(&compose_result);

    let target = podman_target()?;
    let quadlet = QuadletExporter::new()?.plan(application, &target)?;
    assert_evidence_losses(&quadlet, &expected);
    let quadlet_result = quadlet.authorize(LossPolicy::AllowPartial);
    let quadlet_output = quadlet_result.output().ok_or("Quadlet output expected")?;
    let container = quadlet_output.file("web.container").ok_or("container expected")?.text();
    let volume = quadlet_output.file("data.volume").ok_or("volume expected")?.text();
    assert!(container.contains("Image=example.invalid/web:1"));
    assert!(container.contains("Notify=healthy"));
    assert!(volume.contains("VolumeName=data-runtime"));
    assert!(volume.contains("Driver=local"));
    assert!(volume.contains("Copy=true"));
    assert!(volume.contains("User=1000"));
    assert_no_opaque_native_values(container);
    assert_no_opaque_native_values(volume);
    assert_private(&quadlet_result);

    let podman = PodmanExporter::new()?.plan(application, &target)?;
    assert_evidence_losses(&podman, &expected);
    let podman_result = podman.authorize(LossPolicy::AllowPartial);
    let podman_output = podman_result.output().ok_or("Podman output expected")?;
    assert!(podman_output.deployment_json().contains("example.invalid/web:1"));
    assert_no_opaque_native_values(podman_output.deployment_json());
    assert_no_opaque_native_values(podman_output.commands_shell());
    assert_private(&podman_result);
    Ok(())
}

fn compose_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        COMPOSE_SPECIFICATION_PROFILE_REVISION,
        Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
    )?)
}

fn podman_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        PODMAN_TARGET,
        PlatformVersion::new(6, 0, 0),
        Some(PlatformVersion::new(6, 0, 0)),
    )?)
}

fn assert_evidence_losses<T: std::fmt::Debug>(plan: &boxferry::ConversionPlan<T>, expected: &[&str]) {
    for subject in expected {
        let matches = plan
            .outcomes()
            .iter()
            .filter(|outcome| {
                outcome.subject() == *subject
                    && outcome.kind() == ConversionKind::Unsupported
                    && !outcome.origins().is_empty()
            })
            .count();
        assert_eq!(
            matches, 1,
            "missing, unprovenanced, or duplicate retained-evidence loss for {subject}"
        );
        let diagnostic_matches = plan
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .fields()
                    .iter()
                    .any(|field| field.name() == "subject" && field.value().redacted() == *subject)
            })
            .count();
        assert_eq!(
            diagnostic_matches, 1,
            "missing or duplicate retained-evidence diagnostic for {subject}"
        );
    }
    assert!(!format!("{plan:?}").contains(SENTINEL));
}

fn assert_no_opaque_native_values(text: &str) {
    for forbidden in ["ContainersConfModule=", "GlobalArgs=", "PodmanArgs=", SENTINEL] {
        assert!(
            !text.contains(forbidden),
            "opaque native value escaped into output: {forbidden}"
        );
    }
}

fn assert_private<T: std::fmt::Debug>(result: &boxferry::ConversionResult<T>) {
    assert!(!format!("{result:?}").contains(SENTINEL));
    assert!(
        result
            .diagnostics()
            .iter()
            .all(|diagnostic| !format!("{diagnostic:?}").contains(SENTINEL))
    );
}
