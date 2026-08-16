//! `iter_property_index_stats` must make a demoted index visible, and must not
//! claim demotion for a healthy one.

use selene_core::{GraphId, LabelSet, PropertyMap, Value, db_string};

use crate::typed_index::TypedIndexKind;
use crate::{IndexedEntity, SharedGraph};

fn props(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(db_string(name).unwrap(), value)]).unwrap()
}

fn insert_node(graph: &SharedGraph, label: &str, property: &str, value: Value) {
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string(label).unwrap()),
                props(property, value),
            )
            .unwrap();
    }
    txn.commit().unwrap();
}

/// An `I64` index over an int column, then one float write. The index can no
/// longer key every live row, so it declines every probe — the exact state
/// #1102 says an operator could not see.
fn drifted_node_graph(id: u64) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(id));
    insert_node(&graph, "Reading", "level", Value::Int(3));
    graph
        .create_property_index(
            db_string("Reading").unwrap(),
            db_string("level").unwrap(),
            TypedIndexKind::I64,
        )
        .expect("the index builds over the clean column");
    insert_node(&graph, "Reading", "level", Value::Float(3.0));
    graph
}

#[test]
fn a_demoted_node_index_reports_its_drift() {
    let graph = drifted_node_graph(102_001);
    let snapshot = graph.read();
    let rows = snapshot.iter_property_index_stats().collect::<Vec<_>>();

    assert_eq!(rows.len(), 1, "one registration, one row");
    let row = &rows[0];
    assert_eq!(row.entity, IndexedEntity::Node);
    assert_eq!(row.label.as_str(), "Reading");
    assert_eq!(
        row.properties
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["level"]
    );
    assert_eq!(row.kinds, vec![TypedIndexKind::I64]);
    assert_eq!(row.drifted_rows, 1, "the float write is the drifted row");
    assert!(
        !row.answers_probes(),
        "any drift at all demotes the index, which is the point of reporting it"
    );
}

/// The counterpart, so the report discriminates instead of firing on every
/// registered index.
#[test]
fn a_healthy_node_index_reports_no_drift() {
    let graph = SharedGraph::new(GraphId::new(102_002));
    insert_node(&graph, "Reading", "level", Value::Int(3));
    graph
        .create_property_index(
            db_string("Reading").unwrap(),
            db_string("level").unwrap(),
            TypedIndexKind::I64,
        )
        .unwrap();
    insert_node(&graph, "Reading", "level", Value::Int(4));

    let snapshot = graph.read();
    let rows = snapshot.iter_property_index_stats().collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].drifted_rows, 0);
    assert!(rows[0].answers_probes());
    assert_eq!(
        rows[0].indexed_rows, 2,
        "both rows are keyed, so both are answerable"
    );
}

/// Edge indexes carry their own `drifted_rows`, and surfacing node drift while
/// hiding edge drift would recreate the blind spot this closes.
#[test]
fn an_edge_index_is_reported_and_tagged_as_an_edge() {
    let graph = SharedGraph::new(GraphId::new(102_003));
    let (source, target) = {
        let mut txn = graph.begin_write();
        let ids = {
            let mut mutator = txn.mutator();
            let source = mutator
                .create_node(
                    LabelSet::single(db_string("Person").unwrap()),
                    PropertyMap::new(),
                )
                .unwrap();
            let target = mutator
                .create_node(
                    LabelSet::single(db_string("Person").unwrap()),
                    PropertyMap::new(),
                )
                .unwrap();
            (source, target)
        };
        txn.commit().unwrap();
        ids
    };
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_edge(
                    db_string("KNOWS").unwrap(),
                    source,
                    target,
                    props("since", Value::Int(2020)),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    graph
        .create_edge_property_index(
            db_string("KNOWS").unwrap(),
            db_string("since").unwrap(),
            TypedIndexKind::I64,
        )
        .unwrap();

    let snapshot = graph.read();
    let rows = snapshot.iter_property_index_stats().collect::<Vec<_>>();
    let edge_rows = rows
        .iter()
        .filter(|row| row.entity == IndexedEntity::Edge)
        .collect::<Vec<_>>();
    assert_eq!(edge_rows.len(), 1, "the edge index must be reported");
    assert_eq!(edge_rows[0].label.as_str(), "KNOWS");
    assert!(edge_rows[0].answers_probes());
}

/// An index-free graph reports nothing rather than an empty-but-present row.
#[test]
fn a_graph_with_no_property_indexes_reports_nothing() {
    let graph = SharedGraph::new(GraphId::new(102_004));
    insert_node(&graph, "Reading", "level", Value::Int(3));
    let snapshot = graph.read();
    assert_eq!(snapshot.iter_property_index_stats().count(), 0);
}

/// Composite indexes are the third family carrying `drifted_rows`. A walker
/// that surfaced node and edge drift while skipping composites would recreate
/// exactly the blind spot this closes, so the composite path is pinned
/// separately rather than assumed to follow from the others.
#[test]
fn a_composite_index_is_reported_with_all_its_properties() {
    use smallvec::smallvec;

    let graph = SharedGraph::new(GraphId::new(102_005));
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(db_string("Reading").unwrap()),
                    PropertyMap::from_pairs([
                        (db_string("level").unwrap(), Value::Int(3)),
                        (db_string("site").unwrap(), Value::Int(7)),
                    ])
                    .unwrap(),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_composite_property_index_named(
                    db_string("Reading").unwrap(),
                    smallvec![db_string("level").unwrap(), db_string("site").unwrap()],
                    smallvec![TypedIndexKind::I64, TypedIndexKind::I64],
                    None,
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }

    let snapshot = graph.read();
    let rows = snapshot.iter_property_index_stats().collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "the composite registration must be reported");
    let row = &rows[0];
    assert!(row.composite, "the family must be carried, not inferred");
    assert_eq!(
        row.properties
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["level", "site"],
        "declaration order is what the catalog names it by"
    );
    assert_eq!(row.kinds.len(), 2);
    assert!(row.answers_probes());
}
