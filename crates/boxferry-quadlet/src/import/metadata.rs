//! Resolve bounded metadata records solely within the caller-supplied document set.
use super::Mapping;
use crate::metadata::{Content, Record, Scope};
use boxferry_model::{Application, Provenance};

impl Mapping<'_> {
    pub(super) fn import_metadata(&mut self, application: &mut Application) {
        let mut groups = Vec::<((u8, String, String), Vec<(Record, String, Provenance)>)>::new();
        let mut bytes = 0_usize;
        let subject = "application.vendor_metadata";
        for named in self.source.documents().documents() {
            let filename = named.name().as_str();
            for section in named
                .document()
                .sections()
                .iter()
                .filter(|section| section.name().text().starts_with("X-"))
            {
                for entry in section.entries() {
                    let origin = self.entry_origin(entry);
                    if entry.key().text() != "Metadata" {
                        self.unsupported_value(subject, filename, "Metadata", "ordinary X- section content has no reviewed inert metadata schema; retain the source unit separately", origin);
                        continue;
                    }
                    let text = entry.value().primary().text();
                    bytes = bytes.saturating_add(text.len());
                    if bytes > boxferry_model::METADATA_MAX_BYTES {
                        self.invalid_model(
                            subject,
                            filename,
                            "metadata records exceed the aggregate 1 MiB limit",
                            Some(origin),
                        );
                        return;
                    }
                    if !entry.value().continuations().is_empty() {
                        self.invalid_model(
                            subject,
                            filename,
                            "metadata records must occupy one physical line",
                            Some(origin),
                        );
                        continue;
                    }
                    let record = match crate::metadata::decode(text) {
                        Ok(record) => record,
                        Err(reason) => {
                            if reason.contains("unsupported")
                                || reason.contains("supports depth")
                                || reason.contains("exceeds 64 KiB")
                            {
                                self.unsupported_value(subject, filename, "Metadata", reason, origin);
                            } else {
                                self.invalid_model(subject, filename, reason, Some(origin));
                            }
                            continue;
                        }
                    };
                    if section.name().text() != record.namespace.value.to_ascii_uppercase() {
                        self.invalid_model(
                            subject,
                            filename,
                            "metadata namespace disagrees with its vendor section",
                            Some(origin),
                        );
                        continue;
                    }
                    let (kind, owner) = match &record.scope {
                        Scope::Application => (0, String::new()),
                        Scope::Service(owner) => (1, owner.clone()),
                    };
                    let identity = (kind, owner, record.namespace.value.clone());
                    let index = groups.iter().position(|(key, _)| key == &identity).unwrap_or_else(|| {
                        groups.push((identity, Vec::new()));
                        groups.len() - 1
                    });
                    groups[index].1.push((record, filename.to_owned(), origin));
                }
            }
        }
        for (_, records) in groups {
            self.resolve_metadata_group(application, &records);
        }
    }

    fn resolve_metadata_group(&mut self, application: &mut Application, records: &[(Record, String, Provenance)]) {
        let subject = "application.vendor_metadata";
        let first_unit = self
            .source
            .documents()
            .documents()
            .iter()
            .map(|named| named.name().as_str())
            .min();
        let origins = records.iter().map(|(_, _, origin)| origin.clone()).collect::<Vec<_>>();
        let first = &records[0].0;
        let carrier = &first.carrier;
        let misplaced = records.iter().any(|(record, filename, _)| {
            record.carrier != *carrier
                || match &record.content {
                    Content::Payload(_) => filename != carrier,
                    Content::Reference => filename == carrier || !matches!(record.scope, Scope::Application),
                }
        }) || matches!(&first.scope, Scope::Service(owner) if carrier != &format!("{owner}.container"));
        if misplaced {
            self.invalid_model(
                subject,
                carrier,
                "conflicting, misplaced or wrong-owner metadata records; restore the canonical carrier payload",
                origins.first().cloned(),
            );
            return;
        }
        if !self
            .source
            .documents()
            .documents()
            .iter()
            .any(|named| named.name().as_str() == carrier)
        {
            self.unsupported_value_with_origins(
                subject,
                "quadlet",
                "Metadata",
                &format!(
                    "metadata carrier `{carrier}` is missing; supply that exact unit basename with this document set"
                ),
                origins,
            );
            return;
        }
        let correct_owner = match &first.scope {
            Scope::Application => first_unit == Some(carrier.as_str()),
            Scope::Service(owner) => carrier == &format!("{owner}.container"),
        };
        let payloads = records
            .iter()
            .filter_map(|(record, _, _)| match &record.content {
                Content::Payload(node) => Some(node),
                Content::Reference => None,
            })
            .collect::<Vec<_>>();
        let conflict = !correct_owner || payloads.windows(2).any(|pair| pair[0] != pair[1]);
        if conflict {
            self.invalid_model(subject, carrier, "conflicting, misplaced or wrong-owner metadata records; supply one matching payload on its canonical carrier", origins.first().cloned());
            return;
        }
        let Some((record, _, _)) = records
            .iter()
            .find(|(record, filename, _)| filename == carrier && matches!(record.content, Content::Payload(_)))
        else {
            self.unsupported_value_with_origins(
                subject,
                carrier,
                "Metadata",
                "metadata carrier has no matching payload; restore the carrier's Metadata directive",
                origins,
            );
            return;
        };
        let payload_origins = records
            .iter()
            .filter(|(record, _, _)| matches!(record.content, Content::Payload(_)))
            .map(|(_, _, origin)| origin.clone())
            .collect::<Vec<_>>();
        match record.metadata(&origins, &payload_origins).and_then(|metadata| {
            application
                .add_vendor_metadata(metadata)
                .map_err(|_| "metadata has an absent service owner, duplicate identity or aggregate limit violation")
        }) {
            Ok(()) => {
                for origin in origins {
                    self.exact(subject, Some(origin));
                }
            }
            Err(reason) => self.invalid_model(subject, carrier, reason, origins.first().cloned()),
        }
    }
}
