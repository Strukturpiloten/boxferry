//! Deployment restart mapping, independent of container-runtime restart intent.
use std::time::Duration;

use boxferry_model::{
    DeploymentRestartCondition, DeploymentRestartDurationField, DeploymentRestartPolicy, Provenance, Service, Sourced,
};
use compose_lens::{
    model::{DeployRestartCondition, DeployRestartDuration, DeployRestartMaxAttempts},
    project::{ProjectDeployRestartPolicy, ProjectService, ProjectValue},
};

use super::Mapping;

impl Mapping<'_> {
    pub(super) fn map_deployment_restart_policy(
        &mut self,
        subject: &str,
        native: &ProjectService,
        service: &mut Service,
    ) {
        let Some(deploy) = native.deploy() else {
            return;
        };
        let deploy_subject = format!("{subject}.deploy");
        let value = deploy.value();
        if value.restart_policy().is_none() {
            self.report_typed_service_value(subject, "deploy", native.deploy());
            return;
        }
        macro_rules! unsupported {
            ($field:ident, $name:literal) => {
                self.report_typed_service_value(&deploy_subject, $name, value.$field());
            };
        }
        unsupported!(endpoint_mode, "endpoint_mode");
        unsupported!(labels, "labels");
        unsupported!(mode, "mode");
        unsupported!(placement, "placement");
        unsupported!(replicas, "replicas");
        unsupported!(resources, "resources");
        unsupported!(rollback_config, "rollback_config");
        unsupported!(update_config, "update_config");
        self.report_project_fields(&deploy_subject, "deployment field", value.unmodeled_fields());
        let Some(native_policy) = value.restart_policy() else {
            return;
        };
        let subject = format!("{deploy_subject}.restart_policy");
        let policy = native_policy.value();
        self.report_project_fields(&subject, "deployment restart field", policy.unmodeled_fields());
        let Some(condition) = self.deployment_restart_condition(&subject, native_policy) else {
            return;
        };
        let mut mapped = DeploymentRestartPolicy::new(condition);
        if let Some(delay) = policy.delay()
            && !self.map_deployment_duration(&subject, delay, DeploymentRestartDurationField::Delay, &mut mapped)
        {
            return;
        }
        if let Some(attempts) = policy.max_attempts() {
            let (DeployRestartMaxAttempts::YamlNumber(raw) | DeployRestartMaxAttempts::String(raw)) = attempts.value()
            else {
                self.invalid_value_optional(
                    &format!("{subject}.max_attempts"),
                    "unrecognized deployment restart attempt value",
                    attempts.effective_source(),
                );
                return;
            };
            let Ok(value) = raw.parse::<u64>() else {
                self.invalid_value_optional(
                    &format!("{subject}.max_attempts"),
                    "deployment restart attempts must be a nonnegative unsigned 64-bit integer",
                    attempts.effective_source(),
                );
                return;
            };
            mapped.set_maximum_attempts(self.sourced_provenance(value, attempts.provenance()));
            self.exact_provenance(format!("{subject}.max_attempts"), attempts.provenance());
        }
        if let Some(window) = policy.window()
            && !self.map_deployment_duration(&subject, window, DeploymentRestartDurationField::Window, &mut mapped)
        {
            return;
        }
        service.set_deployment_restart_policy(self.sourced_provenance(mapped, native_policy.provenance()));
        self.exact_provenance(subject, native_policy.provenance());
    }

    fn deployment_restart_condition(
        &mut self,
        subject: &str,
        native_policy: &ProjectValue<ProjectDeployRestartPolicy>,
    ) -> Option<Sourced<DeploymentRestartCondition>> {
        let condition = if let Some(condition) = native_policy.value().condition() {
            let value = match condition.value() {
                DeployRestartCondition::None => DeploymentRestartCondition::Never,
                DeployRestartCondition::OnFailure => DeploymentRestartCondition::OnFailure,
                DeployRestartCondition::Any => DeploymentRestartCondition::Any,
                _ => {
                    self.invalid_value_optional(
                        &format!("{subject}.condition"),
                        "deployment restart condition must be a resolved none, on-failure, or any",
                        condition.effective_source(),
                    );
                    return None;
                }
            };
            self.exact_provenance(format!("{subject}.condition"), condition.provenance());
            self.sourced_provenance(value, condition.provenance())
        } else {
            let mut condition = self.sourced_provenance(DeploymentRestartCondition::Any, native_policy.provenance());
            if let Some(origin) = condition.origins().first() {
                condition.add_origin(Provenance::implementation_default(origin.source_id().clone()));
            }
            condition
        };
        Some(condition)
    }

    fn map_deployment_duration(
        &mut self,
        subject: &str,
        native: &ProjectValue<DeployRestartDuration>,
        field: DeploymentRestartDurationField,
        policy: &mut DeploymentRestartPolicy,
    ) -> bool {
        let name = match field {
            DeploymentRestartDurationField::Delay => "delay",
            DeploymentRestartDurationField::Window => "window",
            _ => return false,
        };
        let subject = format!("{subject}.{name}");
        match compose_restart_duration(native.value().raw()) {
            Ok(value) => {
                let sourced = self.sourced_provenance(value, native.provenance());
                match field {
                    DeploymentRestartDurationField::Delay => policy.set_delay(sourced),
                    DeploymentRestartDurationField::Window => policy.set_window(sourced),
                    _ => return false,
                }
                self.exact_provenance(subject, native.provenance());
            }
            Err(DurationMappingError::Unrepresentable) => {
                policy.add_unrepresented_duration(self.sourced_provenance(field, native.provenance()));
                self.unsupported_optional(&subject, "deployment restart duration exceeds the neutral precision or range; choose an exactly representable duration", native.effective_source());
            }
            Err(DurationMappingError::Invalid) => {
                self.invalid_value_optional(
                    &subject,
                    "deployment restart duration must be a resolved nonnegative Compose duration",
                    native.effective_source(),
                );
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug)]
enum DurationMappingError {
    Invalid,
    Unrepresentable,
}

/// Normalizes the raw value exposed by `ComposeLens`'s typed `DeployRestartDuration`.
/// Grammar recognition precedes checked arithmetic so precision and range losses are Unsupported.
fn compose_restart_duration(mut text: &str) -> Result<Duration, DurationMappingError> {
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    if text.is_empty() {
        return Err(DurationMappingError::Invalid);
    }
    let mut parts = Vec::new();
    while !text.is_empty() {
        let number_end = text
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(text.len());
        let number = &text[..number_end];
        let (integer, fraction) = number.split_once('.').unwrap_or((number, ""));
        if (integer.is_empty() && fraction.is_empty())
            || !integer.bytes().chain(fraction.bytes()).all(|c| c.is_ascii_digit())
        {
            return Err(DurationMappingError::Invalid);
        }
        text = &text[number_end..];
        let (unit, scale) = [
            ("ns", 1_u128),
            ("us", 1_000),
            ("µs", 1_000),
            ("μs", 1_000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
        ]
        .into_iter()
        .find(|(unit, _)| text.starts_with(unit))
        .ok_or(DurationMappingError::Invalid)?;
        parts.push((integer, fraction, scale));
        text = &text[unit.len()..];
    }
    normalize_restart_duration(&parts).ok_or(DurationMappingError::Unrepresentable)
}

fn normalize_restart_duration(parts: &[(&str, &str, u128)]) -> Option<Duration> {
    let mut total = 0_u128;
    for &(integer, fraction, scale) in parts {
        let denominator = 10_u128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
        let fractional = if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u128>().ok()?.checked_mul(scale)?
        };
        if !fractional.is_multiple_of(denominator) {
            return None;
        }
        let whole = if integer.is_empty() {
            0
        } else {
            integer.parse::<u128>().ok()?
        };
        total = total
            .checked_add(whole.checked_mul(scale)?)?
            .checked_add(fractional / denominator)?;
    }
    Some(Duration::new(
        u64::try_from(total / 1_000_000_000).ok()?,
        u32::try_from(total % 1_000_000_000).ok()?,
    ))
}
