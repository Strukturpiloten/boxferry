//! Independent invariants for bounded inert vendor metadata.
use boxferry_model::*;
use std::error::Error;
fn node(value: MetadataValue) -> Result<MetadataNode, MetadataError> {
    MetadataNode::new(value, false, false)
}
fn extension(scope: MetadataScope, namespace: &str, payload: MetadataNode) -> Result<VendorMetadata, MetadataError> {
    VendorMetadata::new(
        scope,
        Sourced::generated(ProtectedString::plain(namespace)),
        Sourced::generated(payload),
    )
}
#[test]
fn exact_scope_namespace_identity_and_owner_are_validated() -> Result<(), Box<dyn Error>> {
    let mut app = Application::new(Identifier::new("app")?);
    let owner = Identifier::new("web")?;
    assert_eq!(
        app.add_vendor_metadata(extension(
            MetadataScope::Service(owner.clone()),
            "x-vendor",
            node(MetadataValue::Null)?
        )?),
        Err(MetadataError::UnknownOwner)
    );
    app.add_service(Sourced::generated(Service::new(owner.clone())))?;
    for scope in [MetadataScope::Application, MetadataScope::Service(owner)] {
        for key in ["x-vendor", "x-Vendor"] {
            app.add_vendor_metadata(extension(scope.clone(), key, node(MetadataValue::Null)?)?)?;
        }
    }
    assert_eq!(app.vendor_metadata().len(), 4);
    assert_eq!(
        app.add_vendor_metadata(extension(
            MetadataScope::Application,
            "x-vendor",
            node(MetadataValue::Null)?
        )?),
        Err(MetadataError::Duplicate)
    );
    Ok(())
}
#[test]
fn decimal_text_depth_size_and_duplicate_keys_have_finite_bounds() -> Result<(), Box<dyn Error>> {
    for number in ["0", "-0", "1.20e+3000", "123456789012345678901234567890"] {
        assert!(MetadataNode::new(MetadataValue::Number(ProtectedString::plain(number)), false, false).is_ok());
    }
    for number in ["01", "NaN", ".inf", "1.", "+1", "", "1e"] {
        assert!(MetadataNode::new(MetadataValue::Number(ProtectedString::plain(number)), false, false).is_err());
    }
    let mut nested = node(MetadataValue::Null)?;
    for _ in 1..METADATA_MAX_DEPTH {
        nested = node(MetadataValue::Sequence(vec![Sourced::generated(nested)]))?;
    }
    assert!(MetadataNode::new(MetadataValue::Sequence(vec![Sourced::generated(nested)]), false, false).is_err());
    assert!(
        MetadataNode::new(
            MetadataValue::String(ProtectedString::plain("a".repeat(METADATA_MAX_BYTES))),
            false,
            false
        )
        .is_err()
    );
    let entry = MetadataEntry::new(
        Sourced::generated(ProtectedString::plain("same")),
        Sourced::generated(node(MetadataValue::Null)?),
    );
    assert_eq!(
        MetadataNode::new(MetadataValue::Mapping(vec![entry.clone(), entry]), false, false),
        Err(MetadataError::Duplicate)
    );
    assert!(MetadataNode::new(MetadataValue::Null, false, true).is_err());
    Ok(())
}
#[test]
fn debug_redacts_tainted_values_keys_and_namespaces() -> Result<(), Box<dyn Error>> {
    let secret = "metadata-private-canary";
    let value = MetadataNode::new(MetadataValue::String(ProtectedString::plain(secret)), true, false)?;
    assert!(!format!("{value:?}").contains(secret));
    assert!(!format!("{:?}", value.value()).contains(secret));
    let map = node(MetadataValue::Mapping(vec![MetadataEntry::new(
        Sourced::generated(ProtectedString::sensitive(secret)),
        Sourced::generated(value),
    )]))?;
    assert!(map.contains_sensitive());
    assert!(!format!("{map:?}").contains(secret));
    let extension = VendorMetadata::new(
        MetadataScope::Application,
        Sourced::generated(ProtectedString::sensitive(format!("x-{secret}"))),
        Sourced::generated(map),
    )?;
    assert!(!format!("{extension:?}").contains(secret));
    Ok(())
}

#[test]
fn protected_parent_masks_directly_extracted_keys_children_and_values() -> Result<(), Box<dyn Error>> {
    let origin = Provenance::source(SourceId::new("metadata.yaml")?);
    let child = MetadataNode::new(
        MetadataValue::String(ProtectedString::plain("child-private-canary")),
        false,
        false,
    )?;
    let map = MetadataNode::new(
        MetadataValue::Mapping(vec![MetadataEntry::new(
            Sourced::from_source(ProtectedString::plain("key-private-canary"), origin.clone()),
            Sourced::from_source(
                MetadataNode::new(
                    MetadataValue::Sequence(vec![Sourced::from_source(child, origin.clone())]),
                    false,
                    false,
                )?,
                origin.clone(),
            ),
        )]),
        true,
        false,
    )?;
    assert!(!format!("{:?}", map.value()).contains("private-canary"));
    let MetadataValue::Mapping(entries) = map.value() else {
        return Err("mapping expected".into());
    };
    assert!(entries[0].key().value().is_sensitive());
    assert_eq!(entries[0].key().origins(), &[origin]);
    assert!(!format!("{:?}", entries[0].key().value()).contains("private-canary"));
    assert!(!format!("{:?}", entries[0].value().value().value()).contains("private-canary"));
    let MetadataValue::Sequence(children) = entries[0].value().value().value() else {
        return Err("sequence expected".into());
    };
    assert!(children[0].value().is_sensitive());
    assert!(!format!("{:?}", children[0].value().value()).contains("private-canary"));
    Ok(())
}
