//! Native adapter contracts and shared public orchestration.

use std::{error::Error, fmt};

use boxferry_model::Application;

use crate::{ConversionOutcome, ConversionPlan, ConversionResult, Diagnostic, LossPolicy, PlanError, TargetProfile};

/// Recoverable result of importing a native source model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportResult {
    application: Option<Application>,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl ImportResult {
    /// Creates an import result with an optional neutral application and source-mapping decisions.
    #[must_use]
    pub const fn new(
        application: Option<Application>,
        outcomes: Vec<ConversionOutcome>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        Self {
            application,
            outcomes,
            diagnostics,
        }
    }

    /// Creates a successful import without diagnostics.
    #[must_use]
    pub const fn success(application: Application) -> Self {
        Self::new(Some(application), Vec::new(), Vec::new())
    }

    /// Returns the recoverable application, when available.
    #[must_use]
    pub const fn application(&self) -> Option<&Application> {
        self.application.as_ref()
    }

    /// Returns source-to-neutral-model fidelity decisions.
    #[must_use]
    pub fn outcomes(&self) -> &[ConversionOutcome] {
        &self.outcomes
    }

    /// Returns import diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Decomposes the import result.
    #[must_use]
    pub fn into_parts(self) -> (Option<Application>, Vec<ConversionOutcome>, Vec<Diagnostic>) {
        (self.application, self.outcomes, self.diagnostics)
    }
}

/// Maps one native source model into the format-independent application model.
pub trait ImportAdapter {
    /// Native source model accepted by this adapter.
    type Source: ?Sized;

    /// Imports one source without reading ambient process state.
    fn import(&self, source: &Self::Source) -> ImportResult;
}

/// Plans one neutral application for a native target model.
pub trait ExportAdapter {
    /// Native target candidate returned by this adapter.
    type Output;

    /// Builds a validated candidate plan for the explicit target profile.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when the adapter violates plan invariants.
    fn plan(
        &self,
        application: &Application,
        target: &TargetProfile,
    ) -> Result<ConversionPlan<Self::Output>, PlanError>;
}

/// Failure before a policy-authorized conversion result can be created.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConversionError {
    /// Import returned no neutral application or an error diagnostic.
    Import(Vec<Diagnostic>),
    /// A target adapter returned a structurally invalid plan.
    InvalidPlan(PlanError),
}

impl fmt::Display for ConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Import(diagnostics) => {
                write!(
                    formatter,
                    "source import failed with {} diagnostic(s)",
                    diagnostics.len()
                )
            }
            Self::InvalidPlan(error) => write!(formatter, "target adapter returned an invalid plan: {error}"),
        }
    }
}

impl Error for ConversionError {}

/// Runs the same explicit import-plan-authorize path used by the `BoxFerry` CLI.
///
/// # Errors
///
/// Returns [`ConversionError::Import`] when import has no usable application or
/// contains an error diagnostic, and [`ConversionError::InvalidPlan`] when the
/// export adapter violates plan invariants.
pub fn convert<I, E>(
    importer: &I,
    source: &I::Source,
    exporter: &E,
    target: &TargetProfile,
    policy: LossPolicy,
) -> Result<ConversionResult<E::Output>, ConversionError>
where
    I: ImportAdapter,
    E: ExportAdapter,
{
    convert_imported(importer.import(source), exporter, target, policy)
}

/// Continues conversion from one completed native-to-neutral import.
///
/// This boundary lets orchestration select importers and exporters as independent
/// dimensions while retaining the same validation, combined fidelity plan, and
/// policy authorization used by convert.
///
/// # Errors
///
/// Returns [`ConversionError::Import`] when import has no usable application or
/// contains an error diagnostic, and [`ConversionError::InvalidPlan`] when the
/// export adapter violates plan invariants.
pub fn convert_imported<E>(
    import: ImportResult,
    exporter: &E,
    target: &TargetProfile,
    policy: LossPolicy,
) -> Result<ConversionResult<E::Output>, ConversionError>
where
    E: ExportAdapter,
{
    let (application, import_outcomes, import_diagnostics) = import.into_parts();
    if application.is_none()
        || import_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == crate::Severity::Error)
    {
        return Err(ConversionError::Import(import_diagnostics));
    }
    let Some(application) = application else {
        return Err(ConversionError::Import(import_diagnostics));
    };
    let mut plan = exporter
        .plan(&application, target)
        .map_err(ConversionError::InvalidPlan)?;
    plan.extend_import(import_outcomes, import_diagnostics)
        .map_err(ConversionError::InvalidPlan)?;
    Ok(plan.authorize(policy))
}

/// Deterministic adapter for public API tests and embedding examples.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InMemoryAdapter<T> {
    output: T,
    outcomes: Vec<ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl<T> InMemoryAdapter<T> {
    /// Creates an adapter that reports one exact application outcome.
    #[must_use]
    pub fn exact(output: T) -> Self {
        Self {
            output,
            outcomes: vec![ConversionOutcome::exact("application")],
            diagnostics: Vec::new(),
        }
    }

    /// Creates an adapter with caller-selected validated-plan inputs.
    #[must_use]
    pub const fn new(output: T, outcomes: Vec<ConversionOutcome>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            output,
            outcomes,
            diagnostics,
        }
    }
}

impl<T> ImportAdapter for InMemoryAdapter<T> {
    type Source = Application;

    fn import(&self, source: &Self::Source) -> ImportResult {
        ImportResult::success(source.clone())
    }
}

impl<T: Clone> ExportAdapter for InMemoryAdapter<T> {
    type Output = T;

    fn plan(
        &self,
        _application: &Application,
        _target: &TargetProfile,
    ) -> Result<ConversionPlan<Self::Output>, PlanError> {
        ConversionPlan::new(
            Some(self.output.clone()),
            self.outcomes.clone(),
            self.diagnostics.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use boxferry_model::{Application, Identifier};

    use super::{InMemoryAdapter, convert};
    use crate::{LossPolicy, PlatformVersion, TargetProfile};

    #[test]
    fn in_memory_adapter_proves_the_public_orchestration_path() -> Result<(), String> {
        let application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
        let adapter = InMemoryAdapter::exact("rendered target".to_owned());
        let target = TargetProfile::new("test-target", PlatformVersion::new(1, 0, 0), None)
            .map_err(|error| error.to_string())?;

        let result = convert(&adapter, &application, &adapter, &target, LossPolicy::ExactOnly)
            .map_err(|error| error.to_string())?;
        assert_eq!(result.output().map(String::as_str), Some("rendered target"));
        assert!(!result.is_blocked());
        Ok(())
    }
}
