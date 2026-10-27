//! Inert metadata semantics, native bytes, document-set ownership and privacy refusals.
#![cfg(all(feature = "compose", feature = "quadlet", feature = "podman"))]
use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeId,
};
use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletId;
use boxferry::*;
use std::error::Error;
const SOURCE: &str = concat!(
    "---\nname: metadata\nx-yantr:\n  description: >\n    first paragraph\n\n    second paragraph\n",
    "  values: [null, true, false, 123456789012345678901234567890, 1.20e+30]\n",
    "  text: \"quote \\\" slash \\\\ dollar $ percent % Unicode λ\\r\\nnext\"\n",
    "  env_generators: ['never execute this']\nx-Yantr: {case: distinct}\n",
    "services:\n  web:\n    image: example.invalid/web:1\n    restart: 'no'\n    x-yantr: {service: [one, two]}\n",
    "networks:\n  front: {}\n",
);
fn compose(text: &str) -> Result<ComposeSource, Box<dyn Error>> {
    let loaded = LoadedProject::load([DocumentInput::new(
        ComposeId::new(1),
        DocumentOrigin::new("metadata.yaml", "metadata.yaml"),
        text,
    )])?;
    let merged = merge_project(&loaded, None);
    assert!(merged.is_valid(), "{:#?}", merged.diagnostics());
    Ok(ComposeSource::new(
        merged.project().ok_or("merged source")?.clone(),
        Identifier::new("metadata")?,
    )?
    .with_source_id(ComposeId::new(1), SourceId::new("metadata.yaml")?))
}
fn target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        "podman",
        PlatformVersion::new(5, 4, 0),
        Some(PlatformVersion::new(6, 1, 2)),
    )?)
}
fn compose_target() -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        COMPOSE_SPECIFICATION_TARGET,
        PlatformVersion::new(1, 0, 0),
        Some(PlatformVersion::new(1, 0, 0)),
    )?)
}
fn parse(files: &[(String, String)]) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(
        Identifier::new("metadata")?,
        files
            .iter()
            .zip(1_u32..)
            .map(|((name, text), id)| QuadletDocumentInput::new(name.clone(), QuadletId::new(id), text.clone())),
    )?
    .into_source())
}
fn exported() -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let result = convert(
        &ComposeImporter::new()?,
        &compose(SOURCE)?,
        &QuadletExporter::new()?,
        &target()?,
        LossPolicy::ExactOnly,
    )?;
    assert!(!result.is_blocked(), "{:#?}", result.diagnostics());
    Ok(result
        .output()
        .ok_or("Quadlet output")?
        .files()
        .iter()
        .map(|file| (file.name().as_str().to_owned(), file.text().to_owned()))
        .collect())
}
fn metadata_signature(app: &Application) -> Vec<String> {
    app.vendor_metadata()
        .iter()
        .map(|metadata| {
            format!(
                "{:?}|{:?}|{:?}",
                metadata.scope(),
                metadata.namespace().value(),
                signature_node(metadata.payload().value())
            )
        })
        .collect()
}
fn signature_node(node: &MetadataNode) -> String {
    let value = match node.value() {
        MetadataValue::Null => "null".to_owned(),
        MetadataValue::Boolean(value) => format!("boolean:{value}"),
        MetadataValue::Number(value) => format!("number:{value:?}"),
        MetadataValue::String(value) => format!("string:{value:?}"),
        MetadataValue::Sequence(values) => format!(
            "sequence:{:?}",
            values
                .iter()
                .map(|node| signature_node(node.value()))
                .collect::<Vec<_>>()
        ),
        MetadataValue::Mapping(entries) => format!(
            "mapping:{:?}",
            entries
                .iter()
                .map(|entry| (
                    format!("{:?}", entry.key().value()),
                    signature_node(entry.value().value())
                ))
                .collect::<Vec<_>>()
        ),
        _ => "unsupported future metadata variant".to_owned(),
    };
    format!("{value}|taint:{}|folded:{}", node.is_sensitive(), node.is_folded())
}

fn assert_authored_metadata(app: &Application) -> Result<(), Box<dyn Error>> {
    let metadata = app.vendor_metadata();
    assert_eq!(metadata.len(), 3);
    assert_eq!(metadata[0].scope(), &MetadataScope::Application);
    assert_eq!(metadata[0].namespace().value().expose(), "x-yantr");
    let MetadataValue::Mapping(entries) = metadata[0].payload().value().value() else {
        return Err("root map expected".into());
    };
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.key().value().expose())
            .collect::<Vec<_>>(),
        ["description", "values", "text", "env_generators"]
    );
    assert!(entries[0].value().value().is_folded());
    let MetadataValue::String(description) = entries[0].value().value().value() else {
        return Err("description string expected".into());
    };
    assert_eq!(description.expose(), "first paragraph\nsecond paragraph\n");
    let MetadataValue::Sequence(values) = entries[1].value().value().value() else {
        return Err("values sequence expected".into());
    };
    assert_eq!(values.len(), 5);
    assert!(matches!(values[0].value().value(), MetadataValue::Null));
    assert!(matches!(values[1].value().value(), MetadataValue::Boolean(true)));
    assert!(matches!(values[2].value().value(), MetadataValue::Boolean(false)));
    for (index, expected) in [(3, "123456789012345678901234567890"), (4, "1.20e+30")] {
        let MetadataValue::Number(number) = values[index].value().value() else {
            return Err("lexical number expected".into());
        };
        assert_eq!(number.expose(), expected);
    }
    let MetadataValue::String(text) = entries[2].value().value().value() else {
        return Err("escaped string expected".into());
    };
    assert_eq!(text.expose(), "quote \" slash \\ dollar $ percent % Unicode λ\r\nnext");
    assert_eq!(metadata[1].scope(), &MetadataScope::Application);
    assert_eq!(metadata[1].namespace().value().expose(), "x-Yantr");
    let MetadataValue::Mapping(distinct) = metadata[1].payload().value().value() else {
        return Err("distinct case map expected".into());
    };
    assert_eq!(distinct[0].key().value().expose(), "case");
    assert!(
        matches!(distinct[0].value().value().value(), MetadataValue::String(value) if value.expose() == "distinct")
    );
    assert_eq!(metadata[2].scope(), &MetadataScope::Service(Identifier::new("web")?));
    assert_eq!(metadata[2].namespace().value().expose(), "x-yantr");
    Ok(())
}

#[test]
fn public_nested_case_sensitive_metadata_roundtrips_fresh_bytes_and_folded_text() -> Result<(), Box<dyn Error>> {
    let imported = ComposeImporter::new()?.import(&compose(SOURCE)?);
    let original = imported.application().ok_or("application")?;
    assert_authored_metadata(original)?;
    let files = exported()?;
    assert_eq!(
        files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
        ["front.network", "web.container"]
    );
    for (_, text) in &files {
        assert!(text.contains("[X-YANTR]"));
        assert!(!text.contains("[X-BOXFERRY]"));
        for line in text.lines().filter(|line| line.starts_with("Metadata=")) {
            assert!(line.ends_with('}'));
        }
    }
    assert!(files[0].1.contains("\"kind\":\"payload\""));
    assert!(files[1].1.contains("\"kind\":\"reference\""));
    let quadlet = QuadletImporter::new()?.import(&parse(&files)?);
    assert!(quadlet.diagnostics().is_empty(), "{:#?}", quadlet.diagnostics());
    let app = quadlet.application().ok_or("fresh Quadlet import")?;
    assert_authored_metadata(app)?;
    assert_eq!(metadata_signature(original), metadata_signature(app));
    assert!(app.vendor_metadata().iter().all(|item| {
        item.payload()
            .origins()
            .iter()
            .all(|origin| origin.source_id().as_str() != "metadata.yaml")
    }));
    let generated = ComposeExporter::new()?
        .plan(app, &compose_target()?)?
        .authorize(LossPolicy::ExactOnly);
    let text = generated.output().ok_or("Compose output")?.text();
    assert!(text.contains("description: >"), "{text}");
    let reimported = ComposeImporter::new()?.import(&compose(text)?);
    assert_eq!(
        metadata_signature(app),
        metadata_signature(reimported.application().ok_or("fresh Compose import")?)
    );
    let fixed = QuadletExporter::new()?
        .plan(app, &target()?)?
        .authorize(LossPolicy::ExactOnly);
    assert_eq!(
        files,
        fixed
            .output()
            .ok_or("fixed output")?
            .files()
            .iter()
            .map(|file| (file.name().as_str().to_owned(), file.text().to_owned()))
            .collect::<Vec<_>>()
    );
    Ok(())
}
#[test]
fn missing_carrier_is_an_actionable_loss_and_identical_duplicates_merge_provenance() -> Result<(), Box<dyn Error>> {
    let files = exported()?;
    let missing = QuadletImporter::new()?.import(&parse(&files[1..])?);
    assert!(
        missing
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    assert!(format!("{:?}", missing.diagnostics()).contains("front.network"));
    assert_eq!(missing.application().ok_or("partial app")?.vendor_metadata().len(), 1);
    let mut duplicate = files.clone();
    let record = duplicate[0]
        .1
        .lines()
        .find(|line| line.starts_with("Metadata="))
        .ok_or("record")?
        .to_owned();
    duplicate[0].1.push_str(&record);
    duplicate[0].1.push('\n');
    let imported = QuadletImporter::new()?.import(&parse(&duplicate)?);
    assert!(imported.diagnostics().is_empty(), "{:#?}", imported.diagnostics());
    assert_eq!(imported.application().ok_or("app")?.vendor_metadata().len(), 3);
    Ok(())
}
#[test]
fn malformed_future_conflicting_and_misplaced_records_are_never_last_wins() -> Result<(), Box<dyn Error>> {
    let files = exported()?;
    for (from, to, kind) in [
        ("\"version\":1", "\"version\":2", ConversionKind::Unsupported),
        ("\"version\":1", "\"version\":1,\"version\":1", ConversionKind::Invalid),
        (
            "\"version\":1",
            "\"version\":1,\"unknown\":\"private-parser-canary\"",
            ConversionKind::Invalid,
        ),
        (
            "\"carrier\":\"front.network\"",
            "\"carrier\":\"web.container\"",
            ConversionKind::Invalid,
        ),
    ] {
        let changed = files
            .iter()
            .map(|(name, text)| (name.clone(), text.replace(from, to)))
            .collect::<Vec<_>>();
        let imported = QuadletImporter::new()?.import(&parse(&changed)?);
        assert!(
            imported.outcomes().iter().any(|outcome| outcome.kind() == kind),
            "{:#?}",
            imported.outcomes()
        );
        assert!(!format!("{:?}", imported.diagnostics()).contains("private-parser-canary"));
    }
    for extra in [
        r#","version":1"#,
        r#","\u0076ersion":1"#,
        r#","\u0075nknown":"private-parser-canary""#,
    ] {
        let changed = files
            .iter()
            .map(|(name, text)| {
                let text = text.replace("boxferry.vendor-metadata", r"boxferry.vendor-\u006detadata");
                (
                    name.clone(),
                    text.replace("\"version\":1", &format!("\"version\":1{extra}")),
                )
            })
            .collect::<Vec<_>>();
        let imported = QuadletImporter::new()?.import(&parse(&changed)?);
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Invalid)
        );
        let app = imported.application().ok_or("application")?;
        assert!(
            QuadletExporter::new()?
                .plan(app, &target()?)?
                .authorize(LossPolicy::AllowPartial)
                .output()
                .is_some()
        );
        let conversion = convert(
            &QuadletImporter::new()?,
            &parse(&changed)?,
            &QuadletExporter::new()?,
            &target()?,
            LossPolicy::AllowPartial,
        );
        assert!(matches!(conversion, Err(ConversionError::Import(_))));
        assert!(!format!("{:?}", imported.diagnostics()).contains("private-parser-canary"));
    }
    let mut conflict = files.clone();
    let record = conflict[0]
        .1
        .lines()
        .find(|line| line.starts_with("Metadata="))
        .ok_or("record")?
        .replace("first paragraph", "different paragraph");
    conflict[0].1.push_str(&record);
    conflict[0].1.push('\n');
    let imported = QuadletImporter::new()?.import(&parse(&conflict)?);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    Ok(())
}
fn sensitive_application(namespace_sensitive: bool) -> Result<Application, Box<dyn Error>> {
    let mut app = Application::new(Identifier::new("private")?);
    let mut service = Service::new(Identifier::new("web")?);
    service.set_image(Sourced::generated(ImageReference::parse("example.invalid/web:1")?));
    app.add_service(Sourced::generated(service))?;
    let namespace = if namespace_sensitive {
        ProtectedString::sensitive("x-namespace-private-canary")
    } else {
        ProtectedString::plain("x-public")
    };
    let value = MetadataNode::new(
        MetadataValue::Mapping(vec![MetadataEntry::new(
            Sourced::generated(ProtectedString::sensitive("key-private-canary")),
            Sourced::generated(MetadataNode::new(
                MetadataValue::String(ProtectedString::sensitive("value-private-canary")),
                true,
                false,
            )?),
        )]),
        false,
        false,
    )?;
    app.add_vendor_metadata(VendorMetadata::new(
        MetadataScope::Application,
        Sourced::generated(namespace),
        Sourced::generated(value),
    )?)?;
    Ok(app)
}
#[test]
fn sensitive_payload_taint_survives_quadlet_and_blocks_every_compose_policy() -> Result<(), Box<dyn Error>> {
    let app = sensitive_application(false)?;
    assert!(!format!("{app:?}").contains("private-canary"));
    let result = QuadletExporter::new()?
        .plan(&app, &target()?)?
        .authorize(LossPolicy::ExactOnly);
    let output = result.output().ok_or("protected Quadlet transport")?;
    assert!(!format!("{output:?}").contains("private-canary"));
    let files = output
        .files()
        .iter()
        .map(|file| (file.name().as_str().to_owned(), file.text().to_owned()))
        .collect::<Vec<_>>();
    let imported = QuadletImporter::new()?.import(&parse(&files)?);
    let app = imported.application().ok_or("protected reimport")?;
    assert!(app.vendor_metadata()[0].contains_sensitive());
    for policy in [
        LossPolicy::ExactOnly,
        LossPolicy::AllowApproximate,
        LossPolicy::AllowPartial,
    ] {
        let result = ComposeExporter::new()?.plan(app, &compose_target()?)?.authorize(policy);
        assert!(result.output().is_none());
        assert!(!format!("{result:?}").contains("private-canary"));
    }
    let app = sensitive_application(true)?;
    let plan = QuadletExporter::new()?.plan(&app, &target()?)?;
    assert!(!format!("{plan:?}").contains("private-canary"));
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    Ok(())
}
#[test]
fn podman_and_unsupported_yaml_shapes_have_explicit_value_free_losses() -> Result<(), Box<dyn Error>> {
    let imported = ComposeImporter::new()?.import(&compose(SOURCE)?);
    let app = imported.application().ok_or("app")?;
    let plan = PodmanExporter::new()?.plan(
        app,
        &TargetProfile::new(
            "podman",
            PlatformVersion::new(6, 1, 0),
            Some(PlatformVersion::new(6, 1, 0)),
        )?,
    )?;
    assert_eq!(
        plan.outcomes()
            .iter()
            .filter(|outcome| outcome.subject().starts_with("application.vendor_metadata[")
                && outcome.kind() == ConversionKind::Unsupported)
            .count(),
        3
    );
    for payload in ["{1: private-shape-canary}", "!custom private-shape-canary", ".nan"] {
        let source = compose(&format!(
            "---\nx-unsupported: {payload}\nservices:\n  web:\n    image: example.invalid/web:1\n"
        ))?;
        let imported = ComposeImporter::new()?.import(&source);
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
        );
        assert!(!format!("{:?}", imported.diagnostics()).contains("private-shape-canary"));
    }
    Ok(())
}

#[test]
fn unsupported_scope_no_unit_and_transport_limits_remain_explicit() -> Result<(), Box<dyn Error>> {
    let source = compose(
        "---\nservices:\n  web:\n    image: example.invalid/web:1\nvolumes:\n  data:\n    x-private-scope-canary: {value: inert}\n",
    )?;
    let imported = ComposeImporter::new()?.import(&source);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    assert!(!format!("{:?}", imported.diagnostics()).contains("private-scope-canary"));
    let mut app = Application::new(Identifier::new("empty")?);
    app.add_vendor_metadata(VendorMetadata::new(
        MetadataScope::Application,
        Sourced::generated(ProtectedString::plain("x-vendor")),
        Sourced::generated(MetadataNode::new(MetadataValue::Null, false, false)?),
    )?)?;
    let plan = QuadletExporter::new()?.plan(&app, &target()?)?;
    assert!(
        plan.outcomes()
            .iter()
            .any(|outcome| outcome.subject() == "application.vendor_metadata[0]"
                && outcome.kind() == ConversionKind::Unsupported)
    );
    let mut files = exported()?;
    let record = files[0]
        .1
        .lines()
        .find(|line| line.starts_with("Metadata="))
        .ok_or("record")?
        .to_owned();
    let oversized = record.replace("first paragraph", &"x".repeat(65536));
    files[0].1 = files[0].1.replace(&record, &oversized);
    let imported = QuadletImporter::new()?.import(&parse(&files)?);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    let mut runtime_missing = exported()?;
    runtime_missing[1].1 = runtime_missing[1]
        .1
        .replace("[Container]\n", "[Container]\nNetwork=front.network\n");
    let imported = QuadletImporter::new()?.import(&parse(&runtime_missing[1..])?);
    assert!(imported.application().is_none());
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    assert!(format!("{:?}", imported.diagnostics()).contains("front.network"));
    Ok(())
}

#[test]
fn ordinary_x_content_stays_unsupported_and_aggregate_records_are_bounded() -> Result<(), Box<dyn Error>> {
    let source = parse(&[("web.container".to_owned(), "[Container]\nImage=example.invalid/web:1\n[X-VENDOR]\nCustomKey=ordinary-private-canary\nMetadata=ordinary-private-canary\n".to_owned())])?;
    let imported = QuadletImporter::new()?.import(&source);
    assert_eq!(
        imported
            .outcomes()
            .iter()
            .filter(|outcome| outcome.kind() == ConversionKind::Unsupported)
            .count(),
        2
    );
    assert!(!format!("{:?}", imported.diagnostics()).contains("ordinary-private-canary"));
    let mut files = exported()?;
    let record = files[0]
        .1
        .lines()
        .find(|line| line.starts_with("Metadata="))
        .ok_or("record")?
        .to_owned();
    let encoded_len = record.strip_prefix("Metadata=").ok_or("record prefix")?.len();
    let repeat = METADATA_MAX_BYTES / encoded_len + 1;
    for _ in 0..repeat {
        files[0].1.push_str(&record);
        files[0].1.push('\n');
    }
    let imported = QuadletImporter::new()?.import(&parse(&files)?);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Invalid)
    );
    assert!(format!("{:?}", imported.diagnostics()).contains("aggregate 1 MiB"));
    Ok(())
}

#[test]
fn native_namespace_grammar_is_symmetric_on_import_and_export() -> Result<(), Box<dyn Error>> {
    let files = exported()?;
    let record = files[0]
        .1
        .lines()
        .find(|line| line.starts_with("Metadata="))
        .ok_or("record")?;
    for (namespace, section, invalid) in [
        ("x-yantr!", "X-YANTR!", true),
        ("x-vendor.name_tag", "X-VENDOR.NAME_TAG", false),
    ] {
        let text = format!("[Network]\n[{section}]\n{}\n", record.replace("x-yantr", namespace));
        let imported = QuadletImporter::new()?.import(&parse(&[("front.network".to_owned(), text)])?);
        if invalid {
            assert!(
                imported
                    .outcomes()
                    .iter()
                    .any(|outcome| outcome.kind() == ConversionKind::Invalid)
            );
            assert!(format!("{:?}", imported.diagnostics()).contains("native X- section grammar"));
            assert!(
                imported
                    .application()
                    .ok_or("application")?
                    .vendor_metadata()
                    .is_empty()
            );
        } else {
            assert!(imported.diagnostics().is_empty(), "{:?}", imported.diagnostics());
            let app = imported.application().ok_or("application")?;
            assert_eq!(app.vendor_metadata()[0].namespace().value().expose(), namespace);
            assert!(
                QuadletExporter::new()?
                    .plan(app, &target()?)?
                    .authorize(LossPolicy::ExactOnly)
                    .output()
                    .is_some()
            );
        }
    }
    Ok(())
}

#[test]
fn carrier_references_require_native_unit_basenames() -> Result<(), Box<dyn Error>> {
    let files = exported()?;
    for carrier in ["not-a-unit", "missing.unsupported", ".container", "missing."] {
        let source = parse(&[(files[1].0.clone(), files[1].1.replace("front.network", carrier))])?;
        let imported = QuadletImporter::new()?.import(&source);
        assert!(
            imported
                .outcomes()
                .iter()
                .any(|outcome| outcome.kind() == ConversionKind::Invalid),
            "{carrier}"
        );
        for diagnostic in imported.diagnostics() {
            for field in diagnostic.fields().iter().filter(|field| field.name() == "reason") {
                assert!(!field.value().expose().contains(carrier));
            }
        }
    }
    let source = parse(&[(
        files[1].0.clone(),
        files[1].1.replace("front.network", "missing.container"),
    )])?;
    let imported = QuadletImporter::new()?.import(&source);
    assert!(
        imported
            .outcomes()
            .iter()
            .any(|outcome| outcome.kind() == ConversionKind::Unsupported)
    );
    assert!(format!("{:?}", imported.diagnostics()).contains("missing.container"));
    Ok(())
}

#[cfg(feature = "cli")]
#[test]
fn cli_public_metadata_is_exact_and_protected_metadata_reports_remain_private() -> Result<(), Box<dyn Error>> {
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "boxferry-metadata-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&scratch.0)?;
    let input = scratch.0.join("compose.yaml");
    fs::write(&input, SOURCE)?;
    let protected = QuadletExporter::new()?
        .plan(&sensitive_application(false)?, &target()?)?
        .authorize(LossPolicy::ExactOnly);
    fs::write(
        scratch.0.join("web.container"),
        protected
            .output()
            .and_then(|output| output.file("web.container"))
            .ok_or("protected unit")?
            .text(),
    )?;
    for policy in ["exact", "approximate", "partial"] {
        let destination = scratch.0.join(format!("public-{policy}"));
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&input)
            .args([
                "--loss-policy",
                policy,
                "--console-format",
                "json",
                "--output-directory",
            ])
            .arg(&destination)
            .output()?;
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        let output = fs::read_to_string(destination.join("compose.yaml"))?;
        assert!(output.contains("x-yantr:"));
        assert!(output.contains("description: >"));
        let destination = scratch.0.join(format!("protected-{policy}"));
        let report = scratch.0.join(format!("report-{policy}.json"));
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "quadlet", "compose", "--input-file"])
            .arg(scratch.0.join("web.container"))
            .args([
                "--loss-policy",
                policy,
                "--environment-values",
                "include",
                "--console-format",
                "json",
                "--report-file",
            ])
            .arg(&report)
            .arg("--output-directory")
            .arg(&destination)
            .output()?;
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        for text in [
            String::from_utf8_lossy(&result.stdout).into_owned(),
            String::from_utf8_lossy(&result.stderr).into_owned(),
            fs::read_to_string(report)?,
        ] {
            assert!(!text.contains("private-canary"));
        }
        assert!(!destination.exists());
    }
    Ok(())
}

#[cfg(all(feature = "cli", unix))]
#[test]
fn protected_metadata_uses_private_native_and_bundle_outputs_without_environment_consent() -> Result<(), Box<dyn Error>>
{
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = Scratch(std::env::temp_dir().join(format!(
        "boxferry-metadata-modes-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&root.0)?;
    let generated = QuadletExporter::new()?
        .plan(&sensitive_application(false)?, &target()?)?
        .authorize(LossPolicy::ExactOnly);
    let input = root.0.join("web.container");
    fs::write(
        &input,
        generated
            .output()
            .and_then(|output| output.file("web.container"))
            .ok_or("protected unit")?
            .text(),
    )?;
    fs::set_permissions(&input, fs::Permissions::from_mode(0o600))?;
    for layout in ["files", "quadlet-bundle"] {
        for existing in [false, true] {
            let destination = root.0.join(format!("{layout}-{existing}"));
            if existing {
                fs::create_dir(&destination)?;
                fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
            }
            let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
                .args(["convert", "quadlet", "quadlet", "--input-file"])
                .arg(&input)
                .args(["--output-layout", layout, "--output-directory"])
                .arg(&destination)
                .args(["--console-format", "json"])
                .output()?;
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
            let file = destination.join(if layout == "files" {
                "web.container"
            } else {
                "quadlet.bundle"
            });
            assert_eq!(fs::metadata(file)?.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(&destination)?.permissions().mode() & 0o777,
                if existing { 0o755 } else { 0o700 }
            );
            assert!(!String::from_utf8_lossy(&result.stdout).contains("private-canary"));
        }
    }
    Ok(())
}
