use super::*;
use parking_lot::Mutex;
use selene_core::{
    Change, HlcTimestamp, LabelSet, PropertyMap, PropertyValueType, SchemaChange, db_string,
};
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

    fn read_section(&self, _sub_tag: crate::SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
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

struct FailingDurableProvider;

impl DurableProvider for FailingDurableProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(*b"FAIL")
    }

    fn write_commit(
        &self,
        _principal: Option<&Arc<[u8]>>,
        _changes: &[Change],
        _timestamp: HlcTimestamp,
    ) -> Result<u64, ProviderError> {
        Err(ProviderError::Inconsistent {
            reason: "synthetic durable failure".to_owned(),
        })
    }
}

fn sample_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("shared.type").unwrap(),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("shared.node").unwrap(),
            key_labels: LabelSet::single(db_string("SharedNode").unwrap()),
            properties: vec![crate::PropertyTypeDef {
                name: db_string("shared.name").unwrap(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,
                unique: false,
                decimal_type: None,
                byte_string_type: None,
                record_field_types: None,
            }],
            validation_mode: crate::ValidationMode::Strict,
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
fn shared_graph_schema_version_initial_zero() {
    let shared = SharedGraph::new(GraphId::new(101));
    assert_eq!(shared.schema_version(), 0);
}

#[test]
fn schema_changed_commit_bumps_version() {
    let graph_type = GraphTypeDef {
        name: db_string("schema.version.type").unwrap(),
        node_types: Vec::new(),
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(102))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let label = db_string("SchemaVersioned").unwrap();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node_type(
            label.clone(),
            LabelSet::single(label),
            Vec::new(),
            crate::ValidationMode::Strict,
        )
        .expect("schema mutation succeeds");

    txn.commit().expect("schema commit succeeds");

    assert_eq!(shared.schema_version(), 1);
}

#[test]
fn data_changed_commit_does_not_bump_version() {
    let shared = SharedGraph::new(GraphId::new(103));
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(db_string("schema.version.data").unwrap()),
            PropertyMap::new(),
        )
        .expect("data mutation succeeds");

    txn.commit().expect("data commit succeeds");

    assert_eq!(shared.schema_version(), 0);
}

#[test]
fn direct_create_property_index_bumps_schema_version() {
    let shared = SharedGraph::new(GraphId::new(105));
    shared
        .create_property_index(
            db_string("Person").unwrap(),
            db_string("age").unwrap(),
            TypedIndexKind::I64,
        )
        .expect("create index succeeds");

    assert_eq!(shared.schema_version(), 1);
}

#[test]
fn direct_drop_property_index_bumps_schema_version_when_present() {
    let shared = SharedGraph::new(GraphId::new(106));
    let label = db_string("Person").unwrap();
    let property = db_string("age").unwrap();
    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .expect("create index succeeds");

    shared
        .drop_property_index(label, property)
        .expect("drop index succeeds");

    assert_eq!(shared.schema_version(), 2);
}

#[test]
fn direct_drop_property_index_idempotent_does_not_bump() {
    let shared = SharedGraph::new(GraphId::new(107));
    shared
        .drop_property_index(db_string("Person").unwrap(), db_string("age").unwrap())
        .expect("absent drop succeeds");

    assert_eq!(shared.schema_version(), 0);
}

#[test]
fn schema_version_bump_implies_snapshot_already_reflects_change() {
    // GRAPH-12: store-before-schema-bump ordering. `publish_appended` stores the
    // new snapshot via `snapshot.store(..)` and only THEN bumps `schema_version`
    // (`fetch_add` strictly after the store). So once the bumped epoch is
    // observable, the bumping commit's snapshot MUST already be observable too —
    // a planner that re-plans on `schema_version()==N+1` can never load a
    // pre-change snapshot. Reverse ordering (bump-before-store) would let this
    // assertion observe epoch 1 with the index still absent.
    let shared = SharedGraph::new(GraphId::new(120));
    let label = db_string("Order").unwrap();
    let property = db_string("age").unwrap();
    assert_eq!(shared.schema_version(), 0);
    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_none()
    );

    shared
        .create_property_index(label.clone(), property.clone(), TypedIndexKind::I64)
        .expect("index create");

    // The schema epoch advanced AND the snapshot that caused it is the published
    // one — the index is present in the same snapshot whose publish bumped the
    // epoch. (publish_appended: store, then fetch_add — never the reverse.)
    assert_eq!(shared.schema_version(), 1);
    assert!(
        shared
            .read()
            .property_index_for(&label, &property)
            .is_some(),
        "schema_version()==1 must imply the bumping commit's snapshot is visible",
    );
}

#[test]
fn concurrent_reader_never_sees_bumped_epoch_without_the_change() {
    // GRAPH-12 reader-loop: while a writer performs a sequence of schema-bumping
    // index creations, a reader spins reading (schema_version, snapshot) and
    // asserts the store-before-bump invariant on every observation: the number of
    // property indexes present in the snapshot is always >= the bumped epoch it
    // co-observes is consistent with. Concretely, each create_property_index bumps
    // the epoch by exactly 1 AND adds exactly 1 index, both published in one
    // store, so an observer that reads epoch E must see at least E indexes in the
    // SAME snapshot it loads right after. A bump-before-store regression would let
    // the reader catch epoch E with < E indexes present.
    let shared = Arc::new(SharedGraph::new(GraphId::new(121)));
    let label = db_string("ConcurrentOrder").unwrap();
    const CREATES: u64 = 32;

    thread::scope(|scope| {
        let reader_graph = Arc::clone(&shared);
        let reader = scope.spawn(move || {
            for _ in 0..20_000 {
                // Load the epoch FIRST, then the snapshot: if the epoch were
                // bumped before the store, this ordering would expose a snapshot
                // missing the change for the observed epoch.
                let epoch = reader_graph.schema_version();
                let snapshot = reader_graph.read();
                let present = snapshot.property_index_count() as u64;
                assert!(
                    present >= epoch,
                    "observed epoch {epoch} but snapshot has only {present} indexes \
                     — store-before-schema-bump violated",
                );
            }
        });
        for i in 0..CREATES {
            shared
                .create_property_index(
                    label.clone(),
                    db_string(format!("prop.{i}").as_str()).unwrap(),
                    TypedIndexKind::I64,
                )
                .expect("index create");
        }
        reader.join().unwrap();
    });

    assert_eq!(shared.schema_version(), CREATES);
    assert_eq!(shared.read().property_index_count() as u64, CREATES);
}

#[test]
fn failed_commit_does_not_bump_schema_version() {
    let durable: Arc<dyn DurableProvider> = Arc::new(FailingDurableProvider);
    let shared = SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(108)),
        Vec::new(),
        vec![durable],
        None,
        None,
        crate::committer_batch::CommitBatching::Off,
    )
    .unwrap();
    let mut txn = shared.begin_write();
    txn.mutator().schema_change(
        GraphId::new(108),
        SchemaChange::GraphCreated {
            id: GraphId::new(109),
            name: db_string("failed.schema.commit").unwrap(),
            graph_type: None,
        },
    );

    assert!(matches!(
        txn.commit(),
        Err(GraphError::Durable { reason }) if reason.contains("synthetic durable failure")
    ));
    assert_eq!(shared.schema_version(), 0);
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
fn durable_write_failure_rolls_back_in_memory_state() {
    let durable: Arc<dyn DurableProvider> = Arc::new(FailingDurableProvider);
    let shared = SharedGraph::from_graph_with_core_and_durables(
        SeleneGraph::new(GraphId::new(1)),
        Vec::new(),
        vec![durable],
        None,
        None,
        crate::committer_batch::CommitBatching::Off,
    )
    .unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string("durable.rollback").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
    }

    assert!(matches!(
        txn.commit(),
        Err(GraphError::Durable { reason }) if reason.contains("synthetic durable failure")
    ));
    assert_eq!(shared.read().node_count(), 0);
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
        .push(selene_core::db_string("e").unwrap());
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
    use selene_core::{LabelSet, NodeId as CoreNodeId, PropertyMap, db_string};

    let label_a = db_string("idx.a").unwrap();
    let label_b = db_string("idx.b").unwrap();

    let mut graph = SeleneGraph::new(GraphId::new(1));
    // Row 0: alive, labels {a, b}.
    let mut labels0 = LabelSet::new();
    labels0.insert(label_a.clone());
    labels0.insert(label_b.clone());
    graph.node_store.labels.push(labels0);
    graph.node_store.properties.push(PropertyMap::new());
    graph.node_store.alive.insert(0);
    // Row 1: dead, label {a} — must NOT appear in the rebuilt index.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label_a.clone()));
    graph.node_store.properties.push(PropertyMap::new());
    // alive bit deliberately not set on row 1.

    // Edge row 0: alive, label.
    let edge_label = db_string("idx.edge").unwrap();
    graph.edge_store.label.push(edge_label.clone());
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
    use selene_core::{LabelSet, NodeId as CoreNodeId, PropertyMap, db_string};

    let edge_label = db_string("idx.edge.adj").unwrap();
    let mut graph = SeleneGraph::new(GraphId::new(1));
    for label in ["adj.node.a", "adj.node.b"] {
        graph
            .node_store
            .labels
            .push(LabelSet::single(db_string(label).unwrap()));
        graph.node_store.properties.push(PropertyMap::new());
    }
    graph.node_store.alive.insert(0);
    graph.node_store.alive.insert(1);
    graph.edge_store.label.push(edge_label.clone());
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
    use selene_core::{LabelSet, PropertyMap, db_string};

    let label = db_string("idx.ok").unwrap();
    let mut graph = SeleneGraph::new(GraphId::new(1));
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(PropertyMap::new());
    graph.node_store.alive.insert(0);

    let shared =
        SharedGraph::try_from_graph(graph).expect("well-formed graph rebuilds successfully");
    let snapshot = shared.read();
    assert!(snapshot.nodes_with_label(&label).is_some());
}

#[test]
fn from_graph_discards_caller_supplied_index_drift() {
    use selene_core::{LabelSet, PropertyMap, db_string};

    let real_label = db_string("idx.real").unwrap();
    let phantom_label = db_string("idx.phantom").unwrap();

    let mut graph = SeleneGraph::new(GraphId::new(1));
    graph
        .node_store
        .labels
        .push(LabelSet::single(real_label.clone()));
    graph.node_store.properties.push(PropertyMap::new());
    graph.node_store.alive.insert(0);

    // Caller injects a phantom index entry that doesn't match storage.
    let mut phantom_bitmap = roaring::RoaringBitmap::new();
    phantom_bitmap.insert(99);
    graph
        .idx_label
        .insert(phantom_label.clone(), phantom_bitmap);

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
