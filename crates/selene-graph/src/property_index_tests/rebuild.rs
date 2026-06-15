use super::*;

#[test]
fn build_property_index_is_strict_for_existing_data() {
    let label = db_string("pi.build.label").unwrap();
    let age = db_string("pi.build.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([(
        age.clone(),
        Value::String(db_string("wrong").unwrap()),
    )]));
    graph.node_store.alive_mut().insert(0);

    let err =
        build_property_index(&graph, label.clone(), age.clone(), TypedIndexKind::I64).unwrap_err();

    assert!(matches!(
        err,
        GraphError::IndexValueRejected {
            label: err_label,
            property: err_property,
            expected_kind: TypedIndexKind::I64,
            observed: "String",
        } if err_label == label && err_property == age
    ));
}

#[test]
fn build_property_index_admits_existing_string_rows() {
    let label = db_string("pi.build.string.label").unwrap();
    let name = db_string("pi.build.string.name").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(11));
    let unique = (0..3)
        .map(|i| db_string(&format!("pi.build.string.foo_{i}")).unwrap())
        .collect::<Vec<_>>();
    for (row, content) in unique.iter().enumerate() {
        graph
            .node_store
            .labels
            .push(LabelSet::single(label.clone()));
        graph.node_store.properties.push(property_map([(
            name.clone(),
            Value::String(content.clone()),
        )]));
        graph.node_store.alive_mut().insert(row as u32);
    }

    let index = build_property_index(&graph, label, name, TypedIndexKind::String).expect("admits");

    assert_eq!(index.cardinality(), 3);
}

#[test]
fn index_rejection_keeps_kind_mismatch_path_unchanged() {
    let label = db_string("pi.kind-mismatch.label").unwrap();
    let name = db_string("pi.kind-mismatch.name").unwrap();
    let synthetic = TypedIndexValueError::KindMismatch {
        expected_kind: TypedIndexKind::I64,
        observed: "String",
    };

    let promoted = index_rejection(label, name, synthetic);

    assert!(matches!(
        promoted,
        GraphError::IndexValueRejected {
            expected_kind: TypedIndexKind::I64,
            observed: "String",
            ..
        }
    ));
}

#[test]
fn rebuild_property_indexes_is_lenient_on_kind_drift() {
    // Snapshot has a property index registered as I64, but the column
    // value is a String - a state that's reachable at runtime when an
    // open-graph update writes a kind-mismatched value (the commit-path
    // logs and skips). Recovery must reconstruct the registry without
    // hard-failing, otherwise a runtime-accepted snapshot becomes
    // unloadable.
    let label = db_string("pi.rebuild.label").unwrap();
    let age = db_string("pi.rebuild.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    // Row 0: matching kind (Int 30) - should land in the rebuilt index.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph
        .node_store
        .properties
        .push(property_map([(age.clone(), Value::Int(30))]));
    graph.node_store.alive_mut().insert(0);
    // Row 1: mismatched kind (String) - should be skipped, not abort.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([(
        age.clone(),
        Value::String(db_string("pi.rebuild.wrong").unwrap()),
    )]));
    graph.node_store.alive_mut().insert(1);
    // Pre-register the index (will be cleared and rebuilt by
    // rebuild_property_indexes).
    graph
        .property_index
        .insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    rebuild_property_indexes(&mut graph).expect("lenient rebuild does not error on drift");

    // The matching row landed; the mismatched row was logged and skipped.
    let index = graph.property_index.get(&(label, age)).unwrap();
    let hits = index
        .index
        .lookup_eq(&Value::Int(30))
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    assert!(hits.contains(0));
    assert!(!hits.contains(1));
}
