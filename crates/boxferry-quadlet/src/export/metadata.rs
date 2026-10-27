//! Place metadata only after the full final unit filename set is known.
use super::Mapping;
use boxferry_model::MetadataScope;
use quadlet_lens::render::{ExtensionValue, XSectionName};
impl Mapping<'_> {
    pub(super) fn finish_staged_documents(&mut self) {
        let mut staged = std::mem::take(&mut self.staged);
        let carrier = staged
            .iter()
            .map(|(filename, _, _, _)| filename.as_str())
            .min()
            .map(str::to_owned);
        let mut total = 0_usize;
        for (index, metadata) in self.application.vendor_metadata().iter().enumerate() {
            let subject = format!("application.vendor_metadata[{index}]");
            if metadata.namespace().value().is_sensitive() {
                self.unsupported(
                    &subject,
                    "protected metadata namespaces cannot become printable vendor sections; retain the original source",
                    metadata.namespace().origins(),
                );
                continue;
            }
            let Ok(section) = XSectionName::new(metadata.namespace().value().expose().to_ascii_uppercase()) else {
                self.unsupported(&subject, "vendor namespace cannot be encoded as an X- section; use an ASCII alphanumeric namespace with dots, underscores or hyphens", metadata.namespace().origins());
                continue;
            };
            let owner = match metadata.scope() {
                MetadataScope::Application => carrier.clone(),
                MetadataScope::Service(owner) => Some(format!("{}.container", owner.as_str())),
                _ => None,
            };
            let Some(owner) = owner.filter(|owner| staged.iter().any(|(name, _, _, _)| name == owner)) else {
                self.unsupported(&subject, "metadata has no generated owning unit; retain the source metadata separately or generate its owning service unit", metadata.payload().origins());
                continue;
            };
            let mut records = Vec::new();
            let mut error = None;
            for (unit_index, (name, _, _, _)) in staged.iter().enumerate() {
                if name != &owner && metadata.scope() != &MetadataScope::Application {
                    continue;
                }
                match crate::metadata::encode(metadata, &owner, name != &owner).and_then(|text| {
                    let len = text.len();
                    ExtensionValue::new(text)
                        .map(|value| (value, len))
                        .map_err(|_| "metadata cannot be encoded safely as a native extension record")
                }) {
                    Ok((value, len)) => records.push((unit_index, value, len)),
                    Err(reason) => {
                        error = Some(reason);
                        break;
                    }
                }
            }
            let bytes: usize = records.iter().map(|(_, _, len)| len).sum();
            if total.saturating_add(bytes) > boxferry_model::METADATA_MAX_BYTES {
                error = Some(
                    "generated metadata exceeds the aggregate 1 MiB transport limit; reduce metadata or unit count",
                );
            }
            if let Some(reason) = error {
                self.unsupported(&subject, reason, metadata.payload().origins());
                continue;
            }
            total += bytes;
            let mut failed = false;
            for (unit_index, value, _) in records {
                if staged[unit_index]
                    .1
                    .push_extension_entry(section.clone(), "Metadata", value)
                    .is_err()
                {
                    failed = true;
                }
            }
            if failed {
                self.generation_failed = true;
                self.unsupported(
                    &subject,
                    "native generation rejected metadata placement; retain the source document",
                    metadata.payload().origins(),
                );
            } else {
                self.exact(&subject, metadata.payload().origins());
            }
        }
        for (filename, builder, subject, origins) in staged {
            self.build_document(filename, &builder, &subject, &origins);
        }
    }
}
