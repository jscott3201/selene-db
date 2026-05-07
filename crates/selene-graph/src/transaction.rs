#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Transaction and mutation-funnel skeleton for `selene-graph`.
//!
//! This module records the BRIEF-03 / D10 lifecycle without implementing the
//! M2 graph engine. A write transaction starts at `Graph::begin_write`, obtains
//! the per-graph `parking_lot::RwLock` write guard, and keeps that guard until
//! `commit`, `commit_with_principal`, `rollback`, or drop. A `Mutator` is only a
//! borrowed handle of `WriteTxn`, so mutation records cannot outlive the held
//! lock. See `_spec/03-property-graph-and-concurrency.md` §4.3 and §6.2 for the
//! commit boundary and serializable-isolation argument.

use std::sync::Arc;

/// Graph handle that owns the per-graph write-lock.
///
/// The lock shape is specified by D10. Runtime storage fields are M2 work; the
/// field here is illustrative so the public transaction skeleton records that
/// `parking_lot::RwLock` is the coordination primitive. See `_spec/03` §6.1.
pub struct Graph {
    _lock: parking_lot::RwLock<GraphState>,
}

impl Graph {
    /// Start a read-write transaction by acquiring the single graph write-lock.
    ///
    /// The returned [`WriteTxn`] holds the lock until commit, rollback, or drop.
    /// See `_spec/03` §6.2 for the `START TRANSACTION` lifecycle.
    pub fn begin_write(&self) -> Result<WriteTxn<'_>, TxError> {
        unimplemented!("M2 work")
    }
}

/// Read-write transaction handle for a single graph.
///
/// A `WriteTxn` is the RAII owner of the per-graph write guard. Dropping it
/// without committing is rollback. See `_spec/03` §6.2 for the commit boundary.
pub struct WriteTxn<'g> {
    _guard: parking_lot::RwLockWriteGuard<'g, GraphState>,
}

impl<'g> WriteTxn<'g> {
    /// Borrow a mutator tied to this transaction.
    ///
    /// The returned [`Mutator`] cannot outlive the transaction, which preserves
    /// the single mutation funnel described in `_spec/03` §4.3.
    pub fn mutator(&mut self) -> Mutator<'_, 'g> {
        unimplemented!("M2 work")
    }

    /// Commit without caller principal bytes.
    ///
    /// Equivalent to `commit_with_principal(None)`. See `_spec/03` §6.2 for the
    /// graph publication boundary and `_spec/04` §3.2 for WAL header fields.
    pub fn commit(self) -> Result<CommitOutcome, TxError> {
        unimplemented!("M2 work")
    }

    /// Commit with optional caller-owned principal bytes for D12 audit replay.
    ///
    /// The principal is forwarded to `selene-persist` unchanged. See `_spec/04`
    /// §3.2 for the `principal: Option<Arc<[u8]>>` WAL header slot.
    pub fn commit_with_principal(
        self,
        _principal: Option<Arc<[u8]>>,
    ) -> Result<CommitOutcome, TxError> {
        unimplemented!("M2 work")
    }

    /// Roll back graph changes and release the write-lock.
    ///
    /// IDs allocated during the transaction remain holes per D11 and spec 02
    /// §4. See `_spec/03` §6.2 for rollback behavior.
    pub fn rollback(self) {
        unimplemented!("M2 work")
    }
}

/// Borrowed mutation builder for one [`WriteTxn`].
///
/// This is the only type that produces persistent `Change` records. It borrows
/// the transaction so it cannot be used after the write-lock is released. See
/// `_spec/03` §4.3.
pub struct Mutator<'tx, 'g> {
    _txn: &'tx mut WriteTxn<'g>,
}

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Create a node and return its real graph-scoped ID immediately.
    ///
    /// Allocation happens under the write-lock via `IdAllocator`; aborted IDs
    /// become holes. See `_spec/02` §4 and `_spec/03` §6.2.
    pub fn create_node(&mut self, _labels: LabelSet, _props: PropertyMap) -> NodeId {
        unimplemented!("M2 work")
    }

    /// Create an edge and return its real graph-scoped ID immediately.
    ///
    /// Allocation follows the same D11 rule as node IDs. See `_spec/02` §4.
    pub fn create_edge(
        &mut self,
        _label: IStr,
        _source: NodeId,
        _target: NodeId,
        _props: PropertyMap,
    ) -> EdgeId {
        unimplemented!("M2 work")
    }

    /// Record a node update in the transaction-local change buffer.
    ///
    /// The update is not visible to readers until [`WriteTxn::commit`].
    pub fn update_node(&mut self, _id: NodeId) {
        unimplemented!("M2 work")
    }

    /// Record a node deletion in the transaction-local change buffer.
    ///
    /// Deletion clears liveness but does not make the ID reusable. See spec 02
    /// §4 for monotonic ID behavior.
    pub fn delete_node(&mut self, _id: NodeId) {
        unimplemented!("M2 work")
    }

    /// Record an edge deletion in the transaction-local change buffer.
    ///
    /// The deletion becomes durable only at the commit boundary in `_spec/03`
    /// §6.2.
    pub fn delete_edge(&mut self, _id: EdgeId) {
        unimplemented!("M2 work")
    }
}

/// Result metadata returned after a successful commit.
///
/// The final M2 type will carry the committed graph generation, WAL sequence,
/// and any statistics needed by callers. See `_spec/03` §6.2.
pub struct CommitOutcome {
    _generation: u64,
    _sequence: u64,
}

/// Transaction failure reported before graph publication.
///
/// Commit errors leave the published `ArcSwap` snapshot unchanged. See
/// `_spec/03` §6.2 and `_spec/04` §3.4.
#[derive(Debug)]
pub enum TxError {
    /// Placeholder until M2 defines validation, WAL, and poisoned-lock errors.
    M2Placeholder,
}

/// Placeholder graph state for the BRIEF-03 skeleton.
pub struct GraphState;

/// Placeholder node ID; the final type lives in `selene-core`.
pub struct NodeId(u64);

/// Placeholder edge ID; the final type lives in `selene-core`.
pub struct EdgeId(u64);

/// Placeholder interned string handle; the final type lives in `selene-core`.
pub struct IStr(u32);

/// Placeholder label set; the final type lives in `selene-core`.
pub struct LabelSet;

/// Placeholder property map; the final type lives in `selene-core`.
pub struct PropertyMap;
