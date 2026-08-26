//! Request-owned execution stack, status collection, and table authority.

use parking_lot::{Mutex, MutexGuard};
use std::sync::Arc;

use super::{BindingTableRegistry, ExecutionStack, GqlStatusObject};

/// Request-owned runtime state shared by statement, procedure, and expression seams.
pub(crate) struct RequestRuntime {
    binding_tables: Arc<BindingTableRegistry>,
    stack: Mutex<ExecutionStack>,
    statuses: Mutex<Vec<GqlStatusObject>>,
}

/// Opaque request-runtime ownership handle used by the stable facade.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct RequestRuntimeHandle(Arc<RequestRuntime>);

impl RequestRuntimeHandle {
    /// Create one request authority with a root execution context.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(RequestRuntime::new()))
    }

    /// Clone every status produced so far in deterministic request order.
    #[doc(hidden)]
    #[must_use]
    pub fn statuses(&self) -> Vec<GqlStatusObject> {
        self.0.statuses()
    }

    pub(crate) fn inner(&self) -> Arc<RequestRuntime> {
        Arc::clone(&self.0)
    }
}

impl Default for RequestRuntimeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestRuntime {
    pub(crate) fn new() -> Self {
        Self {
            binding_tables: Arc::new(BindingTableRegistry::new()),
            stack: Mutex::new(ExecutionStack::new()),
            statuses: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn binding_tables(&self) -> Arc<BindingTableRegistry> {
        Arc::clone(&self.binding_tables)
    }

    pub(crate) fn stack(&self) -> MutexGuard<'_, ExecutionStack> {
        self.stack.lock()
    }

    pub(crate) fn record_status(&self, status: GqlStatusObject) {
        self.statuses.lock().push(status);
    }

    pub(crate) fn statuses(&self) -> Vec<GqlStatusObject> {
        self.statuses.lock().clone()
    }
}

impl std::fmt::Debug for RequestRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestRuntime")
            .field("table_count", &self.binding_tables.len())
            .field("stack_depth", &self.stack.lock().depth())
            .field("status_count", &self.statuses.lock().len())
            .finish()
    }
}
