//! Statement-session state for explicit transaction control.

use std::sync::Arc;

use selene_graph::{SharedGraph, WriteTxn};

/// Caller-owned executor session bound to one shared graph.
pub struct Session<'g> {
    graph: &'g SharedGraph,
    principal: Option<Arc<[u8]>>,
    pub(crate) active_txn: Option<WriteTxn<'g>>,
    pub(crate) aborted: bool,
}

impl<'g> Session<'g> {
    /// Create a session without commit-principal bytes.
    #[must_use]
    pub const fn new(graph: &'g SharedGraph) -> Self {
        Self {
            graph,
            principal: None,
            active_txn: None,
            aborted: false,
        }
    }

    /// Create a session that forwards opaque principal bytes to commits.
    #[must_use]
    pub fn with_principal(graph: &'g SharedGraph, principal: Arc<[u8]>) -> Self {
        Self {
            graph,
            principal: Some(principal),
            active_txn: None,
            aborted: false,
        }
    }

    /// Borrow the graph this session executes against.
    #[must_use]
    pub(crate) const fn graph(&self) -> &'g SharedGraph {
        self.graph
    }

    /// Clone the principal bytes for a commit boundary.
    #[must_use]
    pub(crate) fn principal(&self) -> Option<Arc<[u8]>> {
        self.principal.clone()
    }

    /// Return true when the session owns an explicit write transaction.
    #[must_use]
    pub const fn has_active_txn(&self) -> bool {
        self.active_txn.is_some()
    }

    /// Return true when the active explicit transaction is aborted.
    #[must_use]
    pub const fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// Roll back and clear the explicit transaction, when one is active.
    pub fn abort(&mut self) {
        if let Some(txn) = self.active_txn.take() {
            txn.rollback();
        }
        self.aborted = false;
    }
}
