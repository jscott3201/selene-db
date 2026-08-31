use std::collections::BTreeSet;
use std::sync::Arc;

use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, db_string};

use super::*;
use crate::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, ProviderError,
    SharedGraph, TypedIndexKind, compact_core,
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
            .expect("node creation succeeds");
        nodes.push(node);
        edges.push(
            mutator
                .create_edge(edge_kind.clone(), node, node, PropertyMap::new())
                .expect("edge creation succeeds"),
        );
    }
    txn.commit().expect("fixture commit succeeds");
    (shared, nodes, edges)
}

#[test]
fn scope_validation_distinguishes_graph_generation_and_layout() {
    let first = SeleneGraph::new(GraphId::new(1));
    let candidates = first.live_node_candidates();
    let other_graph = SeleneGraph::new(GraphId::new(2));
    assert!(matches!(
        candidates.validate_scope(&other_graph),
        Err(CandidateSetError::GraphMismatch { .. })
    ));
    assert!(matches!(
        candidates.union(&other_graph.live_node_candidates()),
        Err(CandidateSetError::GraphMismatch { .. })
    ));

    let independent = SeleneGraph::new(GraphId::new(1));
    assert!(matches!(
        candidates.validate_scope(&independent),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
    assert!(matches!(
        candidates.union(&independent.live_node_candidates()),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));

    let mut newer = first.clone();
    newer.meta.generation = 1;
    assert!(first.layout.same_as(&newer.layout));
    assert!(matches!(
        candidates.validate_scope(&newer),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
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
        vec![nodes[2]]
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
        vec![edges[0], edges[2]]
    );
}

#[test]
fn clone_publication_and_derived_rebuild_preserve_layout() {
    let (shared, nodes, _) = populated_graph(4);
    let before = shared.read();
    let cloned = before.as_ref().clone();
    let candidates = before.live_node_candidates();
    candidates.validate_scope(&cloned).unwrap();
    assert_eq!(candidates.iter_ids(&before).unwrap().next(), Some(nodes[0]));
    assert!(candidates.contains_id(&before, nodes[0]).unwrap());

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(label("candidate.Kind")),
            PropertyMap::new(),
        )
        .unwrap();
    txn.commit().unwrap();
    let published = shared.read();
    assert!(before.layout.same_as(&published.layout));
    assert!(matches!(
        candidates.validate_scope(&published),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
    let fresh = published.live_node_candidates();
    assert_eq!(fresh.iter_ids(&published).unwrap().next(), Some(nodes[0]));
    assert!(fresh.contains_id(&published, nodes[0]).unwrap());

    let generation = published.meta.generation;
    shared.rebuild_vector_indexes().unwrap();
    let rebuilt = shared.read();
    assert_eq!(rebuilt.meta.generation, generation);
    assert!(published.layout.same_as(&rebuilt.layout));
    rebuilt
        .live_node_candidates()
        .validate_scope(&rebuilt)
        .unwrap();
}

#[test]
fn shared_runtime_boundary_remints_layout() {
    let graph = SeleneGraph::new(GraphId::new(5));
    let candidates = graph.live_node_candidates();
    let shared = SharedGraph::try_from_graph(graph).unwrap();
    let attached = shared.read();
    assert!(matches!(
        candidates.validate_scope(&attached),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
}

#[test]
fn compaction_remints_at_unchanged_generation_and_preserves_ids() {
    let (shared, nodes, _) = populated_graph(6);
    let mut txn = shared.begin_write();
    txn.mutator().delete_node(nodes[1]).unwrap();
    txn.commit().unwrap();
    let before = shared.read();
    let old = before.live_node_candidates();
    let expected = old.iter_ids(&before).unwrap().collect::<Vec<_>>();

    let compacted = compact_core(&before).unwrap().graph;
    assert_eq!(before.meta.generation, compacted.meta.generation);
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
    assert!(
        compacted
            .live_node_candidates()
            .contains_id(&compacted, nodes[0])
            .unwrap()
    );
}

#[test]
fn recovery_remints_layout() {
    let graph_id = GraphId::new(7);
    let dir = std::env::temp_dir().join(format!(
        "selene-candidate-layout-{}-{}",
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
    let mut txn = durable.begin_write();
    txn.mutator()
        .create_node(
            LabelSet::single(label("candidate.Recovered")),
            PropertyMap::new(),
        )
        .unwrap();
    txn.commit().unwrap();
    let graph = durable.read();
    let candidates = graph.live_node_candidates();
    let generation = graph.meta.generation;
    drop(graph);
    drop(durable);
    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    assert_eq!(recovered.read().meta.generation, generation);
    assert!(matches!(
        candidates.validate_scope(&recovered.read()),
        Err(CandidateSetError::LayoutMismatch { .. })
    ));
    drop(recovered);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn candidate_held_identity_is_not_reused_after_owner_drop() {
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
fn label_property_and_maintained_state_producers_bind_stable_ids() {
    let person = label("candidate.Person");
    let other = label("candidate.Other");
    let knows = label("candidate.KNOWS");
    let age = label("age");
    let weight = label("weight");
    let name = label("name");
    let edge_name = label("edge_name");
    let composite_properties = smallvec::smallvec![age.clone(), name.clone()];
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
    let ignored = mutator
        .create_node(LabelSet::single(other), PropertyMap::new())
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
            composite_properties.clone(),
            smallvec::smallvec![TypedIndexKind::I64, TypedIndexKind::String],
            None,
        )
        .unwrap();
    txn.commit().unwrap();

    let graph = shared.read();
    assert_eq!(
        graph
            .node_label_candidates(&person)
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![ada, bob]
    );
    assert!(
        graph
            .live_node_candidates()
            .contains_id(&graph, ignored)
            .unwrap()
    );
    assert_eq!(
        graph
            .node_property_range_candidates(&person, &age, Value::Int(35)..)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![bob]
    );
    assert_eq!(
        graph
            .node_property_prefix_candidates(&person, &name, "Ad")
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![ada]
    );
    let composite = graph
        .composite_property_index_entry_for(&person, &composite_properties)
        .unwrap();
    let values = [Value::Int(30), Value::String(label("Ada"))];
    let key = composite
        .index
        .key_from_values(&[&values[0], &values[1]])
        .unwrap();
    assert_eq!(
        graph
            .node_composite_property_candidates(&person, &composite_properties, &key)
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![ada]
    );
    assert_eq!(
        graph
            .edge_property_eq_candidates(&knows, &weight, &Value::Int(2))
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![edge]
    );
    assert_eq!(
        graph
            .edge_property_prefix_candidates(&knows, &edge_name, "Ada")
            .unwrap()
            .iter_ids(&graph)
            .unwrap()
            .collect::<Vec<_>>(),
        vec![edge]
    );
    let pinned = graph;
    let maintained = shared
        .node_candidate_set(&state_name, &pinned)
        .unwrap()
        .unwrap();
    shared.begin_write().commit().unwrap();
    let newer = shared.read();
    let observed = maintained.iter_ids(&pinned).unwrap().collect::<Vec<_>>();
    assert_eq!(observed, [ada, bob]);
    assert!(matches!(
        maintained.iter_ids(&newer),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
    let stale = shared.node_candidate_set(&state_name, &pinned);
    assert!(matches!(stale, Err(ProviderError::Inconsistent { .. })));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn node_and_edge_algebra_matches_stable_id_sets(
        left in proptest::collection::btree_set(0_u8..32, 0..32),
        right in proptest::collection::btree_set(0_u8..32, 0..32),
    ) {
        let (shared, nodes, edges) = populated_graph(10);
        let graph = shared.read();
        let left_rows = RoaringBitmap::from_iter(left.iter().map(|row| u32::from(*row)));
        let right_rows = RoaringBitmap::from_iter(right.iter().map(|row| u32::from(*row)));
        let node_left = CandidateSet::from_node_rows(&graph, left_rows.clone());
        let node_right = CandidateSet::from_node_rows(&graph, right_rows.clone());
        let edge_left = CandidateSet::from_edge_rows(&graph, left_rows);
        let edge_right = CandidateSet::from_edge_rows(&graph, right_rows);

        let node_left_ids = left.iter().map(|row| nodes[*row as usize].get()).collect::<BTreeSet<_>>();
        let node_right_ids = right.iter().map(|row| nodes[*row as usize].get()).collect::<BTreeSet<_>>();
        let edge_left_ids = left.iter().map(|row| edges[*row as usize].get()).collect::<BTreeSet<_>>();
        let edge_right_ids = right.iter().map(|row| edges[*row as usize].get()).collect::<BTreeSet<_>>();

        prop_assert_eq!(node_ids(&node_left.union(&node_right).unwrap(), &graph), node_left_ids.union(&node_right_ids).copied().collect());
        prop_assert_eq!(node_ids(&node_left.intersection(&node_right).unwrap(), &graph), node_left_ids.intersection(&node_right_ids).copied().collect());
        prop_assert_eq!(node_ids(&node_left.difference(&node_right).unwrap(), &graph), node_left_ids.difference(&node_right_ids).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.union(&edge_right).unwrap(), &graph), edge_left_ids.union(&edge_right_ids).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.intersection(&edge_right).unwrap(), &graph), edge_left_ids.intersection(&edge_right_ids).copied().collect());
        prop_assert_eq!(edge_ids(&edge_left.difference(&edge_right).unwrap(), &graph), edge_left_ids.difference(&edge_right_ids).copied().collect());
    }
}
