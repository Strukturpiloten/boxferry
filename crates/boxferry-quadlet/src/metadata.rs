//! Versioned, inert vendor metadata transport. No native runtime semantics are interpreted.
use boxferry_model::{
    MetadataEntry, MetadataNode, MetadataScope, MetadataValue, ProtectedString, Provenance, Sourced, VendorMetadata,
};
use serde::{Deserialize, Serialize};

pub(crate) const RECORD_LIMIT: usize = 64 * 1024;
const WIRE_DEPTH: usize = 16;
const PROTOCOL: &str = "boxferry.vendor-metadata";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    protocol: String,
    version: u8,
    pub namespace: Text,
    pub scope: Scope,
    pub carrier: String,
    pub content: Content,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Text {
    pub value: String,
    pub sensitive: bool,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "owner", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Scope {
    Application,
    Service(String),
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Content {
    Payload(Node),
    Reference,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node {
    sensitive: bool,
    folded: bool,
    value: Value,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
enum Value {
    Null,
    Boolean(bool),
    Number(String),
    String(String),
    Sequence(Vec<Node>),
    Mapping(Vec<Entry>),
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: Text,
    value: Node,
}

pub(crate) fn encode(metadata: &VendorMetadata, carrier: &str, reference: bool) -> Result<String, &'static str> {
    let scope = match metadata.scope() {
        MetadataScope::Application => Scope::Application,
        MetadataScope::Service(owner) => Scope::Service(owner.as_str().to_owned()),
        _ => return Err("unsupported metadata owner"),
    };
    let record = Record {
        protocol: PROTOCOL.to_owned(),
        version: 1,
        namespace: Text {
            value: metadata.namespace().value().expose().to_owned(),
            sensitive: metadata.namespace().value().is_sensitive(),
        },
        scope,
        carrier: carrier.to_owned(),
        content: if reference {
            Content::Reference
        } else {
            Content::Payload(Node::encode(metadata.payload().value(), 1)?)
        },
    };
    let text = serde_json::to_string(&record).map_err(|_| "metadata encoding failed")?;
    if text.len() > RECORD_LIMIT {
        return Err("metadata record exceeds 64 KiB; reduce the extension payload");
    }
    Ok(text)
}
pub(crate) fn decode(text: &str) -> Result<Record, &'static str> {
    if text.len() > RECORD_LIMIT {
        return Err("metadata record exceeds 64 KiB; reduce the extension payload");
    }
    let record: Record = serde_json::from_str(text).map_err(|_| {
        // Classify only the native record shape, never a spelling-sensitive protocol substring
        // or a duplicate-blind JSON peek. Escaped field/protocol spellings remain strict.
        if text.trim_start().starts_with(['{', '[']) {
            "metadata record is malformed, has duplicate or unknown fields, or exceeds JSON nesting bounds"
        } else {
            "ordinary X- content has an unsupported metadata protocol; retain the original unit separately"
        }
    })?;
    if record.protocol != PROTOCOL || record.version != 1 {
        return Err("metadata protocol/version is unsupported; use vendor metadata version 1");
    }
    if record.namespace.sensitive {
        return Err("protected metadata namespaces cannot be transported as printable vendor sections");
    }
    if !record.namespace.value.starts_with("x-") || record.namespace.value.len() <= 2 {
        return Err("metadata namespace is invalid");
    }
    quadlet_lens::render::XSectionName::new(record.namespace.value.to_ascii_uppercase())
        .map_err(|_| "metadata namespace does not fit the native X- section grammar")?;
    if record.carrier.is_empty()
        || record.carrier.contains(['/', '\\'])
        || matches!(record.carrier.as_str(), "." | "..")
    {
        return Err("metadata carrier must be an exact unit basename");
    }
    quadlet_lens::model::UnitFileName::new(record.carrier.clone())
        .map_err(|_| "metadata carrier is not a valid native Quadlet unit basename")?;
    if let Content::Payload(node) = &record.content {
        node.decode(&[], 1)?;
    }
    Ok(record)
}
impl Record {
    pub(crate) fn metadata(
        &self,
        origins: &[Provenance],
        payload_origins: &[Provenance],
    ) -> Result<VendorMetadata, &'static str> {
        let scope = match &self.scope {
            Scope::Application => MetadataScope::Application,
            Scope::Service(owner) => MetadataScope::Service(
                boxferry_model::Identifier::new(owner).map_err(|_| "metadata service owner is invalid")?,
            ),
        };
        let Content::Payload(node) = &self.content else {
            return Err("metadata carrier has no payload");
        };
        VendorMetadata::new(
            scope,
            sourced(protect(&self.namespace.value, self.namespace.sensitive), origins),
            sourced(node.decode(payload_origins, 1)?, origins),
        )
        .map_err(|_| "metadata payload violates the neutral contract")
    }
}
impl Node {
    fn encode(node: &MetadataNode, depth: usize) -> Result<Self, &'static str> {
        if depth > WIRE_DEPTH {
            return Err("Quadlet metadata v1 supports depth 16; simplify the extension nesting");
        }
        let value = match node.value() {
            MetadataValue::Null => Value::Null,
            MetadataValue::Boolean(value) => Value::Boolean(*value),
            MetadataValue::Number(value) => Value::Number(value.expose().to_owned()),
            MetadataValue::String(value) => Value::String(value.expose().to_owned()),
            MetadataValue::Sequence(values) => Value::Sequence(
                values
                    .iter()
                    .map(|node| Self::encode(node.value(), depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            MetadataValue::Mapping(entries) => Value::Mapping(
                entries
                    .iter()
                    .map(|entry| {
                        Ok(Entry {
                            key: Text {
                                value: entry.key().value().expose().to_owned(),
                                sensitive: entry.key().value().is_sensitive(),
                            },
                            value: Self::encode(entry.value().value(), depth + 1)?,
                        })
                    })
                    .collect::<Result<_, &'static str>>()?,
            ),
            _ => return Err("metadata contains an unsupported neutral value"),
        };
        Ok(Self {
            sensitive: node.is_sensitive(),
            folded: node.is_folded(),
            value,
        })
    }
    fn decode(&self, origins: &[Provenance], depth: usize) -> Result<MetadataNode, &'static str> {
        if depth > WIRE_DEPTH {
            return Err("Quadlet metadata v1 supports depth 16; simplify the extension nesting");
        }
        let value = match &self.value {
            Value::Null => MetadataValue::Null,
            Value::Boolean(value) => MetadataValue::Boolean(*value),
            Value::Number(value) => MetadataValue::Number(protect(value, self.sensitive)),
            Value::String(value) => MetadataValue::String(protect(value, self.sensitive)),
            Value::Sequence(values) => MetadataValue::Sequence(
                values
                    .iter()
                    .map(|value| Ok(sourced(value.decode(origins, depth + 1)?, origins)))
                    .collect::<Result<_, &'static str>>()?,
            ),
            Value::Mapping(entries) => MetadataValue::Mapping(
                entries
                    .iter()
                    .map(|entry| {
                        Ok(MetadataEntry::new(
                            sourced(protect(&entry.key.value, entry.key.sensitive), origins),
                            sourced(entry.value.decode(origins, depth + 1)?, origins),
                        ))
                    })
                    .collect::<Result<_, &'static str>>()?,
            ),
        };
        MetadataNode::new(value, self.sensitive, self.folded)
            .map_err(|_| "metadata node has invalid numbers, keys, hints or size")
    }
}
fn sourced<T>(value: T, origins: &[Provenance]) -> Sourced<T> {
    let mut value = Sourced::generated(value);
    for origin in origins {
        value.add_origin(origin.clone());
    }
    value
}
fn protect(value: &str, sensitive: bool) -> ProtectedString {
    if sensitive {
        ProtectedString::sensitive(value)
    } else {
        ProtectedString::plain(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    fn metadata(node: MetadataNode) -> Result<VendorMetadata, boxferry_model::MetadataError> {
        VendorMetadata::new(
            MetadataScope::Application,
            Sourced::generated(ProtectedString::plain("x-vendor")),
            Sourced::generated(node),
        )
    }
    fn nested(depth: usize) -> Result<MetadataNode, boxferry_model::MetadataError> {
        let mut node = MetadataNode::new(MetadataValue::String(ProtectedString::plain("value")), false, false)?;
        for _ in 1..depth {
            let value = MetadataValue::Mapping(vec![MetadataEntry::new(
                Sourced::generated(ProtectedString::plain("key")),
                Sourced::generated(node),
            )]);
            node = MetadataNode::new(value, false, false)?;
        }
        Ok(node)
    }
    #[test]
    fn worst_case_depth_sixteen_fits_default_json_guard_and_seventeen_is_explicit_loss() -> Result<(), Box<dyn Error>> {
        let encoded = encode(&metadata(nested(WIRE_DEPTH)?)?, "web.container", false)?;
        assert!(decode(&encoded).is_ok());
        assert!(
            encode(&metadata(nested(WIRE_DEPTH + 1)?)?, "web.container", false)
                .is_err_and(|reason| reason.contains("depth 16"))
        );
        Ok(())
    }
    #[test]
    fn record_bounds_and_strict_nested_numbers_keys_and_fields_are_validated() -> Result<(), Box<dyn Error>> {
        let node = MetadataNode::new(
            MetadataValue::String(ProtectedString::plain("x".repeat(RECORD_LIMIT))),
            false,
            false,
        )?;
        assert!(encode(&metadata(node)?, "web.container", false).is_err_and(|reason| reason.contains("64 KiB")));
        let valid = encode(&metadata(nested(2)?)?, "web.container", false)?;
        for (from, to) in [
            ("\"folded\":false", "\"folded\":false,\"folded\":true"),
            ("\"sensitive\":false", "\"sensitive\":false,\"private-canary\":true"),
            (
                "\"type\":\"string\",\"value\":\"value\"",
                "\"type\":\"number\",\"value\":\"01\"",
            ),
        ] {
            assert!(decode(&valid.replace(from, to)).is_err());
        }
        Ok(())
    }
}
