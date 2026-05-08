//! Shared graph wrapper implementing lock-free reads and serialized writes.

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::{Mutex, RwLock};

use selene_core::GraphId;

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::id_allocator::IdAllocator;
use crate::index_provider::{IndexProvider, ProviderError, ProviderTag};
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
    #[must_use]
    pub fn from_graph(graph: SeleneGraph) -> Self {
        Self::from_graph_parts(graph, Vec::new())
    }

    /// Construct shared state from a graph snapshot and fixed provider list.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Provider`] when two providers declare the same
    /// [`ProviderTag`].
    pub fn from_graph_with_providers(
        graph: SeleneGraph,
        providers: Vec<Arc<dyn IndexProvider>>,
    ) -> GraphResult<Self> {
        validate_unique_provider_tags(&providers)?;
        Ok(Self::from_graph_parts(graph, providers))
    }

    fn from_graph_parts(graph: SeleneGraph, providers: Vec<Arc<dyn IndexProvider>>) -> Self {
        let node_floor = (graph.node_store.labels.len() as u64).saturating_add(1);
        let edge_floor = (graph.edge_store.label.len() as u64).saturating_add(1);
        let allocator = IdAllocator::from_meta_with_floors(&graph.meta, node_floor, edge_floor);
        Self {
            shared: Arc::new(RwLock::new(graph.clone())),
            snapshot: Arc::new(ArcSwap::new(Arc::new(graph))),
            allocator: Arc::new(Mutex::new(allocator)),
            providers,
        }
    }

    /// Load the current immutable snapshot without taking the write lock.
    #[must_use]
    pub fn read(&self) -> Arc<SeleneGraph> {
        self.snapshot.load_full()
    }

    /// Look up a registered provider by tag.
    #[must_use]
    pub fn index_provider_by_tag(&self, tag: ProviderTag) -> Option<Arc<dyn IndexProvider>> {
        self.providers
            .iter()
            .find(|provider| provider.provider_tag() == tag)
            .map(Arc::clone)
    }

    /// Begin a write transaction by acquiring the single graph write lock.
    #[must_use]
    pub fn begin_write(&self) -> WriteTxn<'_> {
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

    /// Build shared graph state and validate provider registration.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Provider`] when provider tags are duplicated.
    pub fn build(self) -> GraphResult<SharedGraph> {
        SharedGraph::from_graph_with_providers(self.graph, self.providers)
    }
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
    use selene_core::Change;
    use std::thread;
    use std::time::{Duration, Instant};

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

    #[test]
    fn new_initial_state_is_empty() {
        let shared = SharedGraph::new(GraphId::new(1));
        assert_eq!(shared.read().node_count(), 0);
        assert!(shared.providers.is_empty());
    }

    #[test]
    fn builder_constructs_empty_graph() {
        let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
        assert_eq!(shared.read().meta.graph_id, GraphId::new(1));
        assert!(shared.providers.is_empty());
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
        assert_eq!(shared.providers[0].provider_tag(), ProviderTag(*b"ONE1"));
        assert_eq!(shared.providers[1].provider_tag(), ProviderTag(*b"TWO2"));
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
    fn existing_constructors_remain_provider_free() {
        let from_new = SharedGraph::new(GraphId::new(1));
        let from_graph = SharedGraph::from_graph(SeleneGraph::new(GraphId::new(2)));
        assert!(from_new.providers.is_empty());
        assert!(from_graph.providers.is_empty());
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
