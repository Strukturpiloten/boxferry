//! Fidelity outcomes, loss authorization, and conversion plans.

use std::{error::Error, fmt};

use boxferry_model::Provenance;

use crate::{Diagnostic, DiagnosticCode, Severity};

/// Fidelity of one source-to-target decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConversionKind {
    /// Target behavior represents the source intent exactly within known evidence.
    Exact,
    /// Target behavior requires a documented adjustment.
    Approximate,
    /// Target cannot represent the source intent.
    Unsupported,
    /// Source intent or target configuration is invalid.
    Invalid,
}

/// One subject-level conversion decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionOutcome {
    subject: String,
    kind: ConversionKind,
    diagnostic: Option<DiagnosticCode>,
    origins: Vec<Provenance>,
}

impl ConversionOutcome {
    /// Creates an exact outcome with no loss diagnostic.
    #[must_use]
    pub fn exact(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            kind: ConversionKind::Exact,
            diagnostic: None,
            origins: Vec::new(),
        }
    }

    /// Creates a non-exact outcome linked to a structured diagnostic.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::ExactOutcomeHasDiagnostic`] if `kind` is exact.
    pub fn loss(
        subject: impl Into<String>,
        kind: ConversionKind,
        diagnostic: DiagnosticCode,
    ) -> Result<Self, PlanError> {
        if kind == ConversionKind::Exact {
            return Err(PlanError::ExactOutcomeHasDiagnostic);
        }
        Ok(Self {
            subject: subject.into(),
            kind,
            diagnostic: Some(diagnostic),
            origins: Vec::new(),
        })
    }

    /// Adds a source origin that contributed to this decision.
    #[must_use]
    pub fn with_origin(mut self, origin: Provenance) -> Self {
        self.origins.push(origin);
        self
    }

    /// Returns the stable subject path.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Returns the decision fidelity.
    #[must_use]
    pub const fn kind(&self) -> ConversionKind {
        self.kind
    }

    /// Returns the required diagnostic code for a non-exact decision.
    #[must_use]
    pub const fn diagnostic(&self) -> Option<&DiagnosticCode> {
        self.diagnostic.as_ref()
    }

    /// Returns contributing source origins in discovery order.
    #[must_use]
    pub fn origins(&self) -> &[Provenance] {
        &self.origins
    }
}

/// Caller-selected authorization for non-exact candidate output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LossPolicy {
    /// Emit output only when every decision is exact.
    ExactOnly,
    /// Permit documented approximate mappings but not unsupported intent.
    AllowApproximate,
    /// Permit partial output with diagnostics for unsupported intent.
    AllowPartial,
}

impl LossPolicy {
    const fn permits(self, kind: ConversionKind) -> bool {
        match kind {
            ConversionKind::Exact => true,
            ConversionKind::Approximate => !matches!(self, Self::ExactOnly),
            ConversionKind::Unsupported => matches!(self, Self::AllowPartial),
            ConversionKind::Invalid => false,
        }
    }
}

/// Invalid conversion plan invariant.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PlanError {
    /// An exact outcome was incorrectly linked to a loss diagnostic.
    ExactOutcomeHasDiagnostic,
    /// A non-exact outcome referenced a diagnostic missing from the plan.
    MissingDiagnostic {
        /// Outcome subject.
        subject: String,
        /// Referenced code.
        code: DiagnosticCode,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExactOutcomeHasDiagnostic => {
                formatter.write_str("exact conversion outcomes must not carry a loss diagnostic")
            }
            Self::MissingDiagnostic { subject, code } => write!(
                formatter,
                "conversion outcome `{subject}` references missing diagnostic {}",
                code.as_str()
            ),
        }
    }
}

impl Error for PlanError {}

/// Validated target candidate, decisions, and structured diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionPlan<T> {
    candidate: Option<T>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<T> ConversionPlan<T> {
    /// Creates a plan and verifies that every loss has its referenced diagnostic.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::MissingDiagnostic`] when a non-exact outcome has no
    /// matching structured diagnostic.
    pub fn new(
        candidate: Option<T>,
        outcomes: Vec<ConversionOutcome>,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Self, PlanError> {
        for outcome in &outcomes {
            if let Some(code) = outcome.diagnostic() {
                if !diagnostics.iter().any(|diagnostic| diagnostic.code() == code) {
                    return Err(PlanError::MissingDiagnostic {
                        subject: outcome.subject().to_owned(),
                        code: code.clone(),
                    });
                }
            }
        }
        Ok(Self {
            candidate,
            outcomes,
            diagnostics,
        })
    }

    /// Returns the unapproved target candidate.
    #[must_use]
    pub const fn candidate(&self) -> Option<&T> {
        self.candidate.as_ref()
    }

    /// Returns all subject outcomes.
    #[must_use]
    pub fn outcomes(&self) -> &[ConversionOutcome] {
        &self.outcomes
    }

    /// Returns all diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn extend_import(
        &mut self,
        outcomes: Vec<ConversionOutcome>,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<(), PlanError> {
        self.diagnostics.extend(diagnostics);
        for outcome in &outcomes {
            if let Some(code) = outcome.diagnostic() {
                if !self.diagnostics.iter().any(|diagnostic| diagnostic.code() == code) {
                    return Err(PlanError::MissingDiagnostic {
                        subject: outcome.subject().to_owned(),
                        code: code.clone(),
                    });
                }
            }
        }
        self.outcomes.splice(0..0, outcomes);
        Ok(())
    }

    /// Applies a caller-selected loss policy without changing the candidate bytes.
    #[must_use]
    pub fn authorize(self, policy: LossPolicy) -> ConversionResult<T> {
        let blocked = self.candidate.is_none()
            || self.outcomes.iter().any(|outcome| !policy.permits(outcome.kind()))
            || self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity() == Severity::Error);
        ConversionResult {
            output: if blocked { None } else { self.candidate },
            candidate_blocked: blocked,
            outcomes: self.outcomes,
            diagnostics: self.diagnostics,
        }
    }
}

/// Policy-authorized conversion output and its complete report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversionResult<T> {
    output: Option<T>,
    candidate_blocked: bool,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<T> ConversionResult<T> {
    /// Returns output only when authorized by the selected loss policy.
    #[must_use]
    pub const fn output(&self) -> Option<&T> {
        self.output.as_ref()
    }

    /// Returns whether output was unavailable because the candidate was missing,
    /// forbidden by policy, or accompanied by an error diagnostic.
    #[must_use]
    pub const fn is_blocked(&self) -> bool {
        self.candidate_blocked
    }

    /// Returns all subject outcomes.
    #[must_use]
    pub fn outcomes(&self) -> &[ConversionOutcome] {
        &self.outcomes
    }

    /// Returns all diagnostics, including import diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Decomposes the authorized result.
    #[must_use]
    pub fn into_parts(self) -> (Option<T>, Vec<ConversionOutcome>, Vec<Diagnostic>) {
        (self.output, self.outcomes, self.diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use boxferry_model::{Provenance, SourceId};

    use super::{ConversionKind, ConversionOutcome, ConversionPlan, LossPolicy, PlanError};
    use crate::{Diagnostic, DiagnosticCode, Severity};

    #[test]
    fn strict_and_partial_policies_treat_unsupported_output_differently() -> Result<(), String> {
        let diagnostic = Diagnostic::new(code("BFE0002")?, Severity::Warning, "feature omitted");
        let outcome = ConversionOutcome::loss(
            "services.web.unsupported",
            ConversionKind::Unsupported,
            diagnostic.code().clone(),
        )
        .map_err(|error| error.to_string())?;

        let strict = ConversionPlan::new(Some("candidate"), vec![outcome.clone()], vec![diagnostic.clone()])
            .map_err(|error| error.to_string())?
            .authorize(LossPolicy::ExactOnly);
        assert!(strict.is_blocked());
        assert_eq!(strict.output(), None);

        let partial = ConversionPlan::new(Some("candidate"), vec![outcome], vec![diagnostic])
            .map_err(|error| error.to_string())?
            .authorize(LossPolicy::AllowPartial);
        assert!(!partial.is_blocked());
        assert_eq!(partial.output(), Some(&"candidate"));
        Ok(())
    }

    #[test]
    fn every_loss_must_reference_a_present_diagnostic() -> Result<(), String> {
        let outcome = ConversionOutcome::loss("services.web.command", ConversionKind::Approximate, code("BFE0003")?)
            .map_err(|error| error.to_string())?;
        assert!(matches!(
            ConversionPlan::<()>::new(None, vec![outcome], Vec::new()),
            Err(PlanError::MissingDiagnostic { .. })
        ));
        Ok(())
    }

    #[test]
    fn missing_and_invalid_candidates_are_always_blocked() -> Result<(), String> {
        let missing = ConversionPlan::<String>::new(None, Vec::new(), Vec::new())
            .map_err(|error| error.to_string())?
            .authorize(LossPolicy::AllowPartial);
        assert!(missing.is_blocked());

        let diagnostic = Diagnostic::new(code("BFE0004")?, Severity::Error, "source value is invalid");
        let invalid = ConversionOutcome::loss("services.web.port", ConversionKind::Invalid, diagnostic.code().clone())
            .map_err(|error| error.to_string())?;
        let result = ConversionPlan::new(Some("candidate"), vec![invalid], vec![diagnostic])
            .map_err(|error| error.to_string())?
            .authorize(LossPolicy::AllowPartial);
        assert!(result.is_blocked());
        assert_eq!(result.output(), None);
        Ok(())
    }

    #[test]
    fn decisions_retain_source_provenance() -> Result<(), String> {
        let origin = Provenance::source(SourceId::new("compose.yaml").map_err(|error| error.to_string())?);
        let outcome = ConversionOutcome::exact("services.web.image").with_origin(origin.clone());
        assert_eq!(outcome.origins(), [origin]);
        Ok(())
    }

    fn code(value: &str) -> Result<DiagnosticCode, String> {
        DiagnosticCode::new(value).map_err(|error| error.to_string())
    }
}
