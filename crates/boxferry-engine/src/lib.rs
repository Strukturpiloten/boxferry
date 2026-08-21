//! Conversion planning, policy, capability, and diagnostics for `BoxFerry`.

mod adapter;
mod diagnostic;
mod outcome;
mod rule;
mod target;

pub use adapter::{
    ConversionError, ExportAdapter, ImportAdapter, ImportResult, InMemoryAdapter, convert, convert_imported,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticField, DiagnosticValue, InvalidDiagnosticCode, NativeFinding,
    NativeFindingLabel, NativeFindingLabelKind, Severity,
};
pub use outcome::{ConversionKind, ConversionOutcome, ConversionPlan, ConversionResult, LossPolicy, PlanError};
pub use rule::{DiagnosticRule, RULES, RuleId, find_rule};
pub use target::{ParsePlatformVersionError, PlatformVersion, TargetProfile, TargetProfileError, VersionRange};

#[cfg(test)]
mod integration_tests {
    use boxferry_model::{Application, Identifier};

    use crate::{
        ConversionKind, ConversionOutcome, ConversionPlan, Diagnostic, DiagnosticCode, ExportAdapter, ImportAdapter,
        ImportResult, InMemoryAdapter, LossPolicy, PlanError, PlatformVersion, Severity, TargetProfile, convert,
        convert_imported,
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

    struct ApplicationNameExporter;

    impl ExportAdapter for ApplicationNameExporter {
        type Output = String;

        fn plan(
            &self,
            application: &Application,
            _target: &TargetProfile,
        ) -> Result<ConversionPlan<Self::Output>, PlanError> {
            ConversionPlan::new(
                Some(application.name().as_str().to_owned()),
                vec![ConversionOutcome::exact("export.sentinel")],
                Vec::new(),
            )
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

    #[test]
    fn imported_boundary_passes_the_neutral_application_and_combines_outcomes() -> Result<(), String> {
        let application = Application::new(Identifier::new("neutral-sentinel").map_err(|error| error.to_string())?);
        let imported = ImportResult::new(
            Some(application),
            vec![ConversionOutcome::exact("import.sentinel")],
            Vec::new(),
        );
        let target =
            TargetProfile::new("target", PlatformVersion::new(1, 0, 0), None).map_err(|error| error.to_string())?;

        let result = convert_imported(imported, &ApplicationNameExporter, &target, LossPolicy::ExactOnly)
            .map_err(|error| error.to_string())?;

        assert_eq!(result.output().map(String::as_str), Some("neutral-sentinel"));
        assert_eq!(
            result
                .outcomes()
                .iter()
                .map(ConversionOutcome::subject)
                .collect::<Vec<_>>(),
            ["import.sentinel", "export.sentinel"]
        );
        Ok(())
    }
}
