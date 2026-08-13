//! Conversion planning, policy, capability, and diagnostics for `BoxFerry`.

mod adapter;
mod diagnostic;
mod outcome;
mod rule;
mod target;

pub use adapter::{ConversionError, ExportAdapter, ImportAdapter, ImportResult, InMemoryAdapter, convert};
pub use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, InvalidDiagnosticCode, Severity};
pub use outcome::{ConversionKind, ConversionOutcome, ConversionPlan, ConversionResult, LossPolicy, PlanError};
pub use rule::{DiagnosticRule, RULES, RuleId, find_rule};
pub use target::{ParsePlatformVersionError, PlatformVersion, TargetProfile, TargetProfileError, VersionRange};

#[cfg(test)]
mod integration_tests {
    use boxferry_model::{Application, Identifier};

    use crate::{
        ConversionKind, ConversionOutcome, Diagnostic, DiagnosticCode, ImportAdapter, ImportResult, InMemoryAdapter,
        LossPolicy, PlatformVersion, Severity, TargetProfile, convert,
    };

    struct LossyImporter {
        application: Application,
    }

    impl ImportAdapter for LossyImporter {
        type Source = ();

        fn import(&self, _source: &Self::Source) -> ImportResult {
            let code = DiagnosticCode::new("BFE0010").ok();
            let (outcomes, diagnostics) = code.map_or_else(
                || (Vec::new(), Vec::new()),
                |code| {
                    let outcome =
                        ConversionOutcome::loss("services.web.extension", ConversionKind::Unsupported, code.clone())
                            .ok();
                    let outcomes = outcome.into_iter().collect();
                    let diagnostics = vec![Diagnostic::new(code, Severity::Warning, "extension omitted")];
                    (outcomes, diagnostics)
                },
            );
            ImportResult::new(Some(self.application.clone()), outcomes, diagnostics)
        }
    }

    #[test]
    fn import_losses_participate_in_output_authorization() -> Result<(), String> {
        let application = Application::new(Identifier::new("example").map_err(|error| error.to_string())?);
        let importer = LossyImporter { application };
        let exporter = InMemoryAdapter::exact("candidate".to_owned());
        let target =
            TargetProfile::new("target", PlatformVersion::new(1, 0, 0), None).map_err(|error| error.to_string())?;

        let strict =
            convert(&importer, &(), &exporter, &target, LossPolicy::ExactOnly).map_err(|error| error.to_string())?;
        assert!(strict.is_blocked());

        let partial =
            convert(&importer, &(), &exporter, &target, LossPolicy::AllowPartial).map_err(|error| error.to_string())?;
        assert_eq!(partial.output().map(String::as_str), Some("candidate"));
        assert!(partial.outcomes().iter().any(|outcome| {
            outcome.kind() == ConversionKind::Unsupported && outcome.subject() == "services.web.extension"
        }));
        Ok(())
    }
}
