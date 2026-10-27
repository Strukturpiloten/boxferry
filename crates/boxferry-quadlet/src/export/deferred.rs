//! Native-owned expansion-safe rendering and external prerequisite reporting.
use super::Mapping;
use boxferry_engine::{Diagnostic, DiagnosticField, DiagnosticValue, RuleId, Severity};
use boxferry_model::{DeferredExpression, DeferredSegment, Mount, Provenance, Service, Sourced};
use quadlet_lens::render::{
    ContainerEnvironmentName, DeferredWord, DeferredWordPart, QuadletDocumentBuilder, ServiceEnvironmentAssignment,
    ServiceEnvironmentFile, ServiceEnvironmentName,
};

fn word(expression: &DeferredExpression) -> Result<DeferredWord, quadlet_lens::render::DeferredValueError> {
    DeferredWord::new(
        expression
            .segments()
            .iter()
            .map(|segment| match segment.value() {
                DeferredSegment::Literal(value) => Ok(DeferredWordPart::Literal(value.expose().to_owned())),
                DeferredSegment::Reference(name) => {
                    ServiceEnvironmentName::new(name.as_str()).map(DeferredWordPart::Reference)
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
    )
}

impl Mapping<'_> {
    pub(super) fn map_execution_environment(
        &mut self,
        service_subject: &str,
        service: &Service,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let Some(plan) = service.execution_environment() else {
            return;
        };
        let subject = format!("{service_subject}.execution_environment");
        if !self.capability("systemd.section.pass-through", &subject, plan.origins()) {
            return;
        }
        if !plan.value().assignments().is_empty() && !self.exporter.deferred_environment_values {
            self.unsupported(
                &subject,
                "inline service providers require explicit environment-values include authorization",
                plan.origins(),
            );
            return;
        }
        for (index, assignment) in plan.value().assignments().iter().enumerate() {
            let native = ServiceEnvironmentName::new(assignment.value().name().as_str())
                .and_then(|name| ServiceEnvironmentAssignment::new(name, assignment.value().value().expose()));
            let Ok(native) = native else {
                self.unsupported(&subject, "invalid service environment assignment", assignment.origins());
                continue;
            };
            match builder.push_service_environment_literal(native) {
                Ok(()) => self.exact(format!("{subject}.inline[{index}]"), assignment.origins()),
                Err(error) => self.generation_error(&subject, &error, assignment.origins()),
            }
        }
        for (index, file) in plan.value().files().iter().enumerate() {
            let Ok(native) = ServiceEnvironmentFile::new(file.value().path()) else {
                self.unsupported(&subject, "invalid mandatory service environment file", file.origins());
                continue;
            };
            match builder.push_service_environment_file(native) {
                Ok(()) => self.exact(format!("{subject}.files[{index}]"), file.origins()),
                Err(error) => self.generation_error(&subject, &error, file.origins()),
            }
        }
        self.exact(&subject, plan.origins());
        self.report_external_requirements(&subject, service);
    }

    fn report_external_requirements(&mut self, subject: &str, service: &Service) {
        let rule = RuleId::QuadletExternalEnvironmentUnverified.definition();
        let Ok(code) = rule.diagnostic_code() else {
            return;
        };
        let mut diagnostic = Diagnostic::new(code, Severity::Note, rule.description())
            .with_field(DiagnosticField::new("subject", DiagnosticValue::plain(subject)))
            .with_field(DiagnosticField::new(
                "verification",
                DiagnosticValue::plain("external-unverified"),
            ));
        if service
            .execution_environment()
            .is_some_and(|plan| !plan.value().files().is_empty())
        {
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "provider_files",
                DiagnosticValue::plain("mandatory external files; contents and availability unverified"),
            ));
        }
        if service
            .environment()
            .iter()
            .any(|item| matches!(item.value().value(), boxferry_model::EnvironmentValue::Deferred(_)))
        {
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "container_environment",
                DiagnosticValue::plain("missing/unset/empty references contribute empty strings"),
            ));
        }
        if service
            .mounts()
            .iter()
            .any(|item| matches!(item.value().source(), boxferry_model::MountSource::DeferredHostPath(_)))
        {
            diagnostic = diagnostic.with_field(DiagnosticField::new(
                "host_bind",
                DiagnosticValue::plain(
                    "effective sources must be nonempty absolute paths without colon, NUL or controls",
                ),
            ));
        }
        self.diagnostics.push(diagnostic);
    }

    pub(super) fn map_deferred_environment(
        &mut self,
        subject: &str,
        name: &str,
        expression: &DeferredExpression,
        origins: &[Provenance],
        builder: &mut QuadletDocumentBuilder,
    ) {
        if expression.has_nonempty_literal() && !self.exporter.deferred_environment_values {
            self.unsupported(subject, "literal-bearing deferred container environment requires explicit environment-values include authorization", origins);
            return;
        }
        let Ok(word) = word(expression) else {
            self.unsupported(
                subject,
                "native renderer rejected deferred environment expression",
                origins,
            );
            return;
        };
        if !self.capability("quadlet.container.environment", subject, origins) {
            return;
        }
        let Ok(name) = ContainerEnvironmentName::new(name) else {
            self.unsupported(subject, "invalid literal container environment name", origins);
            return;
        };
        match builder.push_container_environment_deferred(name, word) {
            Ok(()) => self.exact(subject, origins),
            Err(error) => self.generation_error(subject, &error, origins),
        }
    }

    pub(super) fn map_literal_environment(
        &mut self,
        subject: &str,
        name: &str,
        value: &str,
        origins: &[Provenance],
        builder: &mut QuadletDocumentBuilder,
    ) {
        if ContainerEnvironmentName::new(name).is_err() {
            self.unsupported(subject, "invalid literal container environment name", origins);
            return;
        }
        if !value.contains('$') && !value.contains('%') && !name.contains('$') && !name.contains('%') {
            let Some(encoded) = super::encode_quadlet_environment_assignment(name, value) else {
                self.unsupported(subject, "invalid literal environment assignment", origins);
                return;
            };
            if self.capability("quadlet.container.environment", subject, origins)
                && self.push_container(
                    builder,
                    quadlet_lens::model::ContainerKey::Environment,
                    encoded,
                    subject,
                    origins,
                )
            {
                self.exact(subject, origins);
            }
            return;
        }
        let Ok(word) = DeferredWord::new([DeferredWordPart::Literal(value.to_owned())]) else {
            self.unsupported(subject, "native renderer rejected literal environment value", origins);
            return;
        };
        if !self.capability("quadlet.container.environment", subject, origins) {
            return;
        }
        let Ok(name) = ContainerEnvironmentName::new(name) else {
            self.unsupported(subject, "invalid literal container environment name", origins);
            return;
        };
        match builder.push_container_environment_deferred(name, word) {
            Ok(()) => self.exact(subject, origins),
            Err(error) => self.generation_error(subject, &error, origins),
        }
    }

    pub(super) fn map_literal_bind(
        &mut self,
        subject: &str,
        source: &str,
        mount: &Sourced<Mount>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        let Ok(word) = DeferredWord::new([DeferredWordPart::Literal(source.to_owned())]) else {
            self.unsupported(subject, "invalid literal bind source", mount.origins());
            return;
        };
        self.render_bind(subject, word, mount, builder);
    }

    pub(super) fn map_deferred_bind(
        &mut self,
        subject: &str,
        expression: &DeferredExpression,
        mount: &Sourced<Mount>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if let Err(error) = expression.validate_host_bind() {
            self.unsupported(subject, &error.to_string(), mount.origins());
            return;
        }
        let Ok(word) = word(expression) else {
            self.unsupported(subject, "native renderer rejected deferred bind", mount.origins());
            return;
        };
        self.render_bind(subject, word, mount, builder);
    }

    fn render_bind(
        &mut self,
        subject: &str,
        word: DeferredWord,
        mount: &Sourced<Mount>,
        builder: &mut QuadletDocumentBuilder,
    ) {
        if !self.capability("quadlet.container.volume", subject, mount.origins()) {
            return;
        }
        let mut options = Vec::new();
        if mount.value().read_only() {
            options.push("ro".to_owned());
        }
        if let Some(relabel) = mount.value().selinux_relabel() {
            options.push(
                match relabel {
                    boxferry_model::SelinuxRelabel::Shared => "z",
                    boxferry_model::SelinuxRelabel::Private => "Z",
                    _ => {
                        self.unsupported(subject, "unknown bind relabel mode", mount.origins());
                        return;
                    }
                }
                .to_owned(),
            );
        }
        match builder.push_container_bind_source_deferred(word, mount.value().target(), options) {
            Ok(()) => self.exact(subject, mount.origins()),
            Err(error) => self.unsupported(
                subject,
                &format!(
                    "native renderer cannot safely preserve this bind: {error}; use an explicit target-path mapping"
                ),
                mount.origins(),
            ),
        }
    }
}
