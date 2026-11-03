//! Restart delay activates the separately modelled deployment restart contract.
use std::time::Duration;

use boxferry_model::{DeploymentRestartCondition, DeploymentRestartPolicy, Provenance, Service, Sourced};
use quadlet_lens::model::{QuadletDocument, SectionKind, TypedSection};

use super::Mapping;

impl Mapping<'_> {
    pub(super) fn map_deployment_restart(
        &mut self,
        filename: &str,
        service_name: &str,
        service: &mut Service,
        document: &QuadletDocument,
    ) {
        let entries = document
            .sections()
            .iter()
            .filter(|section| section.kind() == SectionKind::Service)
            .flat_map(TypedSection::entries)
            .filter(|entry| matches!(entry.key().text(), "Restart" | "RestartSec"))
            .collect::<Vec<_>>();
        let subject = format!("services.{service_name}.deploy.restart_policy");
        let mut condition = None;
        let mut delay = None;
        let mut origins = Vec::new();
        for entry in entries {
            let origin = self.entry_origin(entry);
            origins.push(origin.clone());
            let Some(value) = self.direct_value(filename, &subject, entry, origin.clone()) else {
                return;
            };
            match entry.key().text() {
                "Restart" => {
                    if condition.is_some() {
                        self.invalid_model(
                            &subject,
                            filename,
                            "systemd Restart is declared more than once",
                            Some(origin),
                        );
                        return;
                    }
                    let value = match value {
                        "no" => DeploymentRestartCondition::Never,
                        "on-failure" => DeploymentRestartCondition::OnFailure,
                        "always" => DeploymentRestartCondition::Any,
                        _ => {
                            self.unsupported_value(
                                &subject,
                                filename,
                                "Restart",
                                "restart condition has no supported deployment-policy representation",
                                origin,
                            );
                            return;
                        }
                    };
                    condition = Some(Sourced::from_source(value, origin));
                }
                "RestartSec" => {
                    if delay.is_some() {
                        self.invalid_model(
                            &subject,
                            filename,
                            "systemd RestartSec is declared more than once",
                            Some(origin),
                        );
                        return;
                    }
                    let Some(value) = canonical_restart_duration(value) else {
                        self.unsupported_value(&format!("{subject}.delay"), filename, "RestartSec", "only the finite canonical <digits>us restart delay emitted by BoxFerry can be reimported; retain other native timespans separately until QuadletLens exposes duration decoding", origin);
                        return;
                    };
                    delay = Some(Sourced::from_source(value, origin));
                }
                _ => unreachable!(),
            }
        }
        let Some(delay) = delay else {
            return;
        };
        let condition = condition.unwrap_or_else(|| {
            let mut condition = Sourced::generated(DeploymentRestartCondition::Never);
            if let Some(origin) = origins.first() {
                condition.add_origin(Provenance::implementation_default(origin.source_id().clone()));
            }
            condition
        });
        let mut policy = DeploymentRestartPolicy::new(condition);
        policy.set_delay(delay);
        let condition_origins = policy.condition().origins().to_vec();
        let never = matches!(policy.condition().value(), DeploymentRestartCondition::Never);
        let mut sourced = Sourced::generated(policy);
        for origin in &origins {
            sourced.add_origin(origin.clone());
        }
        service.set_deployment_restart_policy(sourced);
        self.exact_origins(format!("{subject}.delay"), origins);
        if never {
            self.exact_origins(format!("{subject}.condition"), condition_origins);
        } else if let Some(origin) = condition_origins.first() {
            self.approximate(&format!("{subject}.condition"), filename, "systemd service restart failure classification and start-rate limits differ from deployment-task managers", origin.clone());
        }
    }
}

/// Recognizes only `BoxFerry`'s emitted wire literal, without decoding systemd's timespan grammar.
fn canonical_restart_duration(text: &str) -> Option<Duration> {
    let digits = text.strip_suffix("us")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let micros = digits.parse::<u64>().ok()?;
    (micros != u64::MAX).then(|| Duration::from_micros(micros))
}
