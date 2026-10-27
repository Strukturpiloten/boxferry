//! Value-free losses for target formats without an inert metadata channel.
use crate::{
    ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, PlanError,
    Severity,
};
use boxferry_model::Application;
/// Accounts for every inert metadata item when the target has no reviewed transport.
///
/// # Errors
/// Returns a planning error if an unsupported decision cannot be constructed.
pub fn vendor_metadata_losses(
    application: &Application,
    code: &DiagnosticCode,
) -> Result<(Vec<ConversionOutcome>, Vec<Diagnostic>), PlanError> {
    let mut outcomes = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, metadata) in application.vendor_metadata().iter().enumerate() {
        let subject = format!("application.vendor_metadata[{index}]");
        let mut outcome = ConversionOutcome::loss(&subject, ConversionKind::Unsupported, code.clone())?;
        for origin in metadata
            .namespace()
            .origins()
            .iter()
            .chain(metadata.payload().origins())
        {
            outcome = outcome.with_origin(origin.clone());
        }
        outcomes.push(outcome);
        diagnostics.push(Diagnostic::new(code.clone(), Severity::Warning, "target cannot retain inert vendor metadata")
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
            .with_field(DiagnosticField::new("reason", DiagnosticValue::plain("retain the original metadata document separately; this target has no reviewed inert metadata transport"))));
    }
    Ok((outcomes, diagnostics))
}
