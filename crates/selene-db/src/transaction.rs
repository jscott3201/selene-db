//! Facade-owned in-memory mutation reservation and publication authority.
//!
//! The writer capability is universally quantified so its lifetime cannot be
//! selected by the caller or appear in the closure's result:
//!
//! ```compile_fail,E0308
//! use std::marker::PhantomData;
//!
//! struct MutationReservation<'writer>(PhantomData<&'writer mut ()>);
//!
//! fn reserve<T>(
//!     execute: impl for<'writer> FnOnce(MutationReservation<'writer>) -> T,
//! ) -> T {
//!     execute(MutationReservation(PhantomData))
//! }
//!
//! let _escaped = reserve(|reservation| reservation);
//! ```

use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    rc::Rc,
    sync::Arc,
};

use parking_lot::{Mutex, MutexGuard};
use selene_catalog::{CatalogGeneration, CatalogSnapshot, GraphId, GraphTypeId};
use selene_core::GraphId as CoreGraphId;
use selene_graph::{GraphTypeDef, SeleneGraph, SharedGraph, write_txn::PreparedGraphCommit};

use crate::{
    Error, Result,
    database::{DatabaseInner, DatabaseState, GraphInstance, HighWaterMarks},
};

/// Closure-local proof that the facade's writer mutex remains held.
///
/// The invariant lifetime comes from a mutable borrow of the stack-local mutex
/// guard. `Rc` also keeps the capability on the reserving thread. Database
/// drafts remain lifetime-free; only this publication authority is borrowed.
pub(crate) struct MutationReservation<'writer> {
    _writer: PhantomData<&'writer mut ()>,
    _not_send: PhantomData<Rc<()>>,
}

impl<'writer> MutationReservation<'writer> {
    fn new(_writer: &'writer mut MutexGuard<'_, ()>) -> Self {
        Self {
            _writer: PhantomData,
            _not_send: PhantomData,
        }
    }
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

enum DetachedGraphReplacement {
    Snapshot(Box<SeleneGraph>),
    Prepared(PreparedGraphCommit),
}

impl DetachedGraphReplacement {
    fn snapshot(&self) -> &SeleneGraph {
        match self {
            Self::Snapshot(snapshot) => snapshot,
            Self::Prepared(prepared) => prepared.snapshot(),
        }
    }

    fn into_snapshot(self) -> SeleneGraph {
        match self {
            Self::Snapshot(snapshot) => *snapshot,
            Self::Prepared(prepared) => prepared.into_snapshot(),
        }
    }
}

/// Lifetime-free detached catalog/graph draft pinned to outer-state metadata.
///
/// This type deliberately contains no outer state allocation, graph instance,
/// shared graph, transaction, lock guard, committer, or provider state.
pub(crate) struct DatabaseDraft {
    base_state_identity: usize,
    base_publication: u64,
    base_catalog_generation: CatalogGeneration,
    pub(crate) catalog: CatalogSnapshot,
    pub(crate) graph_types: BTreeMap<GraphTypeId, Arc<GraphTypeDef>>,
    pub(crate) high_water: HighWaterMarks,
    pinned_graph: Option<PinnedGraph>,
    graph_removals: BTreeSet<GraphId>,
    graph_replacement: Option<(GraphId, DetachedGraphReplacement)>,
    forget_graphs: Vec<CoreGraphId>,
}

impl DatabaseDraft {
    pub(crate) fn new(base: &Arc<DatabaseState>, _reservation: &MutationReservation<'_>) -> Self {
        Self {
            base_state_identity: Arc::as_ptr(base) as usize,
            base_publication: base.publication,
            base_catalog_generation: base.catalog.generation(),
            catalog: base.catalog.clone(),
            graph_types: base.graph_types.clone(),
            high_water: base.high_water,
            pinned_graph: None,
            graph_removals: BTreeSet::new(),
            graph_replacement: None,
            forget_graphs: Vec::new(),
        }
    }

    pub(crate) fn forget_graph(&mut self, id: CoreGraphId) {
        self.forget_graphs.push(id);
    }

    pub(crate) fn pin_graph(
        &mut self,
        base: &Arc<DatabaseState>,
        id: GraphId,
    ) -> Result<Arc<GraphInstance>> {
        if Arc::as_ptr(base) as usize != self.base_state_identity
            || base.publication != self.base_publication
        {
            return Err(Error::catalog_invariant(
                "graph pin does not belong to the database draft base",
            ));
        }
        let instance = base
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

    pub(crate) fn remove_graph(&mut self, id: GraphId) {
        self.graph_removals.insert(id);
    }

    pub(crate) fn replace_graph(&mut self, id: GraphId, snapshot: SeleneGraph) -> Result<()> {
        if snapshot.graph_id().get() != id.get() || self.graph_replacement.is_some() {
            return Err(Error::catalog_invariant(
                "database draft has an invalid or duplicate graph replacement",
            ));
        }
        self.graph_replacement = Some((id, DetachedGraphReplacement::Snapshot(Box::new(snapshot))));
        Ok(())
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
        if self.graph_replacement.is_some() {
            return Err(Error::catalog_invariant(
                "database draft already has a graph replacement",
            ));
        }
        self.graph_replacement = Some((id, DetachedGraphReplacement::Prepared(prepared)));
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
        execute: impl for<'writer> FnOnce(MutationReservation<'writer>) -> T,
    ) -> T {
        #[cfg(test)]
        assert_eq!(
            crate::database::GraphRequestDepth::current(),
            0,
            "catalog lifecycle entered under a same-thread graph request lease"
        );
        let mut writer = self.transactions.writer.lock();
        execute(MutationReservation::new(&mut writer))
    }

    /// Validate, publish once, clean graph-scoped runtime state, and acknowledge.
    pub(crate) fn publish_database_draft(
        &self,
        _reservation: MutationReservation<'_>,
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
        if Arc::as_ptr(&current) as usize != draft.base_state_identity
            || current.publication != draft.base_publication
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
            let Some((replacement_id, replacement)) = draft.graph_replacement.as_ref() else {
                return Err(Error::catalog_invariant(
                    "prepared database state lost its selected graph",
                ));
            };
            if *replacement_id != pinned.id
                || replacement.snapshot().graph_id().get() != pinned.id.get()
                || replacement.snapshot().meta.generation != pinned.generation.saturating_add(1)
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

        let DatabaseDraft {
            catalog,
            graph_types,
            high_water,
            graph_removals,
            graph_replacement,
            forget_graphs,
            ..
        } = draft;
        let mut graphs = current.graphs.clone();
        for id in graph_removals {
            graphs.remove(&id);
        }
        if let Some((id, replacement)) = graph_replacement {
            let graph = SharedGraph::try_from_graph(replacement.into_snapshot())
                .map_err(Error::invalid_graph_type_source)?;
            #[cfg(test)]
            self.replacement_graph_constructions
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            graphs.insert(id, Arc::new(GraphInstance::new(graph)));
        }
        let next = Arc::new(DatabaseState {
            publication: current.publication.saturating_add(1),
            catalog,
            graphs,
            graph_types,
            high_water,
        });

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
        AuthorityOutcome::Canceled => Err(Error::mutation_canceled()),
        AuthorityOutcome::Indeterminate => Err(Error::mutation_indeterminate()),
    }
}

const _: fn() = || {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<DatabaseDraft>();
    assert_send_static::<AuthorityOutcome>();
};

#[cfg(test)]
#[path = "transaction/tests.rs"]
mod tests;
