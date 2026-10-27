//! Controlled parsing of protected service-execution CLI providers.
use boxferry::{
    Identifier, Provenance, QuadletDeferredOptions, ServiceEnvironmentAssignment, ServiceEnvironmentFile,
    ServiceEnvironmentName, ServiceExecutionEnvironment, SourceId, Sourced,
};
use clap::Args;
use std::{collections::BTreeMap, fmt, io};

#[derive(Args, Default)]
pub(super) struct DeferredCliOptions {
    /// Defer one direct reference to the named service execution environment; repeat as needed.
    #[arg(long = "quadlet-defer", value_name = "SERVICE:NAME")]
    references: Vec<String>,
    /// Mandatory absolute service execution environment file; never read during conversion.
    #[arg(long = "quadlet-service-env-file", value_name = "SERVICE=ABSOLUTE_PATH")]
    files: Vec<String>,
    /// Protected literal service execution assignment; requires --environment-values include.
    #[arg(long = "quadlet-service-env", value_name = "SERVICE:NAME=VALUE")]
    assignments: Vec<String>,
}
impl fmt::Debug for DeferredCliOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeferredCliOptions")
            .field("references", &self.references.len())
            .field("files", &self.files.len())
            .field("assignments", &"[REDACTED]")
            .finish()
    }
}
#[derive(Default)]
struct Plan {
    references: Vec<Sourced<ServiceEnvironmentName>>,
    assignments: Vec<Sourced<ServiceEnvironmentAssignment>>,
    files: Vec<Sourced<ServiceEnvironmentFile>>,
}
fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}
fn sourced<T>(value: T) -> io::Result<Sourced<T>> {
    Ok(Sourced::from_source(
        value,
        Provenance::user_override(SourceId::new("quadlet-deferred-cli").map_err(io::Error::other)?),
    ))
}
fn service(value: &str) -> io::Result<Identifier> {
    if value.is_empty() || value.chars().any(|ch| ch.is_control() || matches!(ch, ':' | '=')) {
        return Err(invalid("invalid deferred service identity"));
    }
    Identifier::new(value).map_err(|_| invalid("invalid deferred service identity"))
}
impl DeferredCliOptions {
    pub(super) fn is_empty(&self) -> bool {
        self.references.is_empty() && self.assignments.is_empty() && self.files.is_empty()
    }
    pub(super) fn has_inline(&self) -> bool {
        !self.assignments.is_empty()
    }
    pub(super) fn plan(&self) -> io::Result<QuadletDeferredOptions> {
        let mut plans = BTreeMap::<Identifier, Plan>::new();
        for input in &self.references {
            let (owner, name) = input
                .split_once(':')
                .ok_or_else(|| invalid("--quadlet-defer requires SERVICE:NAME"))?;
            let name = ServiceEnvironmentName::new(name).map_err(|_| invalid("invalid deferred reference name"))?;
            plans
                .entry(service(owner)?)
                .or_default()
                .references
                .push(sourced(name)?);
        }
        for input in &self.assignments {
            let (owner, assignment) = input
                .split_once(':')
                .ok_or_else(|| invalid("--quadlet-service-env requires SERVICE:NAME=VALUE"))?;
            let (name, value) = assignment
                .split_once('=')
                .ok_or_else(|| invalid("--quadlet-service-env requires a literal NAME=VALUE assignment"))?;
            let name =
                ServiceEnvironmentName::new(name).map_err(|_| invalid("invalid inline service provider name"))?;
            let assignment = ServiceEnvironmentAssignment::new(name, value)
                .map_err(|_| invalid("invalid inline service provider assignment"))?;
            plans
                .entry(service(owner)?)
                .or_default()
                .assignments
                .push(sourced(assignment)?);
        }
        for input in &self.files {
            let (owner, path) = input
                .split_once('=')
                .ok_or_else(|| invalid("--quadlet-service-env-file requires SERVICE=ABSOLUTE_PATH"))?;
            let file = ServiceEnvironmentFile::new(path).map_err(|_| invalid("service provider files must be mandatory absolute literal paths without glob activation or control characters"))?;
            plans.entry(service(owner)?).or_default().files.push(sourced(file)?);
        }
        let mut options = QuadletDeferredOptions::new();
        for (owner, plan) in plans {
            let plan = ServiceExecutionEnvironment::new(plan.references, plan.assignments, plan.files)
                .map_err(io::Error::other)?;
            options.add_service(owner, sourced(plan)?).map_err(io::Error::other)?;
        }
        Ok(options)
    }
}
