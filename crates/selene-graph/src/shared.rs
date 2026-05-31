//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::{Change, GraphId, IStr, SchemaChange, SchemaPropertyIndexKind};
use selene_persist::{AuditLog, SyncPolicy, WalConfig, WalWriter};

use crate::adjacency::AdjacencyEdge;
use crate::committer_batch::CommitBatching;
use crate::core_provider::{CoreProvider, DurableState};
use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::{PropertyIndexEntry, SeleneGraph};
use crate::graph_types::GraphTypeDef;
use crate::id_allocator::IdAllocator;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag};
use crate::store::{EdgeStore, RowIndex};
use crate::typed_index::TypedIndexKind;
use crate::write_txn::WriteTxn;

/// Per-graph shared runtime state.
///
/// Since v1.2 (BRIEF 1) every snapshot publish is funneled through a single
/// per-graph committer thread ([`crate::committer::CommitterThread`]), which is
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
    providers: Vec<Arc<dyn IndexProvider>>,
    durable_providers: Vec<Arc<dyn DurableProvider>>,
    /// The single per-graph committer thread; sole publisher of `snapshot`.
    /// Dropped last via [`SharedGraph`]'s implicit drop order, which joins the
    /// thread once every outstanding [`WriteTxn`] submit handle is gone.
    committer: crate::committer::CommitterThread,
}

/// Builder for a [`SharedGraph`] and its fixed provider registry.
pub struct SharedGraphBuilder {
    graph: SeleneGraph,
    providers: Vec<Arc<dyn IndexProvider>>,
    wal_writer: Option<WalWriter>,
    audit_log: Option<AuditLog>,
    commit_batching: CommitBatching,
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
        SharedGraphBuilder {
            graph: SeleneGraph::new(graph_id),
            providers: Vec::new(),
            wal_writer: None,
            audit_log: None,
            commit_batching: CommitBatching::Off,
        }
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
        let mut graph = graph;
        rebuild_derived_state(&mut graph)?;
        crate::property_index::rebuild_property_indexes(&mut graph)?;
        crate::composite_property_index::rebuild_composite_property_indexes(&mut graph)?;
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
                providers: providers.clone(),
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

    /// Compact the live graph in place: reclaim every dead / hole row, renumber
    /// rows dense, and atomically republish the result so the RAM held by deleted
    /// rows is reclaimed immediately (BRIEF-Item-4c — the live-densify half of
    /// snapshot-time compaction).
    ///
    /// This is pure space reclamation: it changes only the internal row layout,
    /// never external `NodeId`/`EdgeId`, properties, or labels, so it emits **no**
    /// [`Change`](selene_core::Change) and writes **no** WAL entry. Durability
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
            .find(|provider| provider.provider_tag() == tag)
            .map(Arc::clone)
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

    /// Register a built-in node property index for `(label, property)`.
    ///
    /// The current node columns are scanned under the write lock and the
    /// published snapshot is updated in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::PropertyIndexAlreadyExists`] if the pair is
    /// already registered, or [`GraphError::IndexValueRejected`] if any
    /// existing node with `label` has a non-null value that does not match
    /// `kind`.
    pub fn create_property_index(
        &self,
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
    ) -> GraphResult<()> {
        self.create_property_index_named(label, property, kind, None)
    }

    /// Register a built-in node property index with optional catalog name.
    pub fn create_property_index_named(
        &self,
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
        name: Option<IStr>,
    ) -> GraphResult<()> {
        let mut txn = self.begin_write();
        if txn.read().property_index.contains_key(&(label, property)) {
            return Err(GraphError::PropertyIndexAlreadyExists { label, property });
        }
        let index = crate::property_index::build_property_index(txn.read(), label, property, kind)?;
        txn.guard_mut()
            .property_index
            .insert((label, property), PropertyIndexEntry::new(index, name));
        let graph = txn.read().graph_id();
        txn.changes.push(Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexCreatedNamed {
                label,
                property,
                kind: schema_kind_from(kind),
                name,
            },
        });
        txn.commit()?;
        Ok(())
    }

    /// Drop a built-in node property index.
    ///
    /// The operation is idempotent; dropping an absent index succeeds without
    /// publishing a new snapshot.
    pub fn drop_property_index(&self, label: IStr, property: IStr) -> GraphResult<()> {
        let mut txn = self.begin_write();
        if !txn.read().property_index.contains_key(&(label, property)) {
            return Ok(());
        }
        txn.guard_mut().property_index.remove(&(label, property));
        let graph = txn.read().graph_id();
        txn.changes.push(Change::SchemaChanged {
            graph,
            change: SchemaChange::PropertyIndexDropped { label, property },
        });
        txn.commit()?;
        Ok(())
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
            self.providers.clone(),
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
}

impl SharedGraphBuilder {
    /// Register an index provider.
    ///
    /// Providers are retained in registration order, which is the order used
    /// for committed mutation delivery.
    #[must_use]
    pub fn with_provider(mut self, provider: Arc<dyn IndexProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Open a WAL file and route commits through the CORE durable provider.
    ///
    /// The path is the WAL file path, not a directory. Callers using the
    /// conventional layout should pass `dir.join(selene_persist::DEFAULT_WAL_FILE_NAME)`.
    ///
    /// # SyncPolicy is OVERRIDDEN (v1.2 BRIEF 2 — read this)
    ///
    /// The single per-graph committer thread is the **sole fsync caller** for the
    /// committer-managed WAL: it appends a contiguous run of commits with fsync
    /// deferred, then issues exactly one [`WalWriter::flush`] per run (the R1
    /// fsync-before-publish barrier). To make that the *only* fsync path, this
    /// method **forces `config.sync_policy` to [`SyncPolicy::OnFlushOnly`]**
    /// before opening the WAL — **whatever policy you pass is discarded.** The
    /// fsync cadence is instead controlled by [`Self::with_commit_batching`]:
    /// [`CommitBatching::Off`] (the default) fsyncs once per commit (behaviorally
    /// identical to the old `EveryN(1)`), and [`CommitBatching::On`] coalesces a
    /// contiguous run into one fsync. `config.snapshot_seq` is passed through
    /// verbatim. Durability is unchanged: the committer always flushes before it
    /// publishes or acks, so a commit is durable before it is ever visible.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the WAL cannot be opened, including
    /// when another writer already holds the file lock.
    pub fn with_wal(mut self, path: impl AsRef<Path>, mut config: WalConfig) -> GraphResult<Self> {
        // BRIEF 2: the committer owns fsync. Force OnFlushOnly before opening so
        // the committer's group flush is the single durability barrier. Done
        // before WalWriter::open so open-error timing (e.g. WriterLockHeld) is
        // unchanged for existing .unwrap() call sites.
        config.sync_policy = SyncPolicy::OnFlushOnly;
        self.wal_writer = Some(WalWriter::open(path.as_ref(), config)?);
        Ok(self)
    }

    /// Set the group-commit batching policy for the committer-managed WAL
    /// (v1.2 BRIEF 2). Default [`CommitBatching::Off`].
    ///
    /// With [`CommitBatching::Off`] the committer fsyncs once per commit
    /// (behaviorally identical to BRIEF 1). With [`CommitBatching::On`] it
    /// coalesces up to `max_commits` (capped by aggregate `max_bytes`) contiguous
    /// commits into one fsync — higher throughput + lower tail latency under
    /// fan-in, at the cost of grouping several commits behind one barrier (all
    /// still durable before any of them is acked or published). Has no effect
    /// without [`Self::with_wal`] (no durable provider to flush).
    #[must_use]
    pub fn with_commit_batching(mut self, batching: CommitBatching) -> Self {
        self.commit_batching = batching;
        self
    }

    /// Attach a durable audit log at `path` (conventionally
    /// `dir.join(selene_persist::DEFAULT_AUDIT_FILE_NAME)`).
    ///
    /// Engine-owned audit events committed through this graph are mirrored to
    /// the audit log so they survive WAL-archive pruning (Item 7 / Seam D, D24).
    /// Requires [`Self::with_wal`]: audit mirroring is part of the durable
    /// commit path, so [`Self::build`] errors if an audit log is configured
    /// without a WAL.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the audit log cannot be opened.
    pub fn with_audit_log(mut self, path: impl AsRef<Path>) -> GraphResult<Self> {
        self.audit_log = Some(AuditLog::open(path.as_ref()).map_err(GraphError::Persist)?);
        Ok(self)
    }

    /// Bind this graph to `type_def` at construction time.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the builder is already bound
    /// or when `type_def` fails self-consistency validation.
    pub fn bound_to(mut self, type_def: GraphTypeDef) -> GraphResult<Self> {
        if self.graph.meta.bound_type.is_some() {
            return Err(GraphError::Inconsistent {
                reason: "graph builder is already bound to a graph type".to_owned(),
            });
        }
        self.graph.meta.bound_type = Some(Arc::new(type_def.validate()?));
        Ok(self)
    }

    /// Build shared graph state and validate provider registration.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Provider`] when provider tags are duplicated.
    pub fn build(self) -> GraphResult<SharedGraph> {
        SharedGraph::from_graph_with_core_and_durables(
            self.graph,
            self.providers,
            Vec::new(),
            self.wal_writer,
            self.audit_log,
            self.commit_batching,
        )
    }
}

const fn schema_kind_from(kind: TypedIndexKind) -> SchemaPropertyIndexKind {
    match kind {
        TypedIndexKind::I64 => SchemaPropertyIndexKind::I64,
        TypedIndexKind::F64 => SchemaPropertyIndexKind::F64,
        TypedIndexKind::String => SchemaPropertyIndexKind::String,
        TypedIndexKind::Date => SchemaPropertyIndexKind::Date,
        TypedIndexKind::LocalDateTime => SchemaPropertyIndexKind::LocalDateTime,
        TypedIndexKind::Uuid => SchemaPropertyIndexKind::Uuid,
    }
}

pub(crate) fn rebuild_derived_state(graph: &mut SeleneGraph) -> GraphResult<()> {
    graph.idx_label.clear();
    graph.idx_edge_label.clear();
    graph.adjacency_out.clear();
    graph.adjacency_in.clear();

    let node_count = graph.node_store.labels.len();
    for row_index in 0..node_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "node store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.node_store.is_alive(row) {
            continue;
        }
        if let Some(labels) = graph.node_store.labels.get(row_index) {
            for label in labels.iter() {
                graph.idx_label.entry(*label).or_default().insert(row);
            }
        }
    }

    let edge_count = graph.edge_store.label.len();
    for row_index in 0..edge_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "edge store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.edge_store.is_alive(row) {
            continue;
        }
        if let Some(label) = graph.edge_store.label.get(row_index) {
            graph.idx_edge_label.entry(*label).or_default().insert(row);
        }
    }
    // BRIEF-Item-4a: bind external ids to rows BEFORE rebuild_adjacency, which
    // (from Increment 3) reads the edge id from the row_to_id column.
    rebuild_id_maps(graph)?;
    rebuild_adjacency(graph)?;
    Ok(())
}

/// Rebuild the external-id <-> [`RowIndex`] maps from the per-store `row_to_id`
/// columns, as the final id-binding step of a full rebuild (recovery /
/// [`SharedGraph`] construction).
///
/// This is the id-map authority for the recovery path — the live commit path
/// populates the maps directly in the mutator and never reaches here. The
/// `row_to_id` column is the persisted source of truth: recovery's
/// `insert_*_row` writes the real external id for every materialized row (alive
/// **and** dead — the snapshot persists dead rows so a deleted id stays
/// resolvable to its dead row -> `NotAlive`, matching the live path) and the
/// tombstone for never-committed aborted-tx holes. Seeding the maps from the
/// column therefore preserves the live/recovery id-map equality exactly; only
/// holes are skipped (-> `None` -> `NotFound`, the accepted refinement).
///
/// For an externally-built graph that never populated `row_to_id` (e.g. a test
/// that pushes store columns directly), an alive row whose column slot is still
/// the tombstone falls back to the `row + 1` identity binding. That fallback is
/// the only `row + 1` arithmetic here and is allowlisted in the BRIEF-Item-4a
/// STEP-8 grep-gate; BRIEF-Item-4b drops it once every construction path
/// persists ids.
fn rebuild_id_maps(graph: &mut SeleneGraph) -> GraphResult<()> {
    graph.node_id_to_row.clear();
    graph.edge_id_to_row.clear();
    // Externally-built graphs may not have populated row_to_id; pad it to the
    // row-column length with tombstones so every materialized row is in-bounds.
    let node_len = graph.node_store.len();
    let edge_len = graph.edge_store.len();
    pad_row_to_id(
        &mut graph.node_store.row_to_id,
        node_len,
        selene_core::NodeId::TOMBSTONE,
    );
    pad_row_to_id(
        &mut graph.edge_store.row_to_id,
        edge_len,
        selene_core::EdgeId::TOMBSTONE,
    );

    for row in 0..node_len {
        let raw = row as u32;
        let mut id = graph
            .node_store
            .row_to_id
            .get(row)
            .copied()
            .unwrap_or(selene_core::NodeId::TOMBSTONE);
        if id == selene_core::NodeId::TOMBSTONE {
            // Hole (never committed) -> stays out of the map. An alive row with
            // no persisted id is an externally-built graph; fall back to the
            // identity binding and repair the column.
            if !graph.node_store.is_alive(raw) {
                continue;
            }
            id = selene_core::NodeId::new(u64::from(raw) + 1); // rowid-arith-ok: 4a identity bootstrap (externally-built graph); 4b reads the persisted id
            graph.node_store.row_to_id.set(row, id);
        }
        graph.node_id_to_row.insert(id, RowIndex::new(raw));
    }
    for row in 0..edge_len {
        let raw = row as u32;
        let mut id = graph
            .edge_store
            .row_to_id
            .get(row)
            .copied()
            .unwrap_or(selene_core::EdgeId::TOMBSTONE);
        if id == selene_core::EdgeId::TOMBSTONE {
            if !graph.edge_store.is_alive(raw) {
                continue;
            }
            id = selene_core::EdgeId::new(u64::from(raw) + 1); // rowid-arith-ok: 4a identity bootstrap (externally-built graph); 4b reads the persisted id
            graph.edge_store.row_to_id.set(row, id);
        }
        graph.edge_id_to_row.insert(id, RowIndex::new(raw));
    }
    Ok(())
}

/// Grow `column` with `tombstone` until it is at least `target_len` long.
///
/// [`crate::chunked_vec::ChunkedVec`] supports only push/set, so this pads but
/// never shrinks. In practice the recovery insert path already keeps the column
/// at row-column length (this is then a no-op), while an externally-built graph
/// is padded from empty.
fn pad_row_to_id<T: Clone>(
    column: &mut crate::chunked_vec::ChunkedVec<T>,
    target_len: usize,
    tombstone: T,
) {
    while column.len() < target_len {
        column.push(tombstone.clone());
    }
}

fn rebuild_adjacency(graph: &mut SeleneGraph) -> GraphResult<()> {
    let edge_count = graph.edge_store.len();
    for row_index in 0..edge_count {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "edge store row index {row_index} exceeds u32::MAX; selene-graph \
                 caps rows at u32::MAX",
            ),
        })?;
        if !graph.edge_store.is_alive(row) {
            continue;
        }
        let (label, source, target) = edge_row_parts(&graph.edge_store, row_index)?;
        // rebuild_id_maps ran first, so the edge id is read from the row_to_id
        // column (the persistence-stable id), never synthesized as row + 1.
        let edge_id =
            graph
                .edge_id_for_row(RowIndex::new(row))
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "alive edge row {row} has no mapped external id during rebuild"
                    ),
                })?;
        graph
            .adjacency_out
            .entry(source)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: target,
                edge_id,
            });
        graph
            .adjacency_in
            .entry(target)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: source,
                edge_id,
            });
    }
    Ok(())
}

fn edge_row_parts(
    store: &EdgeStore,
    row_index: usize,
) -> GraphResult<(IStr, selene_core::NodeId, selene_core::NodeId)> {
    let label = *store
        .label
        .get(row_index)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge label column missing row {row_index}"),
        })?;
    let source = *store
        .source
        .get(row_index)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge source column missing row {row_index}"),
        })?;
    let target = *store
        .target
        .get(row_index)
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!("edge target column missing row {row_index}"),
        })?;
    Ok((label, source, target))
}

pub(crate) fn validate_unique_provider_tags(
    providers: &[Arc<dyn IndexProvider>],
) -> GraphResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for provider in providers {
        let tag = provider.provider_tag();
        if !seen.insert(tag) {
            return Err(GraphError::Provider(ProviderError::Inconsistent {
                reason: format!("duplicate provider tag {tag}"),
            }));
        }
    }
    Ok(())
}

#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
#[path = "shared_property_tests.rs"]
mod property_tests;
#[cfg(test)]
mod tests;
