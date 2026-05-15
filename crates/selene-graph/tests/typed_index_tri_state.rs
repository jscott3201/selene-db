//! Regression tests for property-index equality lookup tri-state semantics.

use std::borrow::Cow;

use selene_core::{GraphId, LabelSet, PropertyMap, Value, intern};
use selene_graph::{SharedGraph, TypedIndexKind};

#[test]
fn typed_index_tri_state_pin() {
    let shared = graph_with_age_index();
    let person = intern("tri.person").unwrap();
    let age = intern("tri.age").unwrap();
    let absent = intern("tri.absent").unwrap();

    let snapshot = shared.read();
    let hit = snapshot
        .nodes_with_property_eq(&person, &age, &Value::Int(30))
        .expect("registered index hit returns rows");
    assert!(matches!(hit, Cow::Borrowed(_)));
    assert_eq!(hit.iter().collect::<Vec<_>>(), vec![0]);

    let missing_key = snapshot
        .nodes_with_property_eq(&person, &age, &Value::Int(999))
        .expect("registered index miss returns Some(empty), not None");
    assert!(matches!(missing_key, Cow::Owned(_)));
    assert!(missing_key.is_empty());

    assert!(
        snapshot
            .nodes_with_property_eq(&person, &absent, &Value::Int(999))
            .is_none(),
        "unregistered index remains None so callers can fall back to scan"
    );
}

fn graph_with_age_index() -> SharedGraph {
    let shared = SharedGraph::new(GraphId::new(88));
    let person = intern("tri.person").unwrap();
    let age = intern("tri.age").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(person),
                PropertyMap::from_pairs([(age, Value::Int(30))]).unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_property_index(person, age, TypedIndexKind::I64)
        .unwrap();
    shared
}
