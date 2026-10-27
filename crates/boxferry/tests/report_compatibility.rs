//! Regression coverage for constructible legacy reports and additive envelopes.
use boxferry::report::{
    BundleMember, ConversionReport, InputBundleMember, InvocationReport, MAX_COLLECTION_ITEMS, MAX_TEXT_BYTES,
    Truncation, VersionBounds,
};

fn legacy_report() -> ConversionReport {
    let report = ConversionReport::new(
        "test",
        "compose",
        "quadlet",
        VersionBounds {
            minimum: "not-applicable".into(),
            maximum: "not-applicable".into(),
        },
    );
    // Deliberately exhaustive: adding a public field breaks existing callers.
    ConversionReport {
        schema_version: report.schema_version,
        boxferry_version: report.boxferry_version,
        status: report.status,
        exit_category: report.exit_category,
        failed_stage: report.failed_stage,
        primary_diagnostic_code: report.primary_diagnostic_code,
        failure_summary: report.failure_summary,
        fix_first: report.fix_first,
        source_type: report.source_type,
        target_type: report.target_type,
        application: report.application,
        inputs: report.inputs,
        discovery: report.discovery,
        choices: report.choices,
        invocation: report.invocation,
        host: report.host,
        fidelity: report.fidelity,
        requested_versions: report.requested_versions,
        resolved_versions: report.resolved_versions,
        diagnostics: report.diagnostics,
        events: report.events,
        output_artifacts: report.output_artifacts,
        review_required: report.review_required,
        redaction: report.redaction,
        truncations: report.truncations,
    }
}

#[test]
fn original_public_literal_and_envelope_access_work_without_cli() {
    let original = legacy_report();
    let mut envelope = InvocationReport::from(original.clone());
    assert_eq!(envelope.as_conversion(), &original);
    assert_eq!(envelope.operation, "convert");
    envelope.as_conversion_mut().application = Some("changed".into());
    envelope.operation = "extract".into();
    envelope.bundle_members.push(BundleMember {
        transport: "<input-1>".into(),
        name: "web.container".into(),
        unit_type: "container".into(),
        size: 17,
    });
    envelope.input_bundle_members.push(InputBundleMember {
        transport_alias: "<bundle-input-1>".into(),
        member_alias: "<input-1>".into(),
    });
    let restored = envelope.into_conversion();
    assert_eq!(restored.application.as_deref(), Some("changed"));
}

#[test]
fn envelope_bounds_cover_operation_and_extension_truncations() {
    let mut report = InvocationReport::from(legacy_report());
    report.operation = "λ".repeat(MAX_TEXT_BYTES);
    report.truncations = (0..MAX_COLLECTION_ITEMS)
        .map(|index| Truncation {
            field: format!("field-{index}"),
            original: 2,
            retained: 1,
        })
        .collect();
    report.enforce_v1_limits();
    assert_eq!(report.operation.len(), MAX_TEXT_BYTES);
    assert_eq!(report.truncations.len(), MAX_COLLECTION_ITEMS);
    report.bundle_members.push(BundleMember {
        transport: "quadlet.bundle".into(),
        name: "web.container".into(),
        unit_type: "container".into(),
        size: 0,
    });
    assert!(report.reduce_for_json());
    assert!(report.bundle_members.is_empty());
    report.input_bundle_members.push(InputBundleMember {
        transport_alias: "<bundle-input-1>".into(),
        member_alias: "<input-1>".into(),
    });
    assert!(report.reduce_for_json());
    assert!(report.input_bundle_members.is_empty());
    assert!(!report.reduce_for_json());
}

#[cfg(feature = "cli")]
#[test]
fn serialized_legacy_shape_and_flat_envelope_remain_distinct() -> Result<(), Box<dyn std::error::Error>> {
    let original = legacy_report();
    let legacy = serde_json::to_value(&original)?;
    for field in ["operation", "bundle_members", "input_bundle_members"] {
        assert!(legacy.get(field).is_none(), "legacy serializer added {field}");
    }
    let mut envelope = InvocationReport::from(original);
    envelope.operation = "extract".into();
    envelope.bundle_members.push(BundleMember {
        transport: "<input-1>".into(),
        name: "web.container".into(),
        unit_type: "container".into(),
        size: 17,
    });
    let encoded = serde_json::to_value(&envelope)?;
    assert!(encoded.get("conversion").is_none());
    assert_eq!(encoded["schema_version"], 1);
    assert_eq!(encoded["operation"], "extract");
    assert_eq!(encoded["bundle_members"][0]["name"], "web.container");
    assert_eq!(encoded["bundle_members"][0]["size"], 17);
    assert_eq!(encoded["requested_versions"]["minimum"], "not-applicable");
    assert_eq!(encoded["fidelity"], legacy["fidelity"]);
    for (key, value) in legacy.as_object().ok_or("legacy object")? {
        assert_eq!(encoded.get(key), Some(value), "legacy field {key}");
    }
    Ok(())
}
