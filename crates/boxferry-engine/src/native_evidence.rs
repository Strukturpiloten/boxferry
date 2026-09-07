//! Shared loss expansion for opaque source evidence retained by the neutral model.

use boxferry_model::{Application, Provenance, Sourced};

use crate::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, PlanError,
    Severity,
};

const SUMMARY: &str = "retained native evidence is not portable desired state";
const REASON: &str = "opaque source evidence is retained for explanation and is never rendered or executed";

/// Builds one consistent unsupported decision per stable retained-evidence subject.
///
/// Events remain stored on [`Application`] in authored order. This projection uses
/// only subjects and provenance from both event envelopes and their physical
/// source segments; protected native values never enter diagnostics.
///
/// # Errors
///
/// Returns [`PlanError`] if construction of an unsupported outcome fails.
pub fn retained_native_evidence_losses(
    application: &Application,
    code: &DiagnosticCode,
) -> Result<(Vec<ConversionOutcome>, Vec<Diagnostic>), PlanError> {
    let mut grouped = Vec::<(String, Vec<Provenance>)>::new();
    for evidence in application.retained_native_evidence() {
        let subject = evidence.subject().conversion_subject();
        let index = grouped
            .iter()
            .position(|(candidate, _)| candidate == &subject)
            .unwrap_or_else(|| {
                grouped.push((subject.clone(), Vec::new()));
                grouped.len() - 1
            });
        let origins = &mut grouped[index].1;
        let event = evidence.event();
        for origin in event
            .origins()
            .iter()
            .chain(event.value().physical_segments().iter().flat_map(Sourced::origins))
        {
            if !origins.contains(origin) {
                origins.push(origin.clone());
            }
        }
    }

    let mut outcomes = Vec::with_capacity(grouped.len());
    let mut diagnostics = Vec::with_capacity(grouped.len());
    for (subject, origins) in grouped {
        let diagnostic = Diagnostic::new(code.clone(), Severity::Warning, SUMMARY)
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(&subject)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain(REASON)))
            .with_field(DiagnosticField::new("decision", DiagnosticValue::plain("omitted")))
            .with_field(DiagnosticField::new(
                "required_loss_policy",
                DiagnosticValue::plain("partial"),
            ));
        let mut outcome = ConversionOutcome::loss(&subject, ConversionKind::Unsupported, code.clone())?;
        for origin in origins {
            outcome = outcome.with_origin(origin);
        }
        diagnostics.push(diagnostic);
        outcomes.push(outcome);
    }
    Ok((outcomes, diagnostics))
}
