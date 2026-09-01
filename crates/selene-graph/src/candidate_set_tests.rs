use std::collections::BTreeSet;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use proptest::prelude::*;
use selene_core::{EdgeId, GraphId, LabelSet, NodeId, PropertyMap, db_string};

use crate::candidate_set::{CandidateSet, Edge, Node};
use crate::store::{EdgeRow, NodeRow};
use crate::{CandidateSetError, SeleneGraph, SharedGraph, WalConfig};

fn identity_graph(width: u32) -> SeleneGraph {
    let mut graph = SeleneGraph::new(GraphId::new(41));
    graph.meta.generation = 7;
    for raw in 0..width {
        let id = NodeId::new(u64::from(raw) + 101);
        let row = NodeRow::new(raw);
        graph.node_store.labels.push(LabelSet::new());
        graph.node_store.properties.push(PropertyMap::new());
        graph.node_store.row_to_id.push(id);
        graph.node_store.mark_alive(row);
        graph.node_rows.insert_cow(id, row);
        graph.node_id_to_row.insert_cow(id, row.lower_row_bridge());
    }
    for raw in 0..width {
        let id = EdgeId::new(u64::from(raw) + 201);
        let row = EdgeRow::new(raw);
        graph
            .edge_store
            .label
            .push(db_string("candidate.edge").unwrap());
        graph.edge_store.source.push(NodeId::new(101));
        graph
            .edge_store
            .target
            .push(NodeId::new(u64::from(raw) + 101));
        graph.edge_store.properties.push(PropertyMap::new());
        graph.edge_store.row_to_id.push(id);
        graph.edge_store.mark_alive(row);
        graph.edge_rows.insert_cow(id, row);
        graph.edge_id_to_row.insert_cow(id, row.lower_row_bridge());
    }
    graph
}

fn node_subset(graph: &SeleneGraph, ids: &BTreeSet<u32>) -> CandidateSet<Node> {
    CandidateSet::from_node_rows(
        graph,
        ids.iter().map(|raw| {
            let id = NodeId::new(u64::from(*raw) + 101);
            (id, graph.node_row_for_id(id).unwrap())
        }),
    )
}

fn edge_subset(graph: &SeleneGraph, ids: &BTreeSet<u32>) -> CandidateSet<Edge> {
    CandidateSet::from_edge_rows(
        graph,
        ids.iter().map(|raw| {
            let id = EdgeId::new(u64::from(*raw) + 201);
            (id, graph.edge_row_for_id(id).unwrap())
        }),
    )
}

proptest! {
    #[test]
    fn node_algebra_matches_btree_reference(
        left in proptest::collection::btree_set(0_u32..64, 0..64),
        right in proptest::collection::btree_set(0_u32..64, 0..64),
    ) {
        let graph = identity_graph(64);
        let left_candidates = node_subset(&graph, &left);
        let right_candidates = node_subset(&graph, &right);
        let stable = |raw: &u32| NodeId::new(u64::from(*raw) + 101);
        let expected_union = left.union(&right).map(stable).collect::<Vec<_>>();
        let expected_intersection = left.intersection(&right).map(stable).collect::<Vec<_>>();
        let expected_difference = left.difference(&right).map(stable).collect::<Vec<_>>();

        prop_assert_eq!(
            graph.union_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_union,
        );
        prop_assert_eq!(
            graph.intersect_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_intersection,
        );
        prop_assert_eq!(
            graph.difference_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_difference,
        );
    }

    #[test]
    fn edge_algebra_matches_btree_reference(
        left in proptest::collection::btree_set(0_u32..64, 0..64),
        right in proptest::collection::btree_set(0_u32..64, 0..64),
    ) {
        let graph = identity_graph(64);
        let left_candidates = edge_subset(&graph, &left);
        let right_candidates = edge_subset(&graph, &right);
        let stable = |raw: &u32| EdgeId::new(u64::from(*raw) + 201);
        let expected_union = left.union(&right).map(stable).collect::<Vec<_>>();
        let expected_intersection = left.intersection(&right).map(stable).collect::<Vec<_>>();
        let expected_difference = left.difference(&right).map(stable).collect::<Vec<_>>();

        prop_assert_eq!(
            graph.union_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_union,
        );
        prop_assert_eq!(
            graph.intersect_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_intersection,
        );
        prop_assert_eq!(
            graph.difference_candidates(&left_candidates, &right_candidates).unwrap().iter().collect::<Vec<_>>(),
            expected_difference,
        );
    }
}

#[test]
fn candidate_identity_rejects_graph_generation_and_layout_mismatch() {
    let graph = identity_graph(4);
    let nodes = graph.live_node_candidates().unwrap();

    let foreign_graph = SeleneGraph::new(GraphId::new(42));
    assert!(matches!(
        foreign_graph.union_candidates(&nodes, &nodes),
        Err(CandidateSetError::GraphMismatch { .. })
    ));

    let mut newer = graph.clone();
    newer.meta.generation += 1;
    assert_eq!(
        newer.union_candidates(&nodes, &nodes).unwrap_err(),
        CandidateSetError::GenerationMismatch {
            expected: 8,
            actual: 7,
        }
    );

    let mut independent = identity_graph(4);
    independent.meta.graph_id = graph.meta.graph_id;
    independent.meta.generation = graph.meta.generation;
    assert_eq!(
        independent.union_candidates(&nodes, &nodes).unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
}

#[test]
fn candidate_retains_non_reusable_layout_after_graph_drop() {
    let graph = identity_graph(2);
    let candidates = graph.live_node_candidates().unwrap();
    let retained = candidates.layout_weak();
    drop(graph);
    assert!(retained.upgrade().is_some());

    let independent = identity_graph(2);
    assert_eq!(
        independent
            .union_candidates(&candidates, &candidates)
            .unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
    drop(candidates);
    assert!(retained.upgrade().is_none());
}

fn populated_shared(graph_id: GraphId) -> SharedGraph {
    let shared = SharedGraph::new(graph_id);
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let second = mutator
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(
                db_string("candidate.edge").unwrap(),
                first,
                second,
                PropertyMap::new(),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    shared
}

#[test]
fn clone_and_ordinary_publication_preserve_layout_but_generation_rejects() {
    let shared = populated_shared(GraphId::new(51));
    let before = shared.read();
    let clone = before.as_ref().clone();
    let candidates = before.live_node_candidates().unwrap();
    assert!(before.shares_layout_with(&clone));
    let _ = clone.union_candidates(&candidates, &candidates).unwrap();

    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    let after = shared.read();
    assert!(before.shares_layout_with(&after));
    assert!(matches!(
        after.union_candidates(&candidates, &candidates),
        Err(CandidateSetError::GenerationMismatch { .. })
    ));
}

#[test]
fn detached_shared_attachment_remints_layout() {
    let graph = identity_graph(3);
    let candidates = graph.live_node_candidates().unwrap();
    let shared = SharedGraph::try_from_graph(graph.clone()).unwrap();
    let attached = shared.read();
    assert_eq!(attached.graph_id(), graph.graph_id());
    assert_eq!(attached.meta.generation, graph.meta.generation);
    assert!(!attached.shares_layout_with(&graph));
    assert_eq!(
        attached
            .union_candidates(&candidates, &candidates)
            .unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
}

#[test]
fn compaction_remints_without_changing_generation_or_surviving_ids() {
    let shared = populated_shared(GraphId::new(52));
    let mut txn = shared.begin_write();
    txn.mutator().delete_edge(EdgeId::new(1)).unwrap();
    txn.commit().unwrap();
    let before = shared.read();
    let candidates = before.live_node_candidates().unwrap();
    let expected_ids = candidates.iter().collect::<Vec<_>>();
    let generation = before.meta.generation;

    shared.compact().unwrap();
    let after = shared.read();
    let fresh = after.live_node_candidates().unwrap();
    assert_eq!(after.meta.generation, generation);
    assert_eq!(fresh.iter().collect::<Vec<_>>(), expected_ids);
    assert!(!before.shares_layout_with(&after));
    assert_eq!(
        after
            .union_candidates(&candidates, &candidates)
            .unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
}

#[test]
fn factory_reset_remints_before_generation_changes() {
    let shared = populated_shared(GraphId::new(53));
    let before = shared.read();
    let candidates = before.live_node_candidates().unwrap();
    let mut txn = shared.begin_write();
    txn.mutator().factory_reset().unwrap();
    assert_eq!(txn.read().meta.generation, before.meta.generation);
    assert!(!txn.read().shares_layout_with(&before));
    assert_eq!(
        txn.read()
            .union_candidates(&candidates, &candidates)
            .unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
    txn.commit().unwrap();
}

#[test]
fn non_remapping_rebuild_preserves_layout_and_generation() {
    let shared = populated_shared(GraphId::new(54));
    let before = shared.read();
    let candidates = before.live_node_candidates().unwrap();
    let generation = before.meta.generation;
    shared.rebuild_vector_indexes().unwrap();
    let after = shared.read();
    assert_eq!(after.meta.generation, generation);
    assert!(after.shares_layout_with(&before));
    let _ = after.union_candidates(&candidates, &candidates).unwrap();
}

fn recovery_dir(name: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "selene-candidate-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn recovery_remints_independent_layout_and_rebuilds_typed_maps() {
    let dir = recovery_dir("remint");
    let graph_id = GraphId::new(55);
    let shared = SharedGraph::builder(graph_id)
        .with_wal(dir.join(crate::DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    txn.mutator()
        .create_node(LabelSet::new(), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    let before = shared.read();
    let candidates = before.live_node_candidates().unwrap();
    let generation = before.meta.generation;
    drop(before);
    drop(shared);

    let recovered = SharedGraph::recover(&dir, graph_id).unwrap();
    let after = recovered.read();
    assert_eq!(after.meta.generation, generation);
    assert_eq!(
        after
            .live_node_candidates()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![NodeId::new(1)]
    );
    assert_eq!(
        after
            .union_candidates(&candidates, &candidates)
            .unwrap_err(),
        CandidateSetError::LayoutMismatch
    );
    after.assert_indexes_consistent().unwrap();
    drop(after);
    drop(recovered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_rows_drive_mapping_endpoint_delete_and_consistency_paths() {
    fn node_row_only(_: NodeRow) {}
    fn edge_row_only(_: EdgeRow) {}

    let shared = populated_shared(GraphId::new(56));
    let before = shared.read();
    let nodes = before.live_node_candidates().unwrap();
    let edges = before.live_edge_candidates().unwrap();
    assert_eq!(
        nodes.iter().collect::<Vec<_>>(),
        vec![NodeId::new(1), NodeId::new(2)]
    );
    assert_eq!(edges.iter().collect::<Vec<_>>(), vec![EdgeId::new(1)]);
    for (_, row) in nodes.trusted_rows() {
        node_row_only(row);
    }
    for (_, row) in edges.trusted_rows() {
        edge_row_only(row);
    }
    assert_eq!(
        before.edge_endpoints(EdgeId::new(1)),
        Some((NodeId::new(1), NodeId::new(2)))
    );
    drop(before);

    let mut txn = shared.begin_write();
    txn.mutator().delete_node(NodeId::new(1)).unwrap();
    txn.commit().unwrap();
    let after = shared.read();
    assert_eq!(
        after
            .live_node_candidates()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        vec![NodeId::new(2)]
    );
    assert!(after.live_edge_candidates().unwrap().is_empty());
    after.assert_indexes_consistent().unwrap();
}
