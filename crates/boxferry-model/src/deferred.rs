//! Explicit execution environment requirements without native variable syntax.
use crate::{EnvironmentValue, ModelError, MountSource, ProtectedString, Service, Sourced};
use std::{collections::BTreeSet, fmt};

/// Maximum decoded bytes in an expression or execution provider plan.
pub const DEFERRED_MAX_BYTES: usize = 1_048_576;
/// Maximum flat segments or provider entries.
pub const DEFERRED_MAX_SEGMENTS: usize = 65_536;

/// Portable environment reference name.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ServiceEnvironmentName(String);
impl fmt::Debug for ServiceEnvironmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceEnvironmentName([REDACTED])")
    }
}
impl ServiceEnvironmentName {
    /// Validates the portable ASCII identifier grammar.
    /// # Errors
    /// Returns a value-free error for invalid names.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        let mut bytes = value.bytes();
        if !bytes.next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.len() > DEFERRED_MAX_BYTES
        {
            return Err(ModelError::InvalidDeferred("invalid service environment name"));
        }
        Ok(Self(value))
    }
    /// Returns the validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A flat expression segment, retaining literal protection independently of references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeferredSegment {
    /// Literal text, never parsed as variable syntax.
    Literal(ProtectedString),
    /// Service execution environment lookup.
    Reference(ServiceEnvironmentName),
}

/// Bounded flat concatenation with per-segment provenance.
///
/// Unset or empty references contribute an empty string to container environment assignments.
/// Host-bind expressions additionally require a nonempty absolute effective source with no colon
/// or control characters. These are external prerequisites, not runtime verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferredExpression(Vec<Sourced<DeferredSegment>>);
impl DeferredExpression {
    /// Creates an expression containing at least one explicit reference.
    /// # Errors
    /// Returns a value-free error for NUL, size limits, or missing references.
    pub fn new(parts: Vec<Sourced<DeferredSegment>>) -> Result<Self, ModelError> {
        let mut bytes = 0_usize;
        let mut reference = false;
        for part in &parts {
            let value = match part.value() {
                DeferredSegment::Literal(value) => value.expose(),
                DeferredSegment::Reference(name) => {
                    reference = true;
                    name.as_str()
                }
            };
            bytes = bytes.saturating_add(value.len());
            if value.contains('\0') {
                return Err(ModelError::InvalidDeferred("NUL in deferred expression"));
            }
        }
        if !reference || parts.len() > DEFERRED_MAX_SEGMENTS || bytes > DEFERRED_MAX_BYTES {
            return Err(ModelError::InvalidDeferred(
                "deferred expression requires references within finite bounds",
            ));
        }
        Ok(Self(parts))
    }
    /// Returns segments in concatenation order.
    #[must_use]
    pub fn segments(&self) -> &[Sourced<DeferredSegment>] {
        &self.0
    }
    /// Returns whether output needs explicit environment-value disclosure authorization.
    #[must_use]
    pub fn has_nonempty_literal(&self) -> bool {
        self.0
            .iter()
            .any(|part| matches!(part.value(), DeferredSegment::Literal(value) if !value.expose().is_empty()))
    }
    /// Validates one leading direct reference and a safe optional slash suffix.
    /// # Errors
    /// Returns a value-free error for unsupported host-bind shapes or unsafe delimiters.
    pub fn validate_host_bind(&self) -> Result<(), ModelError> {
        if !matches!(self.0.first().map(Sourced::value), Some(DeferredSegment::Reference(_))) {
            return Err(ModelError::InvalidDeferred("host bind requires one leading reference"));
        }
        let mut suffix = String::new();
        for part in self.0.iter().skip(1) {
            match part.value() {
                DeferredSegment::Literal(value) => suffix.push_str(value.expose()),
                DeferredSegment::Reference(_) => {
                    return Err(ModelError::InvalidDeferred("host bind requires one reference"));
                }
            }
        }
        if (!suffix.is_empty() && !suffix.starts_with('/')) || suffix.chars().any(|ch| ch.is_control() || ch == ':') {
            return Err(ModelError::InvalidDeferred(
                "unsafe host-bind suffix; use conversion-time resolution",
            ));
        }
        Ok(())
    }
}

/// Explicit protected assignment to the service execution environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceEnvironmentAssignment {
    name: ServiceEnvironmentName,
    value: ProtectedString,
}
impl ServiceEnvironmentAssignment {
    /// Admits the value as protected data.
    /// # Errors
    /// Returns a value-free error for NUL or oversized values.
    pub fn new(name: ServiceEnvironmentName, value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if value.contains('\0') || value.len() > DEFERRED_MAX_BYTES {
            return Err(ModelError::InvalidDeferred(
                "invalid inline service environment assignment",
            ));
        }
        Ok(Self {
            name,
            value: ProtectedString::sensitive(value),
        })
    }
    /// Returns the provider name.
    #[must_use]
    pub const fn name(&self) -> &ServiceEnvironmentName {
        &self.name
    }
    /// Returns the protected value.
    #[must_use]
    pub const fn value(&self) -> &ProtectedString {
        &self.value
    }
}

/// Mandatory absolute literal external file; construction never reads the file.
#[derive(Clone, Eq, PartialEq)]
pub struct ServiceEnvironmentFile(String);
impl fmt::Debug for ServiceEnvironmentFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceEnvironmentFile([REDACTED])")
    }
}
impl ServiceEnvironmentFile {
    /// Validates an absolute path without control characters or glob activation; dollars and percent signs remain literal.
    /// # Errors
    /// Returns a value-free error for invalid paths.
    pub fn new(path: impl Into<String>) -> Result<Self, ModelError> {
        let path = path.into();
        if !path.starts_with('/')
            || path.len() > DEFERRED_MAX_BYTES
            || path
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '*' | '?' | '[' | ']'))
        {
            return Err(ModelError::InvalidDeferred(
                "service environment file requires an absolute literal path",
            ));
        }
        Ok(Self(path))
    }
    /// Returns the literal path without filesystem access.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.0
    }
}

/// Explicit per-service allowlist and external execution providers.
///
/// Files apply after inline assignments, in declaration order; later files override earlier values,
/// including empty overrides. Provider variables are never automatically forwarded to containers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceExecutionEnvironment {
    references: Vec<Sourced<ServiceEnvironmentName>>,
    assignments: Vec<Sourced<ServiceEnvironmentAssignment>>,
    files: Vec<Sourced<ServiceEnvironmentFile>>,
}
impl ServiceExecutionEnvironment {
    /// Creates a bounded plan with unique names and inline assignments covered by the allowlist.
    /// # Errors
    /// Returns a value-free error for duplicates, undeclared names, or size limits.
    pub fn new(
        references: Vec<Sourced<ServiceEnvironmentName>>,
        assignments: Vec<Sourced<ServiceEnvironmentAssignment>>,
        files: Vec<Sourced<ServiceEnvironmentFile>>,
    ) -> Result<Self, ModelError> {
        let names: BTreeSet<_> = references.iter().map(Sourced::value).collect();
        let file_paths: BTreeSet<_> = files.iter().map(|item| item.value().path()).collect();
        let inline: BTreeSet<_> = assignments.iter().map(|item| item.value().name()).collect();
        let bytes = references
            .iter()
            .map(|item| item.value().as_str().len())
            .chain(assignments.iter().map(|item| {
                item.value()
                    .name()
                    .as_str()
                    .len()
                    .saturating_add(item.value().value().expose().len())
            }))
            .chain(files.iter().map(|item| item.value().path().len()))
            .fold(0_usize, usize::saturating_add);
        if file_paths.len() != files.len()
            || names.is_empty()
            || names.len() != references.len()
            || inline.len() != assignments.len()
            || !inline.is_subset(&names)
            || references
                .len()
                .saturating_add(assignments.len())
                .saturating_add(files.len())
                > DEFERRED_MAX_SEGMENTS
            || bytes > DEFERRED_MAX_BYTES
        {
            return Err(ModelError::InvalidDeferred(
                "invalid or duplicate service environment plan entries",
            ));
        }
        Ok(Self {
            references,
            assignments,
            files,
        })
    }
    /// Returns declared reference names.
    #[must_use]
    pub fn references(&self) -> &[Sourced<ServiceEnvironmentName>] {
        &self.references
    }
    /// Returns inline assignments in declaration order.
    #[must_use]
    pub fn assignments(&self) -> &[Sourced<ServiceEnvironmentAssignment>] {
        &self.assignments
    }
    /// Returns files in precedence order.
    #[must_use]
    pub fn files(&self) -> &[Sourced<ServiceEnvironmentFile>] {
        &self.files
    }
}

pub(crate) fn validate_service(service: &Service) -> Result<(), ModelError> {
    let mut used = BTreeSet::new();
    for expression in service
        .environment()
        .iter()
        .filter_map(|entry| match entry.value().value() {
            EnvironmentValue::Deferred(value) => Some(value),
            _ => None,
        })
        .chain(
            service
                .mounts()
                .iter()
                .filter_map(|entry| match entry.value().source() {
                    MountSource::DeferredHostPath(value) => Some(value),
                    _ => None,
                }),
        )
    {
        for segment in expression.segments() {
            if let DeferredSegment::Reference(name) = segment.value() {
                used.insert(name);
            }
        }
    }
    for mount in service.mounts() {
        if let MountSource::DeferredHostPath(expression) = mount.value().source() {
            expression.validate_host_bind()?;
        }
    }
    match service.execution_environment() {
        None if used.is_empty() => Ok(()),
        Some(plan) => {
            let plan = plan.value();
            if plan.files().is_empty() {
                for mount in service.mounts() {
                    if let MountSource::DeferredHostPath(expression) = mount.value().source()
                        && let Some(DeferredSegment::Reference(name)) =
                            expression.segments().first().map(Sourced::value)
                        && let Some(assignment) = plan.assignments().iter().find(|item| item.value().name() == name)
                    {
                        let value = assignment.value().value().expose();
                        if !value.starts_with('/') || value.chars().any(|ch| ch.is_control() || ch == ':') {
                            return Err(ModelError::InvalidDeferred(
                                "inline bind root must be nonempty absolute and free of colon or control characters",
                            ));
                        }
                    }
                }
            }
            let declared: BTreeSet<_> = plan.references().iter().map(Sourced::value).collect();
            let provided: BTreeSet<_> = plan.assignments().iter().map(|item| item.value().name()).collect();
            if used != declared || used.is_empty() || (plan.files().is_empty() && !used.is_subset(&provided)) {
                Err(ModelError::InvalidDeferred(
                    "every used reference must be declared and externally bound; unused plans are forbidden",
                ))
            } else {
                Ok(())
            }
        }
        None => Err(ModelError::InvalidDeferred(
            "deferred references require an explicit execution environment",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, EnvironmentVariable, Identifier};
    fn name(value: &str) -> Result<ServiceEnvironmentName, ModelError> {
        ServiceEnvironmentName::new(value)
    }
    fn reference(value: &str) -> Result<DeferredExpression, ModelError> {
        DeferredExpression::new(vec![Sourced::generated(DeferredSegment::Reference(name(value)?))])
    }
    #[test]
    fn invalid_names_and_paths_return_value_free_errors() -> Result<(), ModelError> {
        for value in ["", "1BAD", "A-B", "PRIVATE=CANARY", "A\0B"] {
            assert!(name(value).is_err());
        }
        for value in ["relative", "-/optional", "/path/*", "/path/\n"] {
            assert!(ServiceEnvironmentFile::new(value).is_err());
        }
        let value = ServiceEnvironmentAssignment::new(name("VALUE")?, "PRIVATE_CANARY")?;
        assert!(!format!("{value:?}").contains("PRIVATE_CANARY"));
        Ok(())
    }
    #[test]
    fn application_rejects_undeclared_unbound_and_unused_plans() -> Result<(), ModelError> {
        for (declared, bound, used) in [("A", true, "B"), ("A", false, "A"), ("A", true, "")] {
            let mut service = Service::new(Identifier::new("app")?);
            if !used.is_empty() {
                service.add_environment(Sourced::generated(EnvironmentVariable::new(
                    Identifier::new("CONTAINER")?,
                    EnvironmentValue::Deferred(reference(used)?),
                )));
            }
            let inline = if bound {
                vec![Sourced::generated(ServiceEnvironmentAssignment::new(
                    name(declared)?,
                    "",
                )?)]
            } else {
                Vec::new()
            };
            service.set_execution_environment(Sourced::generated(ServiceExecutionEnvironment::new(
                vec![Sourced::generated(name(declared)?)],
                inline,
                Vec::new(),
            )?));
            assert!(
                Application::new(Identifier::new("test")?)
                    .add_service(Sourced::generated(service))
                    .is_err()
            );
        }
        Ok(())
    }
    #[test]
    fn bind_shape_and_known_inline_roots_are_checked() -> Result<(), ModelError> {
        for suffix in [":bad", "relative", "/bad\n"] {
            let value = DeferredExpression::new(vec![
                Sourced::generated(DeferredSegment::Reference(name("ROOT")?)),
                Sourced::generated(DeferredSegment::Literal(ProtectedString::plain(suffix))),
            ])?;
            assert!(value.validate_host_bind().is_err());
        }
        assert!(reference("ROOT")?.validate_host_bind().is_ok());
        assert!(
            DeferredExpression::new(vec![Sourced::generated(DeferredSegment::Literal(
                ProtectedString::plain("literal")
            ))])
            .is_err()
        );
        assert!(
            ServiceExecutionEnvironment::new(
                vec![Sourced::generated(name("A")?), Sourced::generated(name("A")?)],
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn expression_and_provider_bounds_are_finite() -> Result<(), ModelError> {
        let expression = vec![
            Sourced::generated(DeferredSegment::Reference(name("A")?)),
            Sourced::generated(DeferredSegment::Literal(ProtectedString::sensitive(
                "x".repeat(DEFERRED_MAX_BYTES),
            ))),
        ];
        assert!(DeferredExpression::new(expression).is_err());
        assert!(
            DeferredExpression::new(vec![
                Sourced::generated(DeferredSegment::Reference(name("A")?));
                DEFERRED_MAX_SEGMENTS + 1
            ])
            .is_err()
        );
        assert!(ServiceEnvironmentAssignment::new(name("A")?, "x".repeat(DEFERRED_MAX_BYTES + 1)).is_err());
        assert!(!format!("{:?}", ServiceEnvironmentFile::new("/PRIVATE_PATH")?).contains("PRIVATE_PATH"));
        assert!(!format!("{:?}", name("PRIVATE_NAME")?).contains("PRIVATE_NAME"));
        Ok(())
    }
}
