//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::{GraphId, IStr};

use crate::adjacency::AdjacencyEdge;
use crate::core_provider::CoreProvider;
use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::graph_types::GraphTypeDef;
use crate::id_allocator::IdAllocator;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag};
use crate::store::EdgeStore;
use crate::typed_index::TypedIndexKind;
use crate::write_txn::WriteTxn;

/// Per-graph shared runtime state.
pub struct SharedGraph {
    shared: Arc<RwLock<SeleneGraph>>,
    snapshot: Arc<ArcSwap<SeleneGraph>>,
    allocator: Arc<Mutex<IdAllocator>>,
    providers: Vec<Arc<dyn IndexProvider>>,
}

/// Builder for a [`SharedGraph`] and its fixed provider registry.
pub struct SharedGraphBuilder {
    graph: SeleneGraph,
    providers: Vec<Arc<dyn IndexProvider>>,
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

    fn from_graph_with_core(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        let snapshot = Arc::new(ArcSwap::from_pointee(graph.clone()));
        let core = CoreProvider::new_for_live(Arc::clone(&snapshot));
        let mut all_providers = Vec::with_capacity(providers.len() + 1);
        all_providers.push(core as Arc<dyn IndexProvider>);
        all_providers.extend(providers);
        validate_unique_provider_tags(&all_providers)?;
        Self::from_graph_parts_and_snapshot(graph, all_providers, snapshot)
    }

    pub(crate) fn from_graph_parts_and_snapshot(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
        snapshot: Arc<ArcSwap<SeleneGraph>>,
    ) -> GraphResult<Self> {
        validate_unique_provider_tags(&providers)?;
        let mut graph = graph;
        rebuild_derived_state(&mut graph)?;
        crate::property_index::rebuild_property_indexes(&mut graph)?;
        if let Some(type_def) = graph.meta.bound_type.as_deref() {
            crate::type_validator::validate_entity_state(&graph, type_def)?;
        }

        let node_floor = (graph.node_store.labels.len() as u64).saturating_add(1);
        let edge_floor = (graph.edge_store.label.len() as u64).saturating_add(1);
        let allocator = IdAllocator::from_meta_with_floors(&graph.meta, node_floor, edge_floor);
        snapshot.store(Arc::new(graph.clone()));
        Ok(Self {
            shared: Arc::new(RwLock::new(graph.clone())),
            snapshot,
            allocator: Arc::new(Mutex::new(allocator)),
            providers,
        })
    }

    /// Load the current immutable snapshot without taking the write lock.
    #[must_use]
    pub fn read(&self) -> Arc<SeleneGraph> {
        self.snapshot.load_full()
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
        let mut txn = self.begin_write();
        if txn.working.property_index.contains_key(&(label, property)) {
            return Err(GraphError::PropertyIndexAlreadyExists { label, property });
        }
        let index =
            crate::property_index::build_property_index(&txn.working, label, property, kind)?;
        txn.working
            .property_index
            .insert((label, property), Arc::new(index));
        txn.commit()?;
        Ok(())
    }

    /// Drop a built-in node property index.
    ///
    /// The operation is idempotent; dropping an absent index succeeds without
    /// publishing a new snapshot.
    pub fn drop_property_index(&self, label: IStr, property: IStr) -> GraphResult<()> {
        let mut txn = self.begin_write();
        if txn
            .working
            .property_index
            .remove(&(label, property))
            .is_none()
        {
            return Ok(());
        }
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
    pub fn begin_write(&self) -> WriteTxn<'_> {
        if crate::reentry::in_fanout() {
            panic!(
                "selene-graph: SharedGraph::begin_write() called from within \
                 an IndexProvider callback on the same thread; same-thread \
                 re-entrant writes are not supported in v1.0. The outer \
                 commit's notify_providers boundary will catch this panic; \
                 the outer commit succeeds, but the offending provider's \
                 chained mutation does not."
            );
        }
        WriteTxn::new(
            self.shared.write(),
            Arc::clone(&self.snapshot),
            self.allocator.lock(),
            self.providers.clone(),
        )
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
        SharedGraph::from_graph_with_core(self.graph, self.providers)
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

fn validate_unique_provider_tags(providers: &[Arc<dyn IndexProvider>]) -> GraphResult<()> {
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
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use selene_core::{Change, LabelSet, PropertyValueType, intern};
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::CORE_PROVIDER_TAG;

    struct TestProvider {
        tag: ProviderTag,
        seen: Mutex<Vec<Change>>,
    }

    impl TestProvider {
        fn new(tag: ProviderTag) -> Self {
            Self {
                tag,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl IndexProvider for TestProvider {
        fn provider_tag(&self) -> ProviderTag {
            self.tag
        }

        fn read_section(
            &self,
            _sub_tag: crate::SubTag,
            _bytes: &[u8],
        ) -> Result<(), ProviderError> {
            Ok(())
        }

        fn write_section(&self, _sub_tag: crate::SubTag) -> Result<Vec<u8>, ProviderError> {
            Ok(Vec::new())
        }

        fn on_change(&self, change: &Change) -> Result<(), ProviderError> {
            self.seen.lock().push(change.clone());
            Ok(())
        }

        fn declared_sub_tags(&self) -> &[crate::SubTag] {
            &[]
        }
    }

    fn sample_type() -> GraphTypeDef {
        GraphTypeDef {
            name: intern("shared.type").unwrap(),
            node_types: vec![crate::NodeTypeDef {
                name: intern("shared.node").unwrap(),
                key_labels: LabelSet::single(intern("SharedNode").unwrap()),
                properties: vec![crate::PropertyTypeDef {
                    name: intern("shared.name").unwrap(),
                    value_type: PropertyValueType::String,
                    required: true,
                }],
            }],
            edge_types: Vec::new(),
        }
    }

    #[test]
    fn new_initial_state_is_empty() {
        let shared = SharedGraph::new(GraphId::new(1));
        assert_eq!(shared.read().node_count(), 0);
        assert!(
            shared
                .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
                .is_some()
        );
    }

    #[test]
    fn builder_constructs_empty_graph() {
        let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
        assert_eq!(shared.read().meta.graph_id, GraphId::new(1));
        assert_eq!(
            shared.providers[0].provider_tag(),
            ProviderTag(CORE_PROVIDER_TAG)
        );
    }

    #[test]
    fn builder_bound_to_constructs_closed_graph() {
        let graph_type = sample_type();
        let shared = SharedGraph::builder(GraphId::new(1))
            .bound_to(graph_type.clone())
            .unwrap()
            .build()
            .unwrap();
        assert!(shared.is_closed());
        assert_eq!(shared.graph_type().as_deref(), Some(&graph_type));
    }

    #[test]
    fn builder_bound_to_rejects_invalid_type() {
        let mut graph_type = sample_type();
        graph_type.node_types[0].key_labels = LabelSet::new();
        assert!(matches!(
            SharedGraph::builder(GraphId::new(1)).bound_to(graph_type),
            Err(GraphError::Inconsistent { reason }) if reason.contains("empty label set")
        ));
    }

    #[test]
    fn builder_with_two_providers_preserves_registration_order() {
        let first = Arc::new(TestProvider::new(ProviderTag(*b"ONE1")));
        let second = Arc::new(TestProvider::new(ProviderTag(*b"TWO2")));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(first)
            .with_provider(second)
            .build()
            .unwrap();
        assert_eq!(
            shared.providers[0].provider_tag(),
            ProviderTag(CORE_PROVIDER_TAG)
        );
        assert_eq!(shared.providers[1].provider_tag(), ProviderTag(*b"ONE1"));
        assert_eq!(shared.providers[2].provider_tag(), ProviderTag(*b"TWO2"));
    }

    #[test]
    fn builder_rejects_duplicate_provider_tags() {
        let result = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(TestProvider::new(ProviderTag(*b"DUPL"))))
            .with_provider(Arc::new(TestProvider::new(ProviderTag(*b"DUPL"))))
            .build();
        let err = match result {
            Ok(_) => panic!("duplicate provider tags should be rejected"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            GraphError::Provider(ProviderError::Inconsistent { reason })
                if reason.contains("duplicate provider tag DUPL")
        ));
    }

    #[test]
    fn from_graph_with_providers_validates_uniqueness() {
        let graph = SeleneGraph::new(GraphId::new(1));
        let result = SharedGraph::from_graph_with_providers(
            graph,
            vec![
                Arc::new(TestProvider::new(ProviderTag(*b"SAME"))),
                Arc::new(TestProvider::new(ProviderTag(*b"SAME"))),
            ],
        );
        let err = match result {
            Ok(_) => panic!("duplicate provider tags should be rejected"),
            Err(err) => err,
        };
        assert!(matches!(
            err,
            GraphError::Provider(ProviderError::Inconsistent { .. })
        ));
    }

    #[test]
    fn public_constructors_auto_register_core_provider() {
        let from_new = SharedGraph::new(GraphId::new(1));
        let from_graph = SharedGraph::from_graph(SeleneGraph::new(GraphId::new(2)));
        assert!(
            from_new
                .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
                .is_some()
        );
        assert!(
            from_graph
                .index_provider_by_tag(ProviderTag(CORE_PROVIDER_TAG))
                .is_some()
        );
    }

    #[test]
    fn user_registered_core_tag_is_rejected() {
        let result = SharedGraph::builder(GraphId::new(1))
            .with_provider(Arc::new(TestProvider::new(ProviderTag(CORE_PROVIDER_TAG))))
            .build();
        assert!(matches!(
            result,
            Err(GraphError::Provider(ProviderError::Inconsistent { reason }))
                if reason.contains("duplicate provider tag CORE")
        ));
    }

    #[test]
    fn index_provider_by_tag_returns_registered_provider() {
        let provider = Arc::new(TestProvider::new(ProviderTag(*b"FIND")));
        let shared = SharedGraph::builder(GraphId::new(1))
            .with_provider(provider)
            .build()
            .unwrap();
        assert_eq!(
            shared
                .index_provider_by_tag(ProviderTag(*b"FIND"))
                .unwrap()
                .provider_tag(),
            ProviderTag(*b"FIND")
        );
    }

    #[test]
    fn index_provider_by_tag_returns_none_for_unknown_tag() {
        let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
        assert!(
            shared
                .index_provider_by_tag(ProviderTag(*b"MISS"))
                .is_none()
        );
    }

    #[test]
    fn read_is_lock_free_concurrent_with_another_reader() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        thread::scope(|scope| {
            for _ in 0..8 {
                let shared = Arc::clone(&shared);
                scope.spawn(move || {
                    for _ in 0..10_000 {
                        assert_eq!(shared.read().meta.graph_id, GraphId::new(1));
                    }
                });
            }
        });
    }

    #[test]
    fn from_graph_floor_derives_allocator_from_storage_when_meta_is_stale() {
        use crate::SeleneGraph;
        use selene_core::{LabelSet, PropertyMap};

        let mut graph = SeleneGraph::new(GraphId::new(1));
        graph.node_store.labels.push(LabelSet::new());
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.alive.insert(0);
        graph
            .edge_store
            .label
            .push(selene_core::intern("e").unwrap());
        graph.edge_store.source.push(selene_core::NodeId::new(1));
        graph.edge_store.target.push(selene_core::NodeId::new(1));
        graph.edge_store.properties.push(PropertyMap::new());
        graph.edge_store.alive.insert(0);
        // Stale meta: still says next is 1 even though one row of each exists.
        graph.meta.next_node_id = 1;
        graph.meta.next_edge_id = 1;

        let shared = SharedGraph::from_graph(graph);
        let mut txn = shared.begin_write();
        let id = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), PropertyMap::new())
                .expect("create_node ok")
        };
        // Storage floor (len + 1 = 2) overrode the stale meta floor of 1, so
        // the allocator returned NodeId(2), not the colliding NodeId(1).
        assert_eq!(id, selene_core::NodeId::new(2));
        txn.commit().unwrap();
    }

    #[test]
    fn from_graph_rebuilds_label_indexes_from_stores() {
        use selene_core::{LabelSet, NodeId as CoreNodeId, PropertyMap, intern};

        let label_a = intern("idx.a").unwrap();
        let label_b = intern("idx.b").unwrap();

        let mut graph = SeleneGraph::new(GraphId::new(1));
        // Row 0: alive, labels {a, b}.
        let mut labels0 = LabelSet::new();
        labels0.insert(label_a);
        labels0.insert(label_b);
        graph.node_store.labels.push(labels0);
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.alive.insert(0);
        // Row 1: dead, label {a} — must NOT appear in the rebuilt index.
        graph.node_store.labels.push(LabelSet::single(label_a));
        graph.node_store.properties.push(PropertyMap::new());
        // alive bit deliberately not set on row 1.

        // Edge row 0: alive, label.
        let edge_label = intern("idx.edge").unwrap();
        graph.edge_store.label.push(edge_label);
        graph.edge_store.source.push(CoreNodeId::new(1));
        graph.edge_store.target.push(CoreNodeId::new(1));
        graph.edge_store.properties.push(PropertyMap::new());
        graph.edge_store.alive.insert(0);

        // Caller-supplied indexes are intentionally empty / stale.
        graph.idx_label.clear();
        graph.idx_edge_label.clear();

        let shared = SharedGraph::from_graph(graph);
        let snapshot = shared.read();
        let nodes_a = snapshot.nodes_with_label(&label_a).expect("idx.a present");
        let nodes_b = snapshot.nodes_with_label(&label_b).expect("idx.b present");
        // Only row 0 (alive) appears for label a; row 1 (dead) is filtered.
        assert_eq!(nodes_a.iter().collect::<Vec<_>>(), vec![0]);
        assert_eq!(nodes_b.iter().collect::<Vec<_>>(), vec![0]);
        let edges = snapshot
            .edges_with_label(&edge_label)
            .expect("edge label present");
        assert_eq!(edges.iter().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn from_graph_rebuilds_adjacency_from_edge_store() {
        use selene_core::{LabelSet, NodeId as CoreNodeId, PropertyMap, intern};

        let edge_label = intern("idx.edge.adj").unwrap();
        let mut graph = SeleneGraph::new(GraphId::new(1));
        for label in ["adj.node.a", "adj.node.b"] {
            graph
                .node_store
                .labels
                .push(LabelSet::single(intern(label).unwrap()));
            graph.node_store.properties.push(PropertyMap::new());
        }
        graph.node_store.alive.insert(0);
        graph.node_store.alive.insert(1);
        graph.edge_store.label.push(edge_label);
        graph.edge_store.source.push(CoreNodeId::new(1));
        graph.edge_store.target.push(CoreNodeId::new(2));
        graph.edge_store.properties.push(PropertyMap::new());
        graph.edge_store.alive.insert(0);
        graph.adjacency_out.clear();
        graph.adjacency_in.clear();

        let shared = SharedGraph::from_graph(graph);
        let snapshot = shared.read();
        let outgoing = snapshot
            .outgoing_edges(CoreNodeId::new(1))
            .expect("outgoing adjacency rebuilt");
        let incoming = snapshot
            .incoming_edges(CoreNodeId::new(2))
            .expect("incoming adjacency rebuilt");
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(outgoing.iter().next().unwrap().label, edge_label);
    }

    #[test]
    fn try_from_graph_returns_ok_for_well_formed_input() {
        use selene_core::{LabelSet, PropertyMap, intern};

        let label = intern("idx.ok").unwrap();
        let mut graph = SeleneGraph::new(GraphId::new(1));
        graph.node_store.labels.push(LabelSet::single(label));
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.alive.insert(0);

        let shared =
            SharedGraph::try_from_graph(graph).expect("well-formed graph rebuilds successfully");
        let snapshot = shared.read();
        assert!(snapshot.nodes_with_label(&label).is_some());
    }

    #[test]
    fn from_graph_discards_caller_supplied_index_drift() {
        use selene_core::{LabelSet, PropertyMap, intern};

        let real_label = intern("idx.real").unwrap();
        let phantom_label = intern("idx.phantom").unwrap();

        let mut graph = SeleneGraph::new(GraphId::new(1));
        graph.node_store.labels.push(LabelSet::single(real_label));
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.alive.insert(0);

        // Caller injects a phantom index entry that doesn't match storage.
        let mut phantom_bitmap = roaring::RoaringBitmap::new();
        phantom_bitmap.insert(99);
        graph.idx_label.insert(phantom_label, phantom_bitmap);

        let shared = SharedGraph::from_graph(graph);
        let snapshot = shared.read();
        // Phantom index is rebuilt away — only the real label remains.
        assert!(snapshot.nodes_with_label(&phantom_label).is_none());
        assert!(snapshot.nodes_with_label(&real_label).is_some());
    }

    #[test]
    fn read_during_write_lock_held_does_not_block() {
        let shared = Arc::new(SharedGraph::new(GraphId::new(1)));
        thread::scope(|scope| {
            let writer_graph = Arc::clone(&shared);
            let writer = scope.spawn(move || {
                let _txn = writer_graph.begin_write();
                thread::sleep(Duration::from_millis(75));
            });
            thread::sleep(Duration::from_millis(10));
            let start = Instant::now();
            for _ in 0..4 {
                let reader_graph = Arc::clone(&shared);
                scope.spawn(move || {
                    for _ in 0..100 {
                        assert_eq!(reader_graph.read().node_count(), 0);
                    }
                });
            }
            writer.join().unwrap();
            assert!(start.elapsed() < Duration::from_millis(500));
        });
    }
}

#[cfg(test)]
#[path = "shared_property_tests.rs"]
mod property_tests;
