//! Facade-owned in-memory mutation reservation and publication authority.

use std::sync::Arc;

use parking_lot::Mutex;
use selene_catalog::{CatalogGeneration, GraphId};
use selene_core::GraphId as CoreGraphId;
use selene_graph::{SharedGraph, write_txn::PreparedGraphCommit};

use crate::{
    Error, Result,
    database::{DatabaseInner, DatabaseState, GraphInstance},
};

/// One lifetime-free proof that facade mutation code owns the common writer.
pub(crate) struct MutationReservation {
    _private: (),
}

/// Durability-independent result of the in-memory authority cut-line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    dead_code,
    reason = "non-committed authority outcomes are injected only by test failpoints until M09"
)]
pub(crate) enum AuthorityOutcome {
    /// Publication was canceled before the outer state store.
    Canceled,
    /// The complete state was stored and acknowledged.
    Committed,
    /// The complete state was stored, but acknowledgement was uncertain.
    Indeterminate,
}

#[derive(Clone, Copy)]
struct PinnedGraph {
    id: GraphId,
    instance_identity: usize,
    generation: u64,
}

/// Lifetime-free catalog/graph draft pinned to one outer database allocation.
pub(crate) struct DatabaseDraft {
    base: Arc<DatabaseState>,
    base_catalog_generation: CatalogGeneration,
    pub(crate) next: DatabaseState,
    pinned_graph: Option<PinnedGraph>,
    prepared_graph: Option<PreparedGraphCommit>,
    forget_graphs: Vec<CoreGraphId>,
}

impl DatabaseDraft {
    pub(crate) fn new(inner: &DatabaseInner, _reservation: &MutationReservation) -> Self {
        let base = inner.state.load_full();
        Self {
            base_catalog_generation: base.catalog.generation(),
            next: DatabaseState {
                catalog: base.catalog.clone(),
                graphs: base.graphs.clone(),
                graph_types: base.graph_types.clone(),
                high_water: base.high_water,
            },
            base,
            pinned_graph: None,
            prepared_graph: None,
            forget_graphs: Vec::new(),
        }
    }

    pub(crate) fn base(&self) -> Arc<DatabaseState> {
        Arc::clone(&self.base)
    }

    pub(crate) const fn state(&self) -> &DatabaseState {
        &self.next
    }

    pub(crate) fn forget_graph(&mut self, id: CoreGraphId) {
        self.forget_graphs.push(id);
    }

    pub(crate) fn pin_graph(&mut self, id: GraphId) -> Result<Arc<GraphInstance>> {
        let instance = self
            .base
            .graphs
            .get(&id)
            .cloned()
            .ok_or_else(Error::stale_session_reference)?;
        let snapshot = instance.graph.read();
        if snapshot.graph_id().get() != id.get() {
            return Err(Error::catalog_invariant(
                "registered graph identity disagrees with its catalog identity",
            ));
        }
        self.pinned_graph = Some(PinnedGraph {
            id,
            instance_identity: Arc::as_ptr(&instance) as usize,
            generation: snapshot.meta.generation,
        });
        drop(snapshot);
        Ok(instance)
    }

    pub(crate) fn attach_prepared_graph(
        &mut self,
        id: GraphId,
        prepared: PreparedGraphCommit,
    ) -> Result<()> {
        let Some(pinned) = self.pinned_graph else {
            return Err(Error::catalog_invariant(
                "prepared graph commit has no pinned graph base",
            ));
        };
        if pinned.id != id || prepared.snapshot().graph_id().get() != id.get() {
            return Err(Error::catalog_invariant(
                "prepared graph commit changed the stable graph identity",
            ));
        }
        if prepared.snapshot().meta.generation != pinned.generation.saturating_add(1) {
            return Err(Error::catalog_invariant(
                "prepared graph generation is not the pinned generation successor",
            ));
        }
        self.prepared_graph = Some(prepared);
        Ok(())
    }

    pub(crate) fn materialize_prepared_graph(&mut self, id: GraphId) -> Result<()> {
        let prepared = self.prepared_graph.take().ok_or_else(|| {
            Error::catalog_invariant("database draft has no prepared graph commit")
        })?;
        let graph = SharedGraph::try_from_graph(prepared.into_snapshot())
            .map_err(Error::invalid_graph_type_source)?;
        self.next
            .graphs
            .insert(id, Arc::new(GraphInstance::new(graph)));
        Ok(())
    }
}

/// One facade-owned serial mutation coordinator.
pub(crate) struct MutationCoordinator {
    writer: Mutex<()>,
}

impl MutationCoordinator {
    pub(crate) const fn new() -> Self {
        Self {
            writer: Mutex::new(()),
        }
    }
}

impl DatabaseInner {
    /// Run one mutation while holding the facade's only writer reservation.
    pub(crate) fn with_mutation_reservation<T>(
        &self,
        execute: impl FnOnce(MutationReservation) -> T,
    ) -> T {
        #[cfg(test)]
        assert_eq!(
            crate::database::GraphRequestDepth::current(),
            0,
            "catalog lifecycle entered under a same-thread graph request lease"
        );
        let _writer = self.transactions.writer.lock();
        execute(MutationReservation { _private: () })
    }

    /// Validate, publish once, clean graph-scoped runtime state, and acknowledge.
    pub(crate) fn publish_database_draft(
        &self,
        _reservation: MutationReservation,
        draft: DatabaseDraft,
    ) -> Result<AuthorityOutcome> {
        #[cfg(test)]
        if self.take_failure(crate::catalog::FailurePoint::BeforeAuthorityPrepare) {
            return Ok(AuthorityOutcome::Canceled);
        }
        #[cfg(test)]
        if self.take_failure(crate::catalog::FailurePoint::BeforeAuthorityFlush) {
            return Ok(AuthorityOutcome::Canceled);
        }

        let current = self.state.load_full();
        if !Arc::ptr_eq(&current, &draft.base)
            || current.catalog.generation() != draft.base_catalog_generation
        {
            return Err(Error::catalog_invariant(
                "database mutation base changed while its reservation was active",
            ));
        }
        if let Some(pinned) = draft.pinned_graph {
            let instance = current
                .graphs
                .get(&pinned.id)
                .ok_or_else(Error::stale_session_reference)?;
            let snapshot = instance.graph.read();
            if Arc::as_ptr(instance) as usize != pinned.instance_identity
                || snapshot.graph_id().get() != pinned.id.get()
                || snapshot.meta.generation != pinned.generation
            {
                return Err(Error::stale_session_reference());
            }
            let replacement = draft.next.graphs.get(&pinned.id).ok_or_else(|| {
                Error::catalog_invariant("prepared database state lost its selected graph")
            })?;
            let replacement_snapshot = replacement.graph.read();
            if replacement_snapshot.graph_id().get() != pinned.id.get()
                || replacement_snapshot.meta.generation != pinned.generation.saturating_add(1)
            {
                return Err(Error::catalog_invariant(
                    "prepared database state has a stale graph identity or generation",
                ));
            }
        }

        #[cfg(test)]
        if self.take_failure(crate::catalog::FailurePoint::BeforePublication) {
            return Ok(AuthorityOutcome::Canceled);
        }

        let forget_graphs = draft.forget_graphs;
        let next = Arc::new(draft.next);

        // The sole facade mutation cut-line. No facade code outside this
        // authority may store the outer database state.
        self.state.store(next);
        for id in forget_graphs {
            self.procedures.forget_graph(id);
        }

        #[cfg(test)]
        if self.take_failure(crate::catalog::FailurePoint::AfterPublicationAcknowledgement) {
            return Ok(AuthorityOutcome::Indeterminate);
        }
        Ok(AuthorityOutcome::Committed)
    }

    #[cfg(test)]
    fn take_failure(&self, point: crate::catalog::FailurePoint) -> bool {
        let mut failure = self.failure.lock();
        if *failure == Some(point) {
            failure.take();
            true
        } else {
            false
        }
    }
}

pub(crate) fn require_committed(outcome: AuthorityOutcome) -> Result<()> {
    match outcome {
        AuthorityOutcome::Committed => Ok(()),
        AuthorityOutcome::Canceled => Err(Error::catalog_invariant(
            "database publication canceled before the outer state store",
        )),
        AuthorityOutcome::Indeterminate => Err(Error::catalog_invariant(
            "database publication acknowledgement is indeterminate",
        )),
    }
}

const _: fn() = || {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<MutationReservation>();
    assert_send_static::<DatabaseDraft>();
    assert_send_static::<AuthorityOutcome>();
};

#[cfg(test)]
#[path = "transaction/tests.rs"]
mod tests;
