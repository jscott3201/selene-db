//! `selene.verify` integrity-check unit tests.
//!
//! Ported verbatim from `selene-pack/src/builtin/verify/tests.rs`. The only
//! change is the interner helper: the runtime-path DoS guard
//! (`tests/dos_guard.rs::no_unbudgeted_intern_call_in_selene_gql`) forbids the
//! bare interner-admission shorthand in any `src/` file, so the helper uses the
//! budget-aware `intern_with_admission` instead.

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern_with_admission};
use selene_graph::{SeleneGraph, SharedGraph, TypedIndex};

use super::verify_snapshot;
use crate::ProcedureResult;

fn istr(value: &str) -> IStr {
    intern_with_admission(value).expect("test string interns").0
}

fn graph_with_one_indexed_node() -> SeleneGraph {
    let graph = SharedGraph::new(GraphId::new(121_301));
    let label = istr("Person");
    let age = istr("age");
    let mut props = PropertyMap::new();
    props.set(age, Value::Int(42)).unwrap();
    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(LabelSet::single(label), props)
        .expect("node created");
    txn.mutator()
        .create_property_index(label, age, selene_graph::TypedIndexKind::I64)
        .expect("index created");
    txn.commit().expect("seed commit succeeds");
    graph.read().as_ref().clone()
}

fn graph_with_one_composite_indexed_node() -> SeleneGraph {
    let graph = SharedGraph::new(GraphId::new(121_303));
    let label = istr("Sensor");
    let ts = istr("ts");
    let location = istr("location");
    let mut props = PropertyMap::new();
    props.set(ts, Value::Int(42)).unwrap();
    props.set(location, Value::String(istr("north"))).unwrap();
    let mut txn = graph.begin_write();
    txn.mutator()
        .create_node(LabelSet::single(label), props)
        .expect("node created");
    txn.mutator()
        .create_composite_property_index_named(
            label,
            [ts, location].into_iter().collect(),
            [
                selene_graph::TypedIndexKind::I64,
                selene_graph::TypedIndexKind::String,
            ]
            .into_iter()
            .collect(),
            None,
        )
        .expect("composite index created");
    txn.commit().expect("seed commit succeeds");
    graph.read().as_ref().clone()
}

fn graph_with_one_edge() -> SeleneGraph {
    let graph = SharedGraph::new(GraphId::new(121_302));
    let label = istr("Person");
    let edge_label = istr("KNOWS");
    let mut txn = graph.begin_write();
    let source = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .expect("source created");
    let target = txn
        .mutator()
        .create_node(LabelSet::single(label), PropertyMap::new())
        .expect("target created");
    txn.mutator()
        .create_edge(edge_label, source, target, PropertyMap::new())
        .expect("edge created");
    txn.commit().expect("seed commit succeeds");
    graph.read().as_ref().clone()
}

fn status_for(result: &ProcedureResult, check: &str) -> String {
    let row = result
        .rows
        .iter()
        .find(|row| matches!(&row[0], Value::String(name) if name.as_str() == check))
        .unwrap_or_else(|| panic!("{check} row exists"));
    let Value::String(status) = &row[1] else {
        panic!("{check} status is a string");
    };
    status.as_str().to_owned()
}

#[test]
fn corrupted_label_bitmap_reports_inconsistent_row_without_rebuild() {
    let mut graph = graph_with_one_indexed_node();
    graph
        .idx_label
        .get_mut(&istr("Person"))
        .expect("label index exists")
        .insert(10);

    let result = verify_snapshot(&graph, false).expect("verification rows");
    assert_eq!(
        status_for(&result, "label_index_cardinality"),
        "inconsistent"
    );
}

#[test]
fn property_index_coverage_reports_extra_live_row_bucket() {
    let mut graph = graph_with_one_indexed_node();
    let label = istr("Person");
    let age = istr("age");
    let entry = graph
        .property_index
        .get_mut(&(label, age))
        .expect("property index exists");
    let index = Arc::make_mut(&mut entry.index);
    let TypedIndex::I64(map) = index else {
        panic!("test index is i64");
    };
    map.entry(99).or_default().insert(0);

    let result = verify_snapshot(&graph, false).expect("verification rows");

    assert_eq!(
        status_for(&result, "property_index_coverage"),
        "inconsistent"
    );
}

#[test]
fn property_index_coverage_accepts_composite_index() {
    let graph = graph_with_one_composite_indexed_node();

    let result = verify_snapshot(&graph, false).expect("verification rows");

    assert_eq!(status_for(&result, "property_index_coverage"), "ok");
}

#[test]
fn property_index_coverage_reports_extra_composite_row_bucket() {
    let mut graph = graph_with_one_composite_indexed_node();
    let entry = graph
        .composite_property_index
        .values_mut()
        .next()
        .expect("composite index exists");
    let index = Arc::make_mut(&mut entry.index);
    let wrong_ts = Value::Int(99);
    let north = Value::String(istr("north"));
    index.insert(&[&wrong_ts, &north], 0).unwrap();

    let result = verify_snapshot(&graph, false).expect("verification rows");

    assert_eq!(
        status_for(&result, "property_index_coverage"),
        "inconsistent"
    );
}

#[test]
fn adjacency_symmetry_reports_live_edge_missing_from_both_maps() {
    let mut graph = graph_with_one_edge();
    graph.adjacency_out.clear();
    graph.adjacency_in.clear();

    let result = verify_snapshot(&graph, false).expect("verification rows");

    assert_eq!(status_for(&result, "adjacency_symmetry"), "inconsistent");
}

#[test]
fn adjacency_symmetry_reports_label_drift_in_both_maps() {
    let mut graph = graph_with_one_edge();
    let wrong_label = istr("LIKES");
    let source = NodeId::new(1);
    let target = NodeId::new(2);
    graph
        .adjacency_out
        .get_mut(&source)
        .expect("source adjacency exists")
        .edges[0]
        .label = wrong_label;
    graph
        .adjacency_in
        .get_mut(&target)
        .expect("target adjacency exists")
        .edges[0]
        .label = wrong_label;

    let result = verify_snapshot(&graph, false).expect("verification rows");

    assert_eq!(status_for(&result, "adjacency_symmetry"), "inconsistent");
}

#[test]
fn typed_index_value_range_reports_live_row_in_wrong_bucket() {
    let mut graph = graph_with_one_indexed_node();
    let label = istr("Person");
    let age = istr("age");
    let entry = graph
        .property_index
        .get_mut(&(label, age))
        .expect("property index exists");
    let index = Arc::make_mut(&mut entry.index);
    let TypedIndex::I64(map) = index else {
        panic!("test index is i64");
    };
    map.entry(99).or_default().insert(0);

    let result = verify_snapshot(&graph, true).expect("verification rows");

    assert_eq!(
        status_for(&result, "typed_index_value_range"),
        "inconsistent"
    );
}

#[test]
fn deep_check_reports_stale_property_index_bitmap_row() {
    let mut graph = graph_with_one_indexed_node();
    let label = istr("Person");
    let age = istr("age");
    let entry = graph
        .property_index
        .get_mut(&(label, age))
        .expect("property index exists");
    let index = Arc::make_mut(&mut entry.index);
    let TypedIndex::I64(map) = index else {
        panic!("test index is i64");
    };
    map.entry(99).or_default().insert(99);

    let result = verify_snapshot(&graph, true).expect("verification rows");
    assert_eq!(
        status_for(&result, "roaring_bitmap_density"),
        "inconsistent"
    );
}

#[test]
fn deep_check_reports_stale_composite_index_bitmap_row() {
    let mut graph = graph_with_one_composite_indexed_node();
    let entry = graph
        .composite_property_index
        .values_mut()
        .next()
        .expect("composite index exists");
    let index = Arc::make_mut(&mut entry.index);
    let valid_ts = Value::Int(42);
    let north = Value::String(istr("north"));
    index.insert(&[&valid_ts, &north], 99).unwrap();

    let result = verify_snapshot(&graph, true).expect("verification rows");
    assert_eq!(
        status_for(&result, "roaring_bitmap_density"),
        "inconsistent"
    );
}
