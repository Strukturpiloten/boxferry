//! Reconstruct inert metadata using native typed extension generation.
use super::Mapping;
use boxferry_model::{MetadataNode, MetadataScope, MetadataValue};
use compose_lens::render::{
    ComposeDocumentBuilder, GeneratedExtension, GeneratedExtensionEntry, GeneratedExtensionValue, GeneratedService,
    GeneratedString,
};

impl Mapping<'_> {
    pub(super) fn map_root_metadata(&mut self, builder: &mut ComposeDocumentBuilder) {
        for (index, metadata) in self.application.vendor_metadata().iter().enumerate() {
            if metadata.scope() != &MetadataScope::Application {
                continue;
            }
            if let Some(extension) = self.metadata_extension(index, metadata) {
                match builder.add_extension(extension) {
                    Ok(()) => self.exact(
                        format!("application.vendor_metadata[{index}]"),
                        metadata.payload().origins(),
                    ),
                    Err(_) => self.unsupported(
                        &format!("application.vendor_metadata[{index}]"),
                        "native metadata generation rejected this extension; retain it in its source document",
                        metadata.payload().origins(),
                    ),
                }
            }
        }
    }
    pub(super) fn map_service_metadata(
        &mut self,
        owner: &boxferry_model::Identifier,
        generated: &mut GeneratedService,
    ) {
        for (index, metadata) in self.application.vendor_metadata().iter().enumerate() {
            if metadata.scope() != &MetadataScope::Service(owner.clone()) {
                continue;
            }
            if let Some(extension) = self.metadata_extension(index, metadata) {
                match generated.add_extension(extension) {
                    Ok(()) => self.exact(
                        format!("application.vendor_metadata[{index}]"),
                        metadata.payload().origins(),
                    ),
                    Err(_) => self.unsupported(
                        &format!("application.vendor_metadata[{index}]"),
                        "native metadata generation rejected this extension; retain it in its source document",
                        metadata.payload().origins(),
                    ),
                }
            }
        }
    }
    fn metadata_extension(
        &mut self,
        index: usize,
        metadata: &boxferry_model::VendorMetadata,
    ) -> Option<GeneratedExtension> {
        let subject = format!("application.vendor_metadata[{index}]");
        if metadata.contains_sensitive() {
            self.protected_metadata = true;
            self.unsupported(&subject, "protected vendor metadata cannot retain sensitivity in fresh Compose text; keep it in the original source or use Quadlet metadata transport", metadata.payload().origins());
            return None;
        }
        let result = GeneratedString::plain(metadata.namespace().value().expose()).and_then(|namespace| {
            node(metadata.payload().value()).and_then(|payload| GeneratedExtension::new(namespace, payload))
        });
        if let Ok(value) = result {
            Some(value)
        } else {
            self.unsupported(
                &subject,
                "vendor metadata cannot be rendered safely by the native Compose generator; simplify its values",
                metadata.payload().origins(),
            );
            None
        }
    }
}
fn node(value: &MetadataNode) -> Result<GeneratedExtensionValue, compose_lens::render::GenerationError> {
    match value.value() {
        MetadataValue::Null => Ok(GeneratedExtensionValue::null()),
        MetadataValue::Boolean(value) => Ok(GeneratedExtensionValue::boolean(*value)),
        MetadataValue::Number(value) => GeneratedExtensionValue::number(GeneratedString::plain(value.expose())?),
        MetadataValue::String(text) => {
            let text = GeneratedString::plain(text.expose())?;
            if value.is_folded() {
                GeneratedExtensionValue::folded_string(text)
            } else {
                Ok(GeneratedExtensionValue::string(text))
            }
        }
        MetadataValue::Sequence(values) => GeneratedExtensionValue::sequence(
            values
                .iter()
                .map(|value| node(value.value()))
                .collect::<Result<_, _>>()?,
        ),
        MetadataValue::Mapping(entries) => GeneratedExtensionValue::mapping(
            entries
                .iter()
                .map(|entry| {
                    Ok(GeneratedExtensionEntry::new(
                        GeneratedString::plain(entry.key().value().expose())?,
                        node(entry.value().value())?,
                    ))
                })
                .collect::<Result<_, compose_lens::render::GenerationError>>()?,
        ),
        _ => unreachable!("current neutral metadata variants"),
    }
}
