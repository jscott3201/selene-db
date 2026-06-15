//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::GraphId;
use selene_persist::{AuditLog, SyncPolicy, WalConfig, WalWriter};

use crate::committer_batch::CommitBatching;
use crate::core_provider::{CoreProvider, DurableState};
use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::graph_types::GraphTypeDef;
use crate::id_allocator::IdAllocator;
use crate::index_provider::{IndexProvider, ProviderTag};
use crate::vector_index::{VectorIndexMaintenancePolicy, VectorIndexRebuildReport};
use crate::write_txn::WriteTxn;

/// Per-graph shared runtime state.
///
/// Since v1.2 (BRIEF 1) every snapshot publish is funneled through a single
/// per-graph committer thread (`CommitterThread`), which is
/// the **sole writer** of the `snapshot` [`ArcSwap`] cell. `begin_write` hands
/// each [`WriteTxn`] a cheap submit handle; `commit`/`compact` seal-and-submit
/// to the committer and block until it publishes. This single-committer +
/// sole-publisher discipline is what preserves D10 strict-serializability once
/// `seal()` drops the write lock early — it is load-bearing and NOT
/// type-enforced (a second committer or ArcSwap writer would silently break it).
pub struct SharedGraph {
    shared: Arc<RwLock<Arc<SeleneGraph>>>,
    snapshot: Arc<ArcSwap<SeleneGraph>>,
    schema_version: Arc<AtomicU64>,
    allocator: Arc<Mutex<IdAllocator>>,
    /// Fixed provider registry, frozen at construction. Shared as one
    /// allocation so `begin_write` hands the registry to each transaction
    /// with a single refcount bump instead of a per-transaction `Vec` clone.
    providers: Arc<[Arc<dyn IndexProvider>]>,
    durable_providers: Vec<Arc<dyn DurableProvider>>,
    /// The single per-graph committer thread; sole publisher of `snapshot`.
    /// Dropped last via [`SharedGraph`]'s implicit drop order, which joins the
    /// thread once every outstanding [`WriteTxn`] submit handle is gone.
    committer: crate::committer::CommitterThread,
}

impl SharedGraph {
    /// Construct an empty shared graph.
    #[must_use]
    pub fn new(graph_id: GraphId) -> Self {
        Self::from_graph(SeleneGraph::new(graph_id))
    }

    /// Start building an empty shared graph with optional providers.
    #[must_use]
    pub fn builder(graph_id: GraphId) -> SharedGraphBuilder {
        SharedGraphBuilder::new(graph_id)
    }

    /// Construct shared state from a pre-built graph snapshot.
    ///
    /// The allocator floors are derived from storage length so that stale
    /// `GraphMeta.next_*_id` values cannot allow ID reuse over rows that
    /// already exist (recovery hardening — spec 02 §4 forbids ID reuse).
    ///
    /// # Panics
    ///
    /// Panics if the supplied graph contains more than `u32::MAX` rows in
    /// either store. Selene-graph's row index is `u32` by construction;
    /// `SeleneGraph::new()` always satisfies this, and any caller-built
    /// fixture must too. Use [`SharedGraph::try_from_graph`] for the
    /// fallible variant when validating untrusted snapshots.
    #[must_use]
    pub fn from_graph(graph: SeleneGraph) -> Self {
        Self::try_from_graph(graph).expect("graph store row count exceeds u32::MAX")
    }

    /// Fallible variant of [`SharedGraph::from_graph`]. Returns
    /// [`GraphError::Inconsistent`] when the graph's stores exceed the
    /// `u32` row capacity.
    pub fn try_from_graph(graph: SeleneGraph) -> GraphResult<Self> {
        Self::from_graph_with_core(graph, Vec::new())
    }

    /// Construct shared state from a graph snapshot and fixed provider list.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Provider`] when two providers declare the same
    /// [`ProviderTag`], and [`GraphError::Inconsistent`] when the graph's
    /// stores exceed the `u32` row capacity.
    pub fn from_graph_with_providers(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        Self::from_graph_with_core(graph, providers)
    }

    /// Construct shared state from a graph snapshot and commit-critical WAL file.
    ///
    /// Since v1.2 (BRIEF 2) the committer is the sole fsync caller, so the WAL is
    /// **always** opened in [`SyncPolicy::OnFlushOnly`] regardless of the
    /// `config.sync_policy` passed (it is overwritten before
    /// [`WalWriter::open`]). This non-builder constructor uses
    /// [`CommitBatching::Off`], so the committer still fsyncs once per commit —
    /// behaviorally identical to BRIEF 1's `EveryN(1)`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the WAL cannot be opened, plus the
    /// same consistency and provider-registration errors as [`Self::try_from_graph`].
    pub fn from_graph_with_wal(
        graph: SeleneGraph,
        path: impl AsRef<Path>,
        mut config: WalConfig,
    ) -> GraphResult<Self> {
        // BRIEF 2: the committer owns fsync via flush_durables(); force the
        // committer-managed WAL into OnFlushOnly before opening it (overwriting
        // any caller policy), keeping open-error timing unchanged.
        config.sync_policy = SyncPolicy::OnFlushOnly;
        let writer = WalWriter::open(path.as_ref(), config)?;
        Self::from_graph_with_core_and_durables(
            graph,
            Vec::new(),
            Vec::new(),
            Some(writer),
            None,
            CommitBatching::Off,
        )
    }

    fn from_graph_with_core(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        Self::from_graph_with_core_and_durables(
            graph,
            providers,
            Vec::new(),
            None,
            None,
            CommitBatching::Off,
        )
    }

    pub(crate) fn from_graph_with_core_and_durables(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        mut durable_providers: Vec<Arc<dyn DurableProvider>>,
        wal_writer: Option<WalWriter>,
        audit_log: Option<AuditLog>,
        batching: CommitBatching,
    ) -> GraphResult<Self> {
        if audit_log.is_some() && wal_writer.is_none() {
            return Err(GraphError::Inconsistent {
                reason: "audit log configured without a WAL; audit mirroring requires durable WAL \
                         state"
                    .to_owned(),
            });
        }
        let snapshot = Arc::new(ArcSwap::from_pointee(graph.clone()));
        let has_wal = wal_writer.is_some();
        let durable = wal_writer
            .map(DurableState::new)
            .map(|durable| match audit_log {
                Some(audit) => durable.with_audit_log(audit),
                None => durable,
            });
        let core = CoreProvider::new_for_live_with_wal(Arc::clone(&snapshot), durable);
        let mut all_providers = Vec::with_capacity(providers.len() + 1);
        all_providers.push(core.clone() as Arc<dyn IndexProvider>);
        all_providers.extend(providers);
        if has_wal {
            durable_providers.push(core as Arc<dyn DurableProvider>);
        }
        validate_unique_provider_tags(&all_providers)?;
        Self::from_graph_parts_and_snapshot(
            graph,
            all_providers,
            durable_providers,
            snapshot,
            batching,
        )
    }

    pub(crate) fn from_graph_parts_and_snapshot(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        durable_providers: Vec<Arc<dyn DurableProvider>>,
        snapshot: Arc<ArcSwap<SeleneGraph>>,
        batching: CommitBatching,
    ) -> GraphResult<Self> {
        validate_unique_provider_tags(&providers)?;
        // Freeze the registry into one shared allocation: the committer and
        // every `begin_write` transaction clone the `Arc`, not the `Vec`.
        let providers: Arc<[Arc<dyn IndexProvider>]> = providers.into();
        let mut graph = graph;
        rebuild_derived_state(&mut graph)?;
        crate::property_index::rebuild_property_indexes(&mut graph)?;
        crate::composite_property_index::rebuild_composite_property_indexes(&mut graph)?;
        crate::vector_index::rebuild_vector_indexes(&mut graph)?;
        crate::text_index::rebuild_text_indexes(&mut graph)?;
        if let Some(type_def) = graph.meta.bound_type.as_deref() {
            // Why: GraphMeta is publicly constructible, so SharedGraph::from_graph
            // can land a malformed bound_type that bypassed builder().bound_to()'s
            // validate(). Re-check self-consistency here so every constructor
            // arrives at the same closed-graph admissibility contract.
            type_def.validate_ref()?;
            crate::type_validator::validate_entity_state(&graph, type_def)?;
        }

        let node_floor = (graph.node_store.labels.len() as u64).saturating_add(1);
        let edge_floor = (graph.edge_store.label.len() as u64).saturating_add(1);
        let allocator = IdAllocator::from_meta_with_floors(&graph.meta, node_floor, edge_floor);

        // Debug-only structural net on the snapshot-load / recovery path: the
        // rebuild_* helpers above re-derive all indexes from columns, so a
        // rebuild bug would otherwise surface only as silent query
        // corruption. Highest-value placement — verify the rebuilt snapshot
        // before it is ever published. Compiled out in release builds.
        #[cfg(debug_assertions)]
        if let Err(reason) = graph.assert_indexes_consistent() {
            return Err(GraphError::Inconsistent {
                reason: format!("rebuilt snapshot failed index consistency check: {reason}"),
            });
        }

        let graph = Arc::new(graph);
        snapshot.store(Arc::clone(&graph));
        let shared = Arc::new(RwLock::new(graph));
        let schema_version = Arc::new(AtomicU64::new(0));
        let allocator = Arc::new(Mutex::new(allocator));
        // Spawn the single per-graph committer thread. It captures clones of
        // every handle it needs to publish + compact; it is the sole writer of
        // `snapshot`. All commit/compact/index-DDL publishes route through it.
        let committer =
            crate::committer::CommitterThread::spawn(crate::committer::CommitterHandles {
                snapshot: Arc::clone(&snapshot),
                schema_version: Arc::clone(&schema_version),
                providers: Arc::clone(&providers),
                durable_providers: durable_providers.clone(),
                batching,
            });
        Ok(Self {
            shared,
            snapshot,
            schema_version,
            allocator,
            providers,
            durable_providers,
            committer,
        })
    }

    /// Load the current immutable snapshot without taking the write lock.
    #[must_use]
    pub fn read(&self) -> Arc<SeleneGraph> {
        self.snapshot.load_full()
    }

    /// Return compaction pressure for the current published snapshot.
    ///
    /// This is a lock-free read of row counts and liveness counters. It does not
    /// compact, rebuild indexes, or take the writer lock.
    #[must_use]
    pub fn compaction_stats(&self) -> crate::compaction::CompactionStats {
        self.read().compaction_stats()
    }

    /// Compact the live graph in place: reclaim every dead / hole row, renumber
    /// rows dense, and atomically republish the result so the RAM held by deleted
    /// rows is reclaimed immediately (BRIEF-Item-4c — the live-densify half of
    /// snapshot-time compaction).
    ///
    /// This is pure space reclamation: it changes only the internal row layout,
    /// never external `NodeId`/`EdgeId`, properties, or labels, so it emits **no**
    /// [`Change`] and writes **no** WAL entry. Durability
    /// comes from the next snapshot, which encodes the now-dense live graph (the
    /// CORE provider reads the same `snapshot` cell this method publishes into). A
    /// crash before that snapshot simply reloads the pre-compaction state and
    /// recompacts later — compaction can never lose data.
    ///
    /// The dense graph is built under the write lock on the calling thread
    /// (seal-and-handover, exactly like a commit), and is allocated a publish
    /// `seal_seq` under that same lock; the single committer then swaps it into
    /// the published `snapshot` cell strictly in `seal_seq` order. So compaction
    /// serializes with writers exactly like a commit and can never be reordered
    /// ahead of an earlier-sealed commit (which would let that commit's stale,
    /// non-dense frozen snapshot clobber the dense one). Lock-free readers keep
    /// observing the old snapshot until the dense graph is published. The
    /// monotonic allocator high-water marks are preserved (the live allocator is
    /// untouched, and [`compact_core`](crate::compact_core) carries `GraphMeta`
    /// verbatim — and the allocator is kept in sync with `GraphMeta` on every
    /// commit), so no external id is ever reused after a later recovery.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError`] if the graph's id↔row mapping is corrupt or the
    /// recompacted graph fails its consistency check (see
    /// [`compact_core`](crate::compact_core)).
    pub fn compact(&self) -> GraphResult<crate::CompactionReport> {
        // Seal-and-handover for compaction (v1.2 BRIEF 1, P1 fix): build the
        // dense graph HERE, on the caller thread, under the write lock — exactly
        // like a commit seals under the lock — then hand the committer a
        // pre-built dense snapshot to publish in seal_seq order. This keeps the
        // committer off the write lock entirely (no deadlock surface) and, more
        // importantly, ties compaction's publish position to a seal_seq taken
        // under the same lock as commits, so a compact can never be reordered
        // ahead of an earlier-sealed commit (which would otherwise let an
        // earlier commit's stale, non-dense frozen snapshot clobber the dense
        // one in the published cell).
        //
        // Ordering under the lock is load-bearing for the reorder buffer's
        // gap-free invariant: densify FIRST (the only fallible step), and only
        // THEN allocate the seal_seq, so a failed compaction consumes no
        // sequence number (which would otherwise wedge the committer waiting for
        // a seq that never arrives).
        let committer = self.committer.handle();
        let (seal_seq, dense, report) = {
            let mut guard = self.shared.write();
            let compacted = crate::compaction::compact_core(&guard)?;
            let dense = Arc::new(compacted.graph);
            // Allocate the publish-order key under the lock, after the fallible
            // densify, so seal_seq order == lock-acquisition order and no seq is
            // ever burned by a failed compaction.
            let seal_seq = committer.next_seal_seq();
            *guard = Arc::clone(&dense);
            (seal_seq, dense, compacted.report)
            // Lock released here, before the (blocking) enqueue + recv — the
            // committer never needs the write lock, but releasing here also
            // means a compactor never holds the lock while blocked on the
            // committer.
        };
        committer.submit_compact(seal_seq, dense, report)
    }

    /// Rebuild every registered vector index from primary node values.
    ///
    /// HNSW indexes retain stale deleted entries after vector update/delete so
    /// in-flight search can still traverse the neighbor graph safely. This
    /// maintenance path reclaims those stale entries by rebuilding only the
    /// derived vector-index state; it does not change graph data, emit
    /// [`Change`], write a WAL entry, bump schema epoch, or
    /// notify providers. The HNSW graph is derived, not durable: snapshots and
    /// recovery persist only vector-index registrations plus primary values, so
    /// a reopen rebuilds the index from that authoritative state.
    ///
    /// The rebuild is strict on live data: if an indexed row no longer satisfies
    /// the registered vector dimension/metric invariant, this method returns an
    /// error instead of silently dropping the row from the index.
    pub fn rebuild_vector_indexes(&self) -> GraphResult<VectorIndexRebuildReport> {
        let committer = self.committer.handle();
        let (seal_seq, rebuilt, report) = {
            let mut guard = self.shared.write();
            let mut rebuilt = guard.as_ref().clone();
            let report = crate::vector_index::rebuild_vector_indexes_strict(&mut rebuilt)?;
            let rebuilt = Arc::new(rebuilt);
            let seal_seq = committer.next_seal_seq();
            *guard = Arc::clone(&rebuilt);
            (seal_seq, rebuilt, report)
        };
        committer.submit_vector_index_rebuild(seal_seq, rebuilt, report)
    }

    /// Rebuild only vector indexes whose diagnostics recommend maintenance.
    ///
    /// This is the bounded maintenance variant for IVF drift: it uses each index's current
    /// [`ivf_rebuild_recommended`](crate::vector_index::VectorIndexMemoryUsage::ivf_rebuild_recommended)
    /// value to decide whether to rebuild that derived index. Indexes that do not recommend rebuild
    /// are left untouched, and a no-op call returns an empty report without publishing a maintenance
    /// item.
    ///
    /// The rebuild is strict on live data for selected indexes, matching
    /// [`Self::rebuild_vector_indexes`].
    pub fn rebuild_recommended_vector_indexes(&self) -> GraphResult<VectorIndexRebuildReport> {
        self.maintain_vector_indexes(VectorIndexMaintenancePolicy::recommended())
    }

    /// Maintain recommended vector indexes under a caller-supplied policy.
    ///
    /// This is the explicit orchestration API for amortized vector-index maintenance. It rebuilds
    /// only indexes whose diagnostics currently recommend maintenance and applies the policy cap
    /// after ordering recommended indexes by pending IVF retrain pressure. It remains a
    /// maintenance-tier operation: reads never trigger it, and a no-op call returns an empty report
    /// without publishing a derived-state replacement.
    ///
    /// The rebuild is strict on live data for selected indexes, matching
    /// [`Self::rebuild_vector_indexes`].
    pub fn maintain_vector_indexes(
        &self,
        policy: VectorIndexMaintenancePolicy,
    ) -> GraphResult<VectorIndexRebuildReport> {
        let committer = self.committer.handle();
        let (seal_seq, rebuilt, report) = {
            let mut guard = self.shared.write();
            let mut rebuilt = guard.as_ref().clone();
            let report = crate::vector_index::maintain_vector_indexes_strict(&mut rebuilt, policy)?;
            if report.entries.is_empty() {
                return Ok(report);
            }
            let rebuilt = Arc::new(rebuilt);
            let seal_seq = committer.next_seal_seq();
            *guard = Arc::clone(&rebuilt);
            (seal_seq, rebuilt, report)
        };
        committer.submit_vector_index_rebuild(seal_seq, rebuilt, report)
    }

    /// Return the runtime schema-version epoch used for plan-cache invalidation.
    ///
    /// The epoch starts at zero for each [`SharedGraph`] instance and advances
    /// only after a successful commit whose change set contains
    /// [`Change::SchemaChanged`].
    #[must_use]
    pub fn schema_version(&self) -> u64 {
        self.schema_version.load(Ordering::Acquire)
    }

    /// Return the bound graph type, if this is a closed graph.
    #[must_use]
    pub fn graph_type(&self) -> Option<Arc<GraphTypeDef>> {
        self.read().meta.bound_type.as_ref().map(Arc::clone)
    }

    /// Return true when this graph is bound to a closed graph type.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.read().meta.bound_type.is_some()
    }

    /// Look up a registered provider by tag.
    #[must_use]
    pub fn index_provider_by_tag(&self, tag: ProviderTag) -> Option<Arc<dyn IndexProvider>> {
        self.providers
            .iter()
            .find_map(|provider| (provider.provider_tag() == tag).then(|| Arc::clone(provider)))
    }

    /// Borrow the fixed provider registry for executor procedure contexts.
    #[must_use]
    pub fn index_providers(&self) -> &[Arc<dyn IndexProvider>] {
        &self.providers
    }

    /// Borrow the fixed commit-critical durable provider registry.
    #[must_use]
    pub fn durable_providers(&self) -> &[Arc<dyn DurableProvider>] {
        &self.durable_providers
    }

    /// Begin a write transaction by acquiring the single graph write lock.
    ///
    /// Concurrent writers from other threads queue normally on the write
    /// lock; the engine does **not** panic legitimate concurrent writes
    /// during another commit's provider fanout.
    ///
    /// Since v1.2 (BRIEF 1) the actual snapshot publish happens on the single
    /// committer thread, not here: `WriteTxn::commit` seals under this lock,
    /// releases it, and hands the frozen bundle to the committer. Provider
    /// fan-out therefore now runs on the **committer thread**, so the
    /// re-entrancy guard below protects the committer thread (sound with exactly
    /// one committer — see `reentry.rs` and the v1.2 design §7.7).
    ///
    /// # Panics
    ///
    /// Panics when called from inside an [`IndexProvider`] callback **on
    /// the committer thread** as the active fanout. Re-entrant writes from a
    /// provider callback are unsupported; the committer is publishing, so a
    /// nested write would recurse indefinitely. The panic is caught by the
    /// committer's `notify_providers` boundary; provider state may drift, but
    /// the commit still completes.
    ///
    /// Cross-thread re-entry — a provider spawning a worker thread that
    /// calls `begin_write` and waiting for it — is **documented misuse**
    /// rather than a detectable footgun (the engine cannot trace causal
    /// thread ancestry). See the module docs in `reentry.rs` and the
    /// `IndexProvider` rustdoc for the contract.
    #[must_use]
    #[tracing::instrument(name = "selene.graph.begin_write", skip(self))]
    pub fn begin_write(&self) -> WriteTxn<'_> {
        if crate::reentry::in_fanout() {
            panic!(
                "selene-graph: SharedGraph::begin_write() called from within \
                 a provider fan-out callback on the committer thread; \
                 re-entrant writes from a provider callback are not supported. \
                 The committer's fan-out boundary will catch this panic; \
                 the commit succeeds, but the offending provider's \
                 chained mutation does not."
            );
        }
        WriteTxn::new(
            self.shared.write(),
            self.committer.handle(),
            self.allocator.lock(),
            Arc::clone(&self.providers),
        )
    }

    #[cfg(test)]
    pub(crate) fn locked_arc_ptr_for_test(&self) -> *const SeleneGraph {
        let guard = self.shared.read();
        Arc::as_ptr(&*guard)
    }

    /// Read the generation of the **live RwLock graph** (`*shared`), as opposed
    /// to the published `ArcSwap` snapshot. Used by divergence tests to assert
    /// the two never disagree after a failed / cancelled commit (the P0
    /// WAL-failure + cancel rollback invariants).
    #[cfg(test)]
    pub(crate) fn locked_generation_for_test(&self) -> u64 {
        self.shared.read().meta.generation
    }

    /// Submit an already-[`seal`](crate::WriteTxn::seal)ed commit straight to the
    /// committer, blocking until it is durable + visible. Test-only seam for
    /// exercising the BRIEF-117 cancellation cut-line (which has no production
    /// producer yet) without re-entering `commit_with_principal`.
    #[cfg(test)]
    pub(crate) fn submit_sealed_for_test(
        &self,
        sealed: crate::write_txn::SealedCommit,
    ) -> GraphResult<crate::CommitOutcome> {
        self.committer.handle().submit_commit(sealed)
    }

    /// Enqueue a sealed commit and return its reply receiver without waiting.
    #[cfg(test)]
    pub(crate) fn submit_sealed_async_for_test(
        &self,
        sealed: crate::write_txn::SealedCommit,
    ) -> GraphResult<std::sync::mpsc::Receiver<GraphResult<crate::CommitOutcome>>> {
        self.committer.handle().submit_commit_async_for_test(sealed)
    }
}

mod builder;
mod index_ddl;
mod rebuild;
pub use builder::SharedGraphBuilder;
pub(crate) use rebuild::{rebuild_derived_state, validate_unique_provider_tags};

#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
#[path = "shared_property_tests.rs"]
mod property_tests;
#[cfg(test)]
mod tests;
