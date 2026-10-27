//! Explicit Compose-to-Quadlet execution-environment policy.
use boxferry_model::{Identifier, ModelError, ServiceExecutionEnvironment, Sourced};
use std::collections::BTreeMap;

/// Per-service explicit deferral allowlists and service execution providers.
///
/// This policy never evaluates variables, reads provider files, or accesses the process environment.
/// Import un-interpolated native merged input. Every plan must belong to an active service and all
/// declared references must be used. Provider values do not imply container-variable forwarding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QuadletDeferredOptions {
    services: BTreeMap<Identifier, Sourced<ServiceExecutionEnvironment>>,
}
impl QuadletDeferredOptions {
    /// Creates an empty opt-in policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            services: BTreeMap::new(),
        }
    }
    /// Adds an explicit service plan without replacing a previous declaration.
    /// # Errors
    /// Returns a value-free error for duplicate service plans.
    pub fn add_service(
        &mut self,
        name: Identifier,
        plan: Sourced<ServiceExecutionEnvironment>,
    ) -> Result<(), ModelError> {
        if self.services.contains_key(&name) {
            return Err(ModelError::InvalidDeferred("duplicate service execution plan"));
        }
        self.services.insert(name, plan);
        Ok(())
    }
    /// Returns plans by service identity.
    #[must_use]
    pub const fn services(&self) -> &BTreeMap<Identifier, Sourced<ServiceExecutionEnvironment>> {
        &self.services
    }
    /// Returns whether the explicit mode has any plans.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}
