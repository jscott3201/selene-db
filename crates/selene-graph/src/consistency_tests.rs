//! Negative self-tests for [`SeleneGraph::assert_indexes_consistent`].
//!
//! These construct a `SeleneGraph` by hand, corrupt exactly one derived index,
//! and assert the checker catches the drift. This is the "would it catch the
//! DbString admission race" bar for the safety net itself: a checker that never
//! fails is worthless.

use roaring::RoaringBitmap;
use selene_core::{DbString, EdgeId, GraphId, LabelSet, NodeId, PropertyMap, Value, db_string};
use smallvec::smallvec;

use crate::adjacency::AdjacencyEdge;
use crate::graph::{CompositePropertyIndexEntry, PropertyIndexEntry, SeleneGraph};
use crate::{CompositeTypedIndex, SharedGraph, TypedIndex, TypedIndexKind};

fn label(name: &str) -> DbString {
    db_string(name).unwrap()
}

/// Build a tiny consistent graph: two alive nodes (one labeled `Person` with
/// `age=30`) and one edge between them. Uses the funnel so it is internally
/// consistent, then returns the published snapshot to mutate by hand.
fn consistent_graph() -> SeleneGraph {
    let shared = SharedGraph::new(GraphId::new(1));
    let person = label("consistency.person");
    let age = label("consistency.age");
    let knows = label("consistency.knows");
    let (a, b) = {
        let mut txn = shared.begin_write();
        let (a, b);
        {
            let mut mutator = txn.mutator();
            a = mutator
                .create_node(
                    LabelSet::single(person.clone()),
                    PropertyMap::from_pairs([(age, Value::Int(30))]).unwrap(),
                )
                .unwrap();
            b = mutator
                .create_node(LabelSet::single(person), PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(knows, a, b, PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
        (a, b)
    };
    let _ = (a, b);
    (*shared.read()).clone()
}

#[test]
fn consistent_graph_passes() {
    let graph = consistent_graph();
    assert!(graph.assert_indexes_consistent().is_ok());
}

#[test]
fn stray_label_bitmap_row_is_caught() {
    let mut graph = consistent_graph();
    let person = label("consistency.person");
    // Insert a row index that no alive node carries.
    graph.idx_label.entry(person).or_default().insert(999);
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("node label index"), "got: {err}");
}

#[test]
fn empty_label_bitmap_is_caught() {
    let mut graph = consistent_graph();
    let ghost = label("consistency.ghost");
    graph.idx_label.insert(ghost, RoaringBitmap::new());
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("present-but-empty"), "got: {err}");
}

#[test]
fn missing_label_bitmap_key_is_caught() {
    let mut graph = consistent_graph();
    let person = label("consistency.person");
    graph.idx_label.remove(&person);
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(
        err.contains("missing from the maintained index"),
        "got: {err}"
    );
}

#[test]
fn stray_edge_label_bitmap_row_is_caught() {
    let mut graph = consistent_graph();
    let knows = label("consistency.knows");
    graph.idx_edge_label.entry(knows).or_default().insert(777);
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("edge label index"), "got: {err}");
}

#[test]
fn drifted_property_index_is_caught() {
    let mut graph = consistent_graph();
    let person = label("consistency.person");
    let age = label("consistency.age");
    // Register a property index consistent with the data, then corrupt it.
    let index = crate::property_index::build_property_index_lenient(
        &graph,
        person.clone(),
        age.clone(),
        TypedIndexKind::I64,
    )
    .unwrap()
    .index;
    graph.property_index.insert(
        (person.clone(), age.clone()),
        PropertyIndexEntry::new(index, None),
    );
    assert!(graph.assert_indexes_consistent().is_ok());

    // Now corrupt: add a stray row to the maintained index.
    let mut corrupt = TypedIndex::new(TypedIndexKind::I64);
    corrupt.insert(&Value::Int(30), 0).unwrap();
    corrupt.insert(&Value::Int(30), 5).unwrap(); // row 5 does not exist
    graph
        .property_index
        .insert((person, age), PropertyIndexEntry::new(corrupt, None));
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("property index"), "got: {err}");
    assert!(err.contains("drifted"), "got: {err}");
}

#[test]
fn empty_property_index_bucket_is_caught() {
    use std::collections::BTreeMap;

    let mut graph = consistent_graph();
    let person = label("consistency.person");
    let age = label("consistency.age");
    // Craft an index with a leaked present-but-empty bucket. Commit-path
    // maintenance always prunes these, so a present empty bucket is a bug.
    let mut buckets: BTreeMap<i64, RoaringBitmap> = BTreeMap::new();
    let mut populated = RoaringBitmap::new();
    populated.insert(0);
    buckets.insert(30, populated);
    buckets.insert(99, RoaringBitmap::new()); // leaked empty bucket
    let leaky = TypedIndex::I64(buckets);
    graph
        .property_index
        .insert((person, age), PropertyIndexEntry::new(leaky, None));
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("present-but-empty"), "got: {err}");
}

#[test]
fn drifted_composite_index_is_caught() {
    let mut graph = consistent_graph();
    let person = label("consistency.person");
    let age = label("consistency.age");
    let name = label("consistency.name");
    let props: smallvec::SmallVec<[DbString; 4]> = smallvec![age, name];
    let kinds: smallvec::SmallVec<[TypedIndexKind; 4]> =
        smallvec![TypedIndexKind::I64, TypedIndexKind::String];
    // No node carries both age AND name, so the consistent index is empty.
    let index = crate::composite_property_index::build_composite_property_index_lenient(
        &graph,
        person.clone(),
        props.clone(),
        kinds.clone(),
    )
    .unwrap()
    .index;
    let key = crate::graph::composite_property_key(&props);
    graph.composite_property_index.insert(
        (person.clone(), key.clone()),
        CompositePropertyIndexEntry::new(index, props.clone(), None),
    );
    assert!(graph.assert_indexes_consistent().is_ok());

    // Corrupt: craft a composite index with a bogus row.
    let mut corrupt = CompositeTypedIndex::new(kinds);
    let v_age = Value::Int(30);
    let v_name = Value::String(label("consistency.bogus"));
    corrupt.insert(&[&v_age, &v_name], 0).unwrap();
    graph.composite_property_index.insert(
        (person, key),
        CompositePropertyIndexEntry::new(corrupt, props, None),
    );
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("composite index"), "got: {err}");
}

#[test]
fn dangling_adjacency_to_missing_edge_is_caught() {
    let mut graph = consistent_graph();
    let knows = label("consistency.knows");
    // Add an adjacency entry for a node with an edge_id that is not alive.
    graph
        .adjacency_out
        .entry(NodeId::new(1))
        .or_default()
        .add(AdjacencyEdge {
            label: knows,
            neighbor: NodeId::new(2),
            edge_id: EdgeId::new(900),
        });
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("adjacency"), "got: {err}");
}

#[test]
fn empty_adjacency_entry_is_caught() {
    let mut graph = consistent_graph();
    // An isolated node with a present-but-empty outgoing entry.
    graph
        .adjacency_out
        .insert(NodeId::new(2), crate::adjacency::AdjacencyEntry::new());
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("present-but-empty"), "got: {err}");
}

#[test]
fn out_of_range_alive_node_is_caught() {
    let mut graph = consistent_graph();
    graph.node_store.alive_mut().insert(500);
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("out-of-range"), "got: {err}");
}

#[test]
fn column_length_mismatch_is_caught() {
    let mut graph = consistent_graph();
    // Push a label without a matching property column entry.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label("consistency.extra")));
    let err = graph.assert_indexes_consistent().unwrap_err();
    assert!(err.contains("column length mismatch"), "got: {err}");
}
