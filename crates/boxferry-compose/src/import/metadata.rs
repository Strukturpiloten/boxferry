//! Inert root/direct-service vendor extension import through the merged native view.
use super::Mapping;
use boxferry_model::{
    Application, MetadataEntry, MetadataNode, MetadataScope, MetadataValue, ProtectedString, Sourced, VendorMetadata,
};
use compose_lens::{
    merge::{MergedEntry, MergedScalarKind, MergedValue, MergedValueKind},
    syntax::ScalarPresentation,
};

impl Mapping<'_> {
    pub(super) fn import_metadata(&mut self, application: &mut Application) {
        let root = self.source.project().root();
        self.import_metadata_scope(application, root, &MetadataScope::Application);
        if let Some(services) = root.get("services").and_then(MergedValue::as_mapping) {
            for service in services {
                if let Some(owner) = application
                    .services()
                    .iter()
                    .find(|owner| owner.value().name().as_str() == service.key())
                    .map(|owner| owner.value().name().clone())
                {
                    self.import_metadata_scope(application, service.value(), &MetadataScope::Service(owner));
                }
            }
        }
        self.report_other_metadata_scopes(application);
    }
    fn report_other_metadata_scopes(&mut self, application: &Application) {
        #[derive(Clone, Copy)]
        enum Location {
            Root,
            Services,
            Service,
            Other,
        }
        let mut stack = vec![(self.source.project().root(), Location::Root)];
        let mut index = 0_usize;
        while let Some((value, location)) = stack.pop() {
            match value.kind() {
                MergedValueKind::Mapping(entries) => {
                    for entry in entries.iter().rev() {
                        if matches!(location, Location::Services) {
                            if application
                                .services()
                                .iter()
                                .any(|service| service.value().name().as_str() == entry.key())
                            {
                                stack.push((entry.value(), Location::Service));
                            }
                            continue;
                        }
                        if entry.key().starts_with("x-") {
                            if !matches!(location, Location::Root | Location::Service) {
                                self.unsupported_optional(&format!("application.vendor_metadata.unsupported_scopes[{index}]"), "unsupported vendor metadata scope; only application-root and direct-service extensions are retained", entry.value().provenance().effective_source());
                                index += 1;
                            }
                            continue;
                        }
                        let child = if matches!(location, Location::Root) && entry.key() == "services" {
                            Location::Services
                        } else {
                            Location::Other
                        };
                        stack.push((entry.value(), child));
                    }
                }
                MergedValueKind::Sequence(values) => {
                    for value in values.iter().rev() {
                        stack.push((value, Location::Other));
                    }
                }
                _ => {}
            }
        }
    }

    fn import_metadata_scope(&mut self, application: &mut Application, value: &MergedValue, scope: &MetadataScope) {
        for entry in value
            .as_mapping()
            .unwrap_or_default()
            .iter()
            .filter(|entry| entry.key().starts_with("x-"))
        {
            let subject = format!("application.vendor_metadata[{}]", application.vendor_metadata().len());
            let result = if entry.is_strict_yaml_string() {
                self.metadata_node(entry.value(), 1).and_then(|node| {
                    VendorMetadata::new(
                        scope.clone(),
                        self.sourced_spans(protect(entry.key(), entry.is_key_sensitive()), entry.key_sources()),
                        node,
                    )
                    .map_err(|_| "metadata namespace or payload exceeds the neutral contract")
                })
            } else {
                Err("metadata namespace must be a YAML string")
            };
            match result.and_then(|metadata| {
                application
                    .add_vendor_metadata(metadata)
                    .map_err(|_| "metadata has a duplicate identity, absent owner or exceeds the aggregate limit")
            }) {
                Ok(()) => self.exact_provenance(&subject, entry.value().provenance()),
                Err(reason) => {
                    self.unsupported_optional(&subject, reason, entry.value().provenance().effective_source());
                }
            }
        }
    }
    fn metadata_node(&self, native: &MergedValue, depth: usize) -> Result<Sourced<MetadataNode>, &'static str> {
        if depth > boxferry_model::METADATA_MAX_DEPTH {
            return Err("metadata exceeds the supported depth; simplify the nested extension");
        }
        let mut sensitive = false;
        let mut folded = false;
        let value = match native.kind() {
            MergedValueKind::Null(_) => MetadataValue::Null,
            MergedValueKind::Scalar(scalar) => {
                sensitive = scalar.is_sensitive();
                match scalar.kind() {
                    MergedScalarKind::String if scalar.is_strict_yaml_string() => {
                        folded = scalar.presentation() == ScalarPresentation::Folded;
                        MetadataValue::String(protect(scalar.value(), sensitive))
                    }
                    MergedScalarKind::Boolean => match scalar.value() {
                        "true" | "True" | "TRUE" => MetadataValue::Boolean(true),
                        "false" | "False" | "FALSE" => MetadataValue::Boolean(false),
                        _ => return Err("metadata boolean spelling is unsupported; use true or false"),
                    },
                    MergedScalarKind::Number => MetadataValue::Number(protect(scalar.value(), sensitive)),
                    MergedScalarKind::String => {
                        return Err("metadata scalar is not a supported string, boolean or decimal number");
                    }
                }
            }
            MergedValueKind::Sequence(values) => MetadataValue::Sequence(
                values
                    .iter()
                    .map(|value| self.metadata_node(value, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            MergedValueKind::Mapping(entries) => MetadataValue::Mapping(
                entries
                    .iter()
                    .map(|entry| self.metadata_entry(entry, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            _ => return Err("metadata tags or unresolved aliases are unsupported; use resolved ordinary YAML values"),
        };
        let node = MetadataNode::new(value, sensitive, folded)
            .map_err(|_| "metadata contains unsupported numbers, duplicate keys or exceeds supported bounds")?;
        Ok(self.sourced_provenance(node, native.provenance()))
    }
    fn metadata_entry(&self, entry: &MergedEntry, depth: usize) -> Result<MetadataEntry, &'static str> {
        if !entry.is_strict_yaml_string() {
            return Err("metadata mapping keys must be YAML strings");
        }
        Ok(MetadataEntry::new(
            self.sourced_spans(protect(entry.key(), entry.is_key_sensitive()), entry.key_sources()),
            self.metadata_node(entry.value(), depth)?,
        ))
    }
}
fn protect(text: &str, sensitive: bool) -> ProtectedString {
    if sensitive {
        ProtectedString::sensitive(text)
    } else {
        ProtectedString::plain(text)
    }
}
