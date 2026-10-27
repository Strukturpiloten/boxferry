//! Mapping explicitly selected direct references through the native inspection kernel.
use super::Mapping;
use boxferry_model::{
    Application, DeferredExpression, DeferredSegment, EnvironmentValue, EnvironmentVariable, Mount, MountSource,
    ProtectedString, Service, ServiceEnvironmentName,
};
use compose_lens::{
    interpolation::{
        InterpolationInput, InterpolationOperator, InterpolationSegment, ScalarInterpolationInspection,
        inspect_scalar_interpolation,
    },
    model::{MountType, VolumeMount},
    project::ProjectService,
    syntax::ScalarPresentation,
};

enum Inspected {
    Literal(String),
    Deferred(DeferredExpression),
}

impl Mapping<'_> {
    pub(super) fn validate_deferred_services(&mut self, application: &Application) {
        for name in self.source.deferred_options().services().keys() {
            if !application
                .services()
                .iter()
                .any(|service| service.value().name() == name)
            {
                self.invalid_optional_span(self.codes.invalid_value.clone(), &format!("services.{}.execution_environment", name.as_str()), "deferred plan requires a valid active service; unknown, inactive and invalid services cannot receive providers", "service execution environment", None);
            }
        }
    }

    fn inspect_deferred(
        &self,
        input: InterpolationInput<'_>,
        style: ScalarPresentation,
        prefix: Option<&str>,
    ) -> Result<Inspected, &'static str> {
        let result = inspect_scalar_interpolation(input, style).map_err(|_| "deferred expression exceeds native inspection bounds; use conversion-time --env or --env-file resolution")?;
        let inspection = match result {
            ScalarInterpolationInspection::Inspected(value) => value,
            ScalarInterpolationInspection::Excluded(value) if !value.contains_dollar() => {
                return Ok(Inspected::Literal(
                    value
                        .original()
                        .strip_prefix(prefix.unwrap_or(""))
                        .ok_or("invalid environment assignment prefix")?
                        .to_owned(),
                ));
            }
            ScalarInterpolationInspection::Excluded(_) => {
                return Err(
                    "dollar syntax in single-quoted or block scalar is excluded from deferral; use eligible plain/double-quoted input or conversion-time resolution",
                );
            }
            _ => return Err("unsupported native inspection result"),
        };
        if !inspection.is_valid() {
            return Err("malformed deferred expression; use conversion-time --env or --env-file resolution");
        }
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut has_reference = false;
        let mut prefix = prefix;
        for segment in inspection.segments() {
            let (value, span) = match segment {
                InterpolationSegment::Literal(value) => {
                    let text = if let Some(expected) = prefix.take() {
                        value
                            .value()
                            .strip_prefix(expected)
                            .ok_or("environment assignment name must be literal")?
                    } else {
                        value.value()
                    };
                    if text.is_empty() {
                        continue;
                    }
                    literal.push_str(text);
                    (DeferredSegment::Literal(ProtectedString::sensitive(text)), value.span())
                }
                InterpolationSegment::Reference(value)
                    if prefix.is_none() && value.operator() == InterpolationOperator::Direct =>
                {
                    has_reference = true;
                    (
                        DeferredSegment::Reference(
                            ServiceEnvironmentName::new(value.name()).map_err(|_| "invalid deferred reference name")?,
                        ),
                        value.span(),
                    )
                }
                _ => {
                    return Err(
                        "only direct deferred references are supported; resolve operators, defaults, alternatives and required expressions using conversion-time --env or --env-file",
                    );
                }
            };
            parts.push(self.sourced_spans(value, &[span]));
        }
        if prefix.is_some() {
            return Err("environment assignment name must be literal");
        }
        if has_reference {
            DeferredExpression::new(parts)
                .map(Inspected::Deferred)
                .map_err(|_| "invalid bounded deferred expression")
        } else {
            Ok(Inspected::Literal(literal))
        }
    }

    fn inspect_environment_entry(
        &self,
        service: &str,
        name: &str,
        span: Option<compose_lens::source::SourceSpan>,
    ) -> Result<Inspected, &'static str> {
        let environment = self
            .source
            .project()
            .root()
            .get("services")
            .and_then(|value| value.get(service))
            .and_then(|value| value.get("environment"))
            .ok_or("missing native environment evidence")?;
        if let Some(entries) = environment.as_mapping() {
            let entry = entries
                .iter()
                .find(|entry| entry.key() == name)
                .ok_or("missing native environment entry")?;
            if let (Some(input), Some(raw)) = (entry.raw_list_item_input(), entry.raw_list_item()) {
                return self.inspect_deferred(input.sensitive(), raw.presentation(), Some(&format!("{name}=")));
            }
            let scalar = entry
                .value()
                .as_scalar()
                .ok_or("deferred environment requires an explicit scalar value")?;
            let span = entry
                .value()
                .provenance()
                .effective_source()
                .ok_or("missing native source evidence")?;
            return self.inspect_deferred(
                InterpolationInput::new(scalar.value(), span).sensitive(),
                scalar.presentation(),
                None,
            );
        }
        let item = environment
            .as_sequence()
            .and_then(|items| items.iter().find(|value| value.provenance().effective_source() == span))
            .ok_or("missing native list environment evidence")?;
        let scalar = item.as_scalar().ok_or("environment list item must be scalar")?;
        self.inspect_deferred(
            InterpolationInput::new(scalar.value(), span.ok_or("missing native source evidence")?).sensitive(),
            scalar.presentation(),
            Some(&format!("{name}=")),
        )
    }

    pub(super) fn eligible_authored_environment(
        &self,
        subject: &str,
        name: &str,
        span: Option<compose_lens::source::SourceSpan>,
    ) -> bool {
        let Some(service) = subject.strip_prefix("services.") else {
            return false;
        };
        let environment = self
            .source
            .project()
            .root()
            .get("services")
            .and_then(|value| value.get(service))
            .and_then(|value| value.get("environment"));
        let style = environment.and_then(|environment| {
            if let Some(entries) = environment.as_mapping() {
                let entry = entries.iter().find(|entry| entry.key() == name)?;
                entry
                    .raw_list_item()
                    .or_else(|| entry.value().as_scalar())
                    .map(compose_lens::merge::MergedScalar::presentation)
            } else {
                environment
                    .as_sequence()?
                    .iter()
                    .find(|value| value.provenance().effective_source() == span)?
                    .as_scalar()
                    .map(compose_lens::merge::MergedScalar::presentation)
            }
        });
        matches!(
            style,
            Some(ScalarPresentation::Plain | ScalarPresentation::DoubleQuoted)
        )
    }

    pub(super) fn map_deferred_service_environment(
        &mut self,
        subject: &str,
        native: &ProjectService,
        service: &mut Service,
    ) -> bool {
        let Some(plan) = self.source.deferred_options().services().get(service.name()).cloned() else {
            return false;
        };
        service.set_execution_environment(plan);
        if let Some(environment) = native.environment() {
            for entry in environment.value().entries() {
                let field = format!("{subject}.environment.{}", entry.name().value());
                let Some(name) =
                    self.identifier_optional(&field, entry.name().value(), entry.value().effective_source())
                else {
                    continue;
                };
                match self.inspect_environment_entry(
                    service.name().as_str(),
                    entry.name().value(),
                    entry.value().effective_source(),
                ) {
                    Ok(value) => {
                        let value = match value {
                            Inspected::Literal(value) => EnvironmentValue::Literal(ProtectedString::sensitive(value)),
                            Inspected::Deferred(value) => EnvironmentValue::Deferred(value),
                        };
                        let mut mapped =
                            self.sourced_provenance(EnvironmentVariable::new(name, value), entry.value().provenance());
                        if let Some(origin) = mapped.origins().first() {
                            mapped.add_origin(boxferry_model::Provenance::conversion_decision(
                                origin.source_id().clone(),
                            ));
                        }
                        service.add_environment(mapped);
                        self.exact_provenance(&field, entry.value().provenance());
                    }
                    Err(reason) => self.unsupported_optional(&field, reason, entry.value().effective_source()),
                }
            }
        }
        if let Some(files) = native.environment_files() {
            self.map_environment_files(subject, files, service);
        }
        true
    }

    pub(super) fn map_service_mounts(&mut self, subject: &str, native: &ProjectService, service: &mut Service) {
        if let Some(volumes) = native.volumes() {
            for (index, mount) in volumes.value().iter().enumerate() {
                let field = format!("{subject}.volumes[{index}]");
                let (value, inspected) = if self.source.interpolation_applied() {
                    (self.map_mount(&field, mount.value()), true)
                } else if self.source.deferred_options().services().contains_key(service.name()) {
                    match self.map_deferred_mount(native.name().value(), index, &field, mount.value()) {
                        MountMapping::Ordinary => (self.map_mount(&field, mount.value()), false),
                        MountMapping::Inspected(value) => (value, true),
                    }
                } else {
                    (self.map_mount(&field, mount.value()), false)
                };
                if let Some(value) = value {
                    let value = if inspected { value.with_literal_paths() } else { value };
                    let mut mapped = self.sourced_provenance(value, mount.provenance());
                    if inspected && let Some(origin) = mapped.origins().first() {
                        mapped.add_origin(boxferry_model::Provenance::conversion_decision(
                            origin.source_id().clone(),
                        ));
                    }
                    service.add_mount(mapped);
                    self.exact_provenance(field, mount.provenance());
                }
            }
        }
    }

    fn mount_input<'a>(&self, service: &str, index: usize, mount: &'a VolumeMount) -> Option<MountInput<'a>> {
        let raw = self
            .source
            .project()
            .root()
            .get("services")?
            .get(service)?
            .get("volumes")?
            .as_sequence()?
            .get(index)?;
        match mount {
            VolumeMount::Short(value) => Some(MountInput {
                source: value.source().unwrap_or(""),
                target: value.target()?,
                span: value.raw().span(),
                style: raw.as_scalar()?.presentation(),
                full: Some(value.raw().value()),
                bind: true,
                dynamic_options: value.options().iter().any(|option| option.contains('$')),
            }),
            VolumeMount::Long(value) => Some(MountInput {
                source: value.source()?.value(),
                target: value.target()?.value(),
                span: value.source()?.span(),
                style: raw.get("source")?.as_scalar()?.presentation(),
                full: None,
                bind: matches!(
                    value.mount_type().map(compose_lens::model::Located::value),
                    Some(MountType::Bind)
                ),
                dynamic_options: false,
            }),
        }
    }

    fn map_deferred_mount(&mut self, service: &str, index: usize, subject: &str, mount: &VolumeMount) -> MountMapping {
        let enabled = self
            .source
            .deferred_options()
            .services()
            .keys()
            .any(|name| name.as_str() == service);
        let Some(input) = self.mount_input(service, index, mount) else {
            return MountMapping::Ordinary;
        };
        let raw = input.full.unwrap_or(input.source);
        if !raw.contains('$') && !raw.contains('%') {
            return MountMapping::Ordinary;
        }
        if let Err(reason) = self.inspect_deferred(InterpolationInput::new(raw, input.span), input.style, None) {
            if !enabled {
                return MountMapping::Ordinary;
            }
            self.unsupported(subject, reason, input.span);
            return MountMapping::Inspected(None);
        }
        if input.target.contains('$') || input.dynamic_options {
            self.unsupported(
                subject,
                "deferred bind targets/options are unsupported; use conversion-time resolution",
                input.span,
            );
            return MountMapping::Inspected(None);
        }
        if !input.bind {
            if !enabled {
                return MountMapping::Ordinary;
            }
            self.unsupported(
                subject,
                "only explicit host binds support deferred sources; named volumes need conversion-time resolution",
                input.span,
            );
            return MountMapping::Inspected(None);
        }
        let source = match self.inspect_deferred(InterpolationInput::new(input.source, input.span), input.style, None) {
            Ok(Inspected::Deferred(value)) if enabled => {
                if let Err(error) = value.validate_host_bind() {
                    self.unsupported(subject, &error.to_string(), input.span);
                    return MountMapping::Inspected(None);
                }
                MountSource::DeferredHostPath(value)
            }
            Ok(Inspected::Literal(value)) if super::is_host_path(&value) => MountSource::HostPath(value),
            Ok(Inspected::Deferred(_) | Inspected::Literal(_)) => return MountMapping::Ordinary,
            Err(reason) => {
                if !enabled {
                    return MountMapping::Ordinary;
                }
                self.unsupported(subject, reason, input.span);
                return MountMapping::Inspected(None);
            }
        };
        if !input.target.starts_with('/') || input.target.chars().any(|ch| ch.is_control() || ch == ':') {
            self.unsupported(subject, "deferred bind requires an absolute literal target", input.span);
            return MountMapping::Inspected(None);
        }
        MountMapping::Inspected(self.inspected_mount_model(subject, mount, source, &input))
    }

    fn inspected_mount_model(
        &mut self,
        subject: &str,
        mount: &VolumeMount,
        source: MountSource,
        input: &MountInput<'_>,
    ) -> Option<Mount> {
        match mount {
            VolumeMount::Short(value) => {
                let mut result =
                    Mount::new(source, input.target, value.options().iter().any(|value| value == "ro")).ok()?;
                for option in value.options() {
                    match option.as_str() {
                        "" | "rw" | "ro" => {}
                        "z" => result.set_selinux_relabel(boxferry_model::SelinuxRelabel::Shared),
                        "Z" => result.set_selinux_relabel(boxferry_model::SelinuxRelabel::Private),
                        _ => {
                            self.unsupported(subject, "unsupported deferred bind option", input.span);
                            return None;
                        }
                    }
                }
                Some(result)
            }
            VolumeMount::Long(value) => {
                let original = self.map_long_mount(subject, value)?;
                let mut result = Mount::new(source, original.target(), original.read_only()).ok()?;
                if let Some(relabel) = original.selinux_relabel() {
                    result.set_selinux_relabel(relabel);
                }
                Some(result)
            }
        }
    }
}

enum MountMapping {
    Ordinary,
    Inspected(Option<Mount>),
}
struct MountInput<'a> {
    source: &'a str,
    target: &'a str,
    span: compose_lens::source::SourceSpan,
    style: ScalarPresentation,
    full: Option<&'a str>,
    bind: bool,
    dynamic_options: bool,
}
