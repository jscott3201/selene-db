//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::{Change, GraphId, IStr, SchemaChange, SchemaPropertyIndexKind};
use selene_persist::{WalConfig, WalWriter};

use crate::adjacency::AdjacencyEdge;
use crate::change_subscriber::ChangeSubscriber;
use crate::core_provider::{CoreProvider, DurableState};
use crate::durable_provider::DurableProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::{PropertyIndexEntry, SeleneGraph};
use crate::graph_types::GraphTypeDef;
use crate::id_allocator::IdAllocator;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag};
use crate::store::EdgeStore;
use crate::typed_index::TypedIndexKind;
use crate::write_txn::WriteTxn;

/// Per-graph shared runtime state.
pub struct SharedGraph {
    shared: Arc<RwLock<Arc<SeleneGraph>>>,
    snapshot: Arc<ArcSwap<SeleneGraph>>,
    schema_version: Arc<AtomicU64>,
    allocator: Arc<Mutex<IdAllocator>>,
    providers: Vec<Arc<dyn IndexProvider>>,
    subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    durable_providers: Vec<Arc<dyn DurableProvider>>,
}

/// Builder for a [`SharedGraph`] and its fixed provider registry.
pub struct SharedGraphBuilder {
    graph: SeleneGraph,
    providers: Vec<Arc<dyn IndexProvider>>,
    subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    wal_writer: Option<WalWriter>,
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
            subscribers: Vec::new(),
            wal_writer: None,
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
        Self::from_graph_with_core(graph, Vec::new(), Vec::new())
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
        Self::from_graph_with_core(graph, providers, Vec::new())
    }

    /// Construct shared state from a graph snapshot and commit-critical WAL file.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the WAL cannot be opened, plus the
    /// same consistency and provider-registration errors as [`Self::try_from_graph`].
    pub fn from_graph_with_wal(
        graph: SeleneGraph,
        path: impl AsRef<Path>,
        config: WalConfig,
    ) -> GraphResult<Self> {
        let writer = WalWriter::open(path.as_ref(), config)?;
        Self::from_graph_with_core_and_durables(
            graph,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(writer),
        )
    }

    fn from_graph_with_core(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
    ) -> GraphResult<Self> {
        Self::from_graph_with_core_and_durables(graph, providers, subscribers, Vec::new(), None)
    }

    pub(crate) fn from_graph_with_core_and_durables(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
        mut durable_providers: Vec<Arc<dyn DurableProvider>>,
        wal_writer: Option<WalWriter>,
    ) -> GraphResult<Self> {
        validate_unique_subscriber_tags(&subscribers)?;
        validate_subscriber_tags_match_providers(&providers, &subscribers)?;
        let snapshot = Arc::new(ArcSwap::from_pointee(graph.clone()));
        let has_wal = wal_writer.is_some();
        let durable = wal_writer.map(DurableState::new);
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
            subscribers,
            durable_providers,
            snapshot,
        )
    }

    pub(crate) fn from_graph_parts_and_snapshot(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        subscribers: Vec<Arc<dyn ChangeSubscriber>>,
        durable_providers: Vec<Arc<dyn DurableProvider>>,
        snapshot: Arc<ArcSwap<SeleneGraph>>,
    ) -> GraphResult<Self> {
        validate_unique_provider_tags(&providers)?;
        validate_unique_subscriber_tags(&subscribers)?;
        validate_subscriber_tags_match_providers(&providers, &subscribers)?;
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
        let graph = Arc::new(graph);
        snapshot.store(Arc::clone(&graph));
        Ok(Self {
            shared: Arc::new(RwLock::new(graph)),
            snapshot,
            schema_version: Arc::new(AtomicU64::new(0)),
            allocator: Arc::new(Mutex::new(allocator)),
            providers,
            subscribers,
            durable_providers,
        })
    }

    /// Load the current immutable snapshot without taking the write lock.
    #[must_use]
    pub fn read(&self) -> Arc<SeleneGraph> {
        self.snapshot.load_full()
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

    /// Borrow the fixed change-subscriber registry.
    #[must_use]
    pub fn change_subscribers(&self) -> &[Arc<dyn ChangeSubscriber>] {
        &self.subscribers
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
    /// # Panics
    ///
    /// Panics when called from inside an [`IndexProvider`] callback **on
    /// the same thread** as the active fanout. Same-thread re-entrant
    /// writes are unsupported in v1.0; the outer commit holds the write
    /// lock and the fanout serializer, so a nested write would deadlock or
    /// recurse indefinitely. The panic is caught by the outer commit's
    /// `notify_providers` boundary; provider state may drift, but the
    /// outer commit still completes.
    ///
    /// Cross-thread re-entry — a provider spawning a worker thread that
    /// calls `begin_write` and waiting for it — is **documented misuse**
    /// rather than a detectable footgun (the engine cannot trace causal
    /// thread ancestry). See the module docs in `reentry.rs` and the
    /// `IndexProvider` rustdoc for the v1.0 contract.
    #[must_use]
    #[tracing::instrument(name = "selene.graph.begin_write", skip(self))]
    pub fn begin_write(&self) -> WriteTxn<'_> {
        if crate::reentry::in_fanout() {
            panic!(
                "selene-graph: SharedGraph::begin_write() called from within \
                 a provider fan-out callback on the same thread; same-thread \
                 re-entrant writes are not supported in v1.0. The outer \
                 commit's fan-out boundary will catch this panic; \
                 the outer commit succeeds, but the offending provider's \
                 chained mutation does not."
            );
        }
        WriteTxn::new(
            self.shared.write(),
            Arc::clone(&self.snapshot),
            Arc::clone(&self.schema_version),
            self.allocator.lock(),
            self.providers.clone(),
            self.subscribers.clone(),
            self.durable_providers.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn locked_arc_ptr_for_test(&self) -> *const SeleneGraph {
        let guard = self.shared.read();
        Arc::as_ptr(&*guard)
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

    /// Register a change subscriber.
    ///
    /// Subscribers are retained in registration order. Validation runs at
    /// [`Self::build`] so callers may add a subscriber before its matching
    /// provider.
    #[must_use]
    pub fn with_change_subscriber(mut self, subscriber: Arc<dyn ChangeSubscriber>) -> Self {
        self.subscribers.push(subscriber);
        self
    }

    /// Open a WAL file and route commits through the CORE durable provider.
    ///
    /// The path is the WAL file path, not a directory. Callers using the
    /// conventional layout should pass `dir.join(selene_persist::DEFAULT_WAL_FILE_NAME)`.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Persist`] when the WAL cannot be opened, including
    /// when another writer already holds the file lock.
    pub fn with_wal(mut self, path: impl AsRef<Path>, config: WalConfig) -> GraphResult<Self> {
        self.wal_writer = Some(WalWriter::open(path.as_ref(), config)?);
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
    /// Returns [`GraphError::Provider`] when provider tags are duplicated,
    /// subscriber tags are duplicated, or a subscriber has no matching provider.
    pub fn build(self) -> GraphResult<SharedGraph> {
        SharedGraph::from_graph_with_core_and_durables(
            self.graph,
            self.providers,
            self.subscribers,
            Vec::new(),
            self.wal_writer,
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

fn rebuild_derived_state(graph: &mut SeleneGraph) -> GraphResult<()> {
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
    rebuild_adjacency(graph)?;
    Ok(())
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
        graph
            .adjacency_out
            .entry(source)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: target,
                edge_id: selene_core::EdgeId::new(row as u64 + 1),
            });
        graph
            .adjacency_in
            .entry(target)
            .or_default()
            .add(AdjacencyEdge {
                label,
                neighbor: source,
                edge_id: selene_core::EdgeId::new(row as u64 + 1),
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

pub(crate) fn validate_unique_subscriber_tags(
    subscribers: &[Arc<dyn ChangeSubscriber>],
) -> GraphResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for subscriber in subscribers {
        let tag = subscriber.subscriber_tag();
        if !seen.insert(tag) {
            return Err(GraphError::Provider(ProviderError::Inconsistent {
                reason: format!("duplicate change subscriber tag {tag}"),
            }));
        }
    }
    Ok(())
}

pub(crate) fn validate_subscriber_tags_match_providers(
    providers: &[Arc<dyn IndexProvider>],
    subscribers: &[Arc<dyn ChangeSubscriber>],
) -> GraphResult<()> {
    let provider_tags = providers
        .iter()
        .map(|provider| provider.provider_tag())
        .collect::<std::collections::BTreeSet<_>>();
    for subscriber in subscribers {
        let tag = subscriber.subscriber_tag();
        if !provider_tags.contains(&tag) {
            return Err(GraphError::Provider(ProviderError::Inconsistent {
                reason: format!("change subscriber tag {tag} has no matching provider"),
            }));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "shared_property_tests.rs"]
mod property_tests;
#[cfg(test)]
mod tests;
