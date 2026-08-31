use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{Change, DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, db_string};

use super::*;
use crate::{
    CANDIDATE_STATE_PROVIDER_TAG, CandidateStateSpec, GraphError, IndexProvider,
    MaintainedCandidateStateProvider, ProviderError, ProviderTag, SharedGraph, SubTag,
    TypedIndexKind, VectorCandidateSet, compact_core,
};

fn label(value: &str) -> DbString {
    db_string(value).expect("test string is valid")
}

fn node_ids(candidates: &CandidateSet<Node>, graph: &SeleneGraph) -> BTreeSet<u64> {
    candidates
        .iter_ids(graph)
        .expect("node candidates match graph")
        .map(NodeId::get)
        .collect()
}

fn edge_ids(candidates: &CandidateSet<Edge>, graph: &SeleneGraph) -> BTreeSet<u64> {
    candidates
        .iter_ids(graph)
        .expect("edge candidates match graph")
        .map(selene_core::EdgeId::get)
        .collect()
}

fn populated_graph(graph_id: u64) -> (SharedGraph, Vec<NodeId>, Vec<selene_core::EdgeId>) {
    let shared = SharedGraph::new(GraphId::new(graph_id));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let kind = label("candidate.Kind");
    let edge_kind = label("candidate.LINK");
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for _ in 0..32 {
        let node = mutator
            .create_node(LabelSet::single(kind.clone()), PropertyMap::new())
            .unwrap();
        nodes.push(node);
        edges.push(
            mutator
                .create_edge(edge_kind.clone(), node, node, PropertyMap::new())
                .unwrap(),
        );
    }
    txn.commit().unwrap();
    (shared, nodes, edges)
}

fn add_node(shared: &SharedGraph, node_label: &DbString) -> NodeId {
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    id
}

struct StubCandidateProvider {
    candidates: VectorCandidateSet,
    calls: AtomicUsize,
}

impl StubCandidateProvider {
    fn new(ids: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            candidates: VectorCandidateSet::from_nodes(ids),
            calls: AtomicUsize::new(0),
        }
    }
}

impl IndexProvider for StubCandidateProvider {
    fn provider_tag(&self) -> ProviderTag {
        ProviderTag(CANDIDATE_STATE_PROVIDER_TAG)
    }

    fn read_section(&self, _sub_tag: SubTag, _bytes: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }

    fn write_section(&self, _sub_tag: SubTag) -> Result<Vec<u8>, ProviderError> {
        Ok(Vec::new())
    }

    fn on_change(&self, _change: &Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn vector_candidate_set(
        &self,
        _name: &DbString,
        _generation: u64,
    ) -> Result<Option<VectorCandidateSet>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(self.candidates.clone()))
    }

    fn declared_sub_tags(&self) -> &[SubTag] {
        &[]
    }
}

#[test]
fn scope_validation_distinguishes_graph_generation_and_layout() {
    let first = SeleneGraph::new(GraphId::new(1));
    let candidates = first.live_node_candidates();
    let other_graph = SeleneGraph::new(GraphId::new(2));
    assert!(matches!(
        candidates.union(&other_graph.live_node_candidates()),
        Err(CandidateSetError::GraphMismatch { .. })
    ));

    let independent = SeleneGraph::new(GraphId::new(1));
    assert!(matches!(
        candidates.union(&independent.live_node_candidates()),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));

    let mut newer = first.clone();
    newer.meta.generation = 1;
    assert!(first.layout.same_as(&newer.layout));
    assert!(first.runtime_lineage.same_as(&newer.runtime_lineage));
    assert!(matches!(
        candidates.union(&newer.live_node_candidates()),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
}

#[test]
fn same_scope_algebra_and_stable_id_access_succeed() {
    let (shared, nodes, edges) = populated_graph(3);
    let graph = shared.read();
    let left = CandidateSet::from_node_rows(&graph, RoaringBitmap::from_iter([0, 1, 2]));
    let right = CandidateSet::from_node_rows(&graph, RoaringBitmap::from_iter([2, 3]));
    assert_eq!(
        left.union(&right)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        nodes[..4]
    );
    assert_eq!(
        left.intersection(&right)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [nodes[2]]
    );
    assert_eq!(
        left.difference(&right)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        nodes[..2]
    );
    assert!(left.contains_id(&graph, nodes[1]).unwrap());
    assert!(!left.contains_id(&graph, nodes[4]).unwrap());
    let edge_candidates = CandidateSet::from_edge_rows(&graph, RoaringBitmap::from_iter([0, 2]));
    assert_eq!(
        edge_candidates
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [edges[0], edges[2]]
    );
}

#[test]
fn typed_label_property_and_composite_producers_bind_stable_ids() {
    let person = label("candidate.Person");
    let knows = label("candidate.KNOWS");
    let age = label("age");
    let name = label("name");
    let weight = label("weight");
    let edge_name = label("edge_name");
    let composite = smallvec::smallvec![age.clone(), name.clone()];
    let shared = SharedGraph::new(GraphId::new(30));
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let ada = mutator
        .create_node(
            LabelSet::single(person.clone()),
            PropertyMap::from_pairs([
                (age.clone(), Value::Int(30)),
                (name.clone(), Value::String(label("Ada"))),
            ])
            .unwrap(),
        )
        .unwrap();
    let bob = mutator
        .create_node(
            LabelSet::single(person.clone()),
            PropertyMap::from_pairs([
                (age.clone(), Value::Int(40)),
                (name.clone(), Value::String(label("Bob"))),
            ])
            .unwrap(),
        )
        .unwrap();
    let edge = mutator
        .create_edge(
            knows.clone(),
            ada,
            bob,
            PropertyMap::from_pairs([
                (weight.clone(), Value::Int(2)),
                (edge_name.clone(), Value::String(label("Ada knows Bob"))),
            ])
            .unwrap(),
        )
        .unwrap();
    mutator
        .create_property_index(person.clone(), age.clone(), TypedIndexKind::I64)
        .unwrap();
    mutator
        .create_property_index(person.clone(), name.clone(), TypedIndexKind::String)
        .unwrap();
    mutator
        .create_edge_property_index(knows.clone(), weight.clone(), TypedIndexKind::I64)
        .unwrap();
    mutator
        .create_edge_property_index(knows.clone(), edge_name.clone(), TypedIndexKind::String)
        .unwrap();
    mutator
        .create_composite_property_index_named(
            person.clone(),
            composite.clone(),
            smallvec::smallvec![TypedIndexKind::I64, TypedIndexKind::String],
            None,
        )
        .unwrap();
    txn.commit().unwrap();

    let graph = shared.read();
    assert_eq!(graph.live_node_candidates().len(), 2);
    assert_eq!(graph.live_edge_candidates().len(), 1);
    assert_eq!(
        graph
            .node_property_any_candidates(&person, &age, &[Value::Int(30), Value::Int(40)])
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [ada, bob]
    );
    assert_eq!(
        graph
            .node_property_range_candidates(&person, &age, Value::Int(35)..)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [bob]
    );
    assert_eq!(
        graph
            .node_property_prefix_candidates(&person, &name, "Ad")
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [ada]
    );
    assert_eq!(
        graph
            .edge_property_range_candidates(&knows, &weight, Value::Int(2)..=Value::Int(2))
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [edge]
    );
    assert_eq!(
        graph
            .edge_property_prefix_candidates(&knows, &edge_name, "Ada")
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [edge]
    );
    let entry = graph
        .composite_property_index_entry_for(&person, &composite)
        .unwrap();
    let values = [Value::Int(30), Value::String(label("Ada"))];
    let key = entry
        .index
        .key_from_values(&[&values[0], &values[1]])
        .unwrap();
    assert_eq!(
        graph
            .node_composite_property_candidates(&person, &composite, &key)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        [ada]
    );
}

#[test]
fn clone_publication_and_derived_rebuild_preserve_both_identities() {
    let (shared, nodes, _) = populated_graph(4);
    let before = shared.read();
    let cloned = before.as_ref().clone();
    let candidates = before.live_node_candidates();
    candidates.validate_scope(&cloned).unwrap();
    assert!(before.runtime_lineage.same_as(&cloned.runtime_lineage));
    assert!(candidates.contains_id(&before, nodes[0]).unwrap());

    add_node(&shared, &label("candidate.Kind"));
    let published = shared.read();
    assert!(before.layout.same_as(&published.layout));
    assert!(before.runtime_lineage.same_as(&published.runtime_lineage));
    assert!(matches!(
        candidates.validate_scope(&published),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));

    let generation = published.meta.generation;
    shared.rebuild_vector_indexes().unwrap();
    let rebuilt = shared.read();
    assert_eq!(rebuilt.meta.generation, generation);
    assert!(published.layout.same_as(&rebuilt.layout));
    assert!(published.runtime_lineage.same_as(&rebuilt.runtime_lineage));
}

#[test]
fn shared_attachment_remints_runtime_and_layout() {
    let graph = SeleneGraph::new(GraphId::new(5));
    let original = graph.clone();
    let candidates = graph.live_node_candidates();
    let shared = SharedGraph::try_from_graph(graph).unwrap();
    let attached = shared.read();
    assert!(!original.layout.same_as(&attached.layout));
    assert!(!original.runtime_lineage.same_as(&attached.runtime_lineage));
    assert!(matches!(
        candidates.validate_scope(&attached),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
}

#[test]
fn compact_core_remints_layout_and_preserves_runtime_and_ids() {
    let (shared, nodes, _) = populated_graph(6);
    let mut txn = shared.begin_write();
    txn.mutator().delete_node(nodes[1]).unwrap();
    txn.commit().unwrap();
    let before = shared.read();
    let old = before.live_node_candidates();
    let expected = old.iter_ids(&before).unwrap().collect::<Vec<_>>();
    let compacted = compact_core(&before).unwrap().graph;
    assert_eq!(before.meta.generation, compacted.meta.generation);
    assert!(before.runtime_lineage.same_as(&compacted.runtime_lineage));
    assert!(!before.layout.same_as(&compacted.layout));
    assert!(matches!(
        old.validate_scope(&compacted),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
    assert_eq!(
        compacted
            .live_node_candidates()
            .iter_ids(&compacted)
            .unwrap()
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn recovery_remints_runtime_and_rejects_old_pinned_snapshot() {
    let graph_id = GraphId::new(7);
    let dir = std::env::temp_dir().join(format!(
        "selene-candidate-runtime-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let durable = SharedGraph::builder(graph_id)
        .with_wal(
            dir.join(crate::DEFAULT_WAL_FILE_NAME),
            crate::WalConfig::default(),
        )
        .unwrap()
        .build()
        .unwrap();
    add_node(&durable, &label("candidate.Recovered"));
    let old = durable.read();
    let candidates = old.live_node_candidates();
    let generation = old.meta.generation;
    drop(durable);
    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let current = recovered.read();
    assert_eq!(current.meta.generation, generation);
    assert!(!old.runtime_lineage.same_as(&current.runtime_lineage));
    assert!(matches!(
        candidates.validate_scope(&current),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
    assert!(matches!(
        recovered.node_candidate_set(&label("active"), &old),
        Err(ProviderError::Inconsistent { .. })
    ));
    drop(current);
    drop(recovered);
    drop(old);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn candidate_held_layout_identity_is_not_reused() {
    let graph_id = GraphId::new(8);
    let graph = SeleneGraph::new(graph_id);
    let candidates = graph.live_node_candidates();
    drop(graph);
    let replacement = SeleneGraph::new(graph_id);
    assert!(matches!(
        candidates.validate_scope(&replacement),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
}

#[test]
fn maintained_binding_survives_compaction_but_not_publication() {
    let person = label("candidate.Person");
    let state_name = label("active");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([
            CandidateStateSpec::new(state_name.clone()).require_label(person.clone())
        ])
        .unwrap(),
    );
    let shared = SharedGraph::builder(GraphId::new(9))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let ada = add_node(&shared, &person);
    let before = shared.read();
    let old = shared
        .node_candidate_set(&state_name, &before)
        .unwrap()
        .unwrap();
    shared.compact().unwrap();
    let after = shared.read();
    let fresh = shared
        .node_candidate_set(&state_name, &after)
        .unwrap()
        .unwrap();
    assert_eq!(old.iter_ids(&before).unwrap().collect::<Vec<_>>(), [ada]);
    assert_eq!(fresh.iter_ids(&after).unwrap().collect::<Vec<_>>(), [ada]);
    assert!(before.runtime_lineage.same_as(&after.runtime_lineage));
    assert!(matches!(
        old.validate_scope(&after),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
    shared.begin_write().commit().unwrap();
    let error = shared.node_candidate_set(&state_name, &after).unwrap_err();
    let ProviderError::Inconsistent { reason } = error else {
        panic!("expected provider generation mismatch")
    };
    assert!(reason.contains("generation"));
}

#[test]
fn pinned_maintained_set_remains_usable_after_later_publication() {
    let person = label("candidate.Person");
    let state_name = label("active");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([
            CandidateStateSpec::new(state_name.clone()).require_label(person.clone())
        ])
        .unwrap(),
    );
    let shared = SharedGraph::builder(GraphId::new(31))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let ada = add_node(&shared, &person);
    let pinned = shared.read();
    let candidates = shared
        .node_candidate_set(&state_name, &pinned)
        .unwrap()
        .unwrap();

    shared.begin_write().commit().unwrap();
    let newer = shared.read();
    assert_eq!(
        candidates.iter_ids(&pinned).unwrap().collect::<Vec<_>>(),
        [ada]
    );
    assert!(matches!(
        candidates.validate_scope(&newer),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
    let error = shared.node_candidate_set(&state_name, &pinned).unwrap_err();
    let ProviderError::Inconsistent { reason } = error else {
        panic!("expected provider generation mismatch")
    };
    assert!(reason.contains("generation"));
}

#[test]
fn foreign_pinned_graphs_reject_before_provider_output() {
    let node_label = label("candidate.Foreign");
    let provider = Arc::new(StubCandidateProvider::new([NodeId::new(1)]));
    let shared = SharedGraph::builder(GraphId::new(10))
        .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    add_node(&shared, &node_label);
    let same_id = SharedGraph::new(GraphId::new(10));
    add_node(&same_id, &node_label);
    let different_id = SharedGraph::new(GraphId::new(11));
    add_node(&different_id, &node_label);
    assert!(matches!(
        shared.node_candidate_set(&label("stub"), &different_id.read()),
        Err(ProviderError::Inconsistent { .. })
    ));
    assert!(matches!(
        shared.node_candidate_set(&label("stub"), &same_id.read()),
        Err(ProviderError::Inconsistent { .. })
    ));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn provider_binding_rejects_tombstone_absent_and_dead_ids() {
    for (graph_id, id) in [(12, NodeId::TOMBSTONE), (13, NodeId::new(999))] {
        let provider = Arc::new(StubCandidateProvider::new([id]));
        let shared = SharedGraph::builder(GraphId::new(graph_id))
            .with_provider(provider as Arc<dyn IndexProvider>)
            .build()
            .unwrap();
        assert!(matches!(
            shared.node_candidate_set(&label("stub"), &shared.read()),
            Err(ProviderError::Inconsistent { .. })
        ));
    }

    let provider = Arc::new(StubCandidateProvider::new([NodeId::new(1)]));
    let shared = SharedGraph::builder(GraphId::new(14))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let id = add_node(&shared, &label("candidate.Dead"));
    let mut txn = shared.begin_write();
    txn.mutator().delete_node(id).unwrap();
    txn.commit().unwrap();
    assert!(matches!(
        shared.node_candidate_set(&label("stub"), &shared.read()),
        Err(ProviderError::Inconsistent { .. })
    ));
}

#[test]
fn maintained_provider_lease_requires_all_old_runtime_owners_to_drop() {
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([CandidateStateSpec::new(label("active"))]).unwrap(),
    );
    let first = SharedGraph::builder(GraphId::new(15))
        .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
    let pinned = first.read();
    provider.attach_runtime(&pinned).unwrap();
    let simultaneous = SharedGraph::builder(GraphId::new(16))
        .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
        .build();
    assert!(matches!(
        simultaneous,
        Err(GraphError::Provider(ProviderError::Inconsistent { .. }))
    ));
    drop(first);
    let pinned_still_live = SharedGraph::builder(GraphId::new(16))
        .with_provider(Arc::clone(&provider) as Arc<dyn IndexProvider>)
        .build();
    assert!(matches!(
        pinned_still_live,
        Err(GraphError::Provider(ProviderError::Inconsistent { .. }))
    ));
    drop(pinned);
    SharedGraph::builder(GraphId::new(16))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn node_and_edge_algebra_matches_stable_id_sets(
        left in proptest::collection::btree_set(0_u8..32, 0..32),
        right in proptest::collection::btree_set(0_u8..32, 0..32),
    ) {
        let (shared, nodes, edges) = populated_graph(20);
        let graph = shared.read();
        let left_rows = RoaringBitmap::from_iter(left.iter().map(|row| u32::from(*row)));
        let right_rows = RoaringBitmap::from_iter(right.iter().map(|row| u32::from(*row)));
        let node_left = CandidateSet::from_node_rows(&graph, left_rows.clone());
        let node_right = CandidateSet::from_node_rows(&graph, right_rows.clone());
        let edge_left = CandidateSet::from_edge_rows(&graph, left_rows);
        let edge_right = CandidateSet::from_edge_rows(&graph, right_rows);
        let nl = left.iter().map(|row| nodes[*row as usize].get()).collect::<BTreeSet<_>>();
        let nr = right.iter().map(|row| nodes[*row as usize].get()).collect::<BTreeSet<_>>();
        let el = left.iter().map(|row| edges[*row as usize].get()).collect::<BTreeSet<_>>();
        let er = right.iter().map(|row| edges[*row as usize].get()).collect::<BTreeSet<_>>();

        prop_assert_eq!(node_ids(&node_left.union(&node_right).unwrap(), &graph), nl.union(&nr).copied().collect());
        prop_assert_eq!(node_ids(&node_left.intersection(&node_right).unwrap(), &graph), nl.intersection(&nr).copied().collect());
        prop_assert_eq!(node_ids(&node_left.difference(&node_right).unwrap(), &graph), nl.difference(&nr).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.union(&edge_right).unwrap(), &graph), el.union(&er).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.intersection(&edge_right).unwrap(), &graph), el.intersection(&er).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.difference(&edge_right).unwrap(), &graph), el.difference(&er).copied().collect());
    }
}
