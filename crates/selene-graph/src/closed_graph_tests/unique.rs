use super::*;

fn unique_person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("closed.unique.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.unique.person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                decimal_type: None,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn unique_person_graph(graph_id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(graph_id))
        .bound_to(unique_person_graph_type())
        .unwrap()
        .build()
        .unwrap()
}

fn serial(value: &str) -> PropertyMap {
    prop("serial", Value::String(db_string(value)))
}

#[test]
fn duplicate_unique_node_property_rejects_commit() {
    let shared = unique_person_graph(31);
    let mut txn = shared.begin_write();
    let (first, second) = {
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(LabelSet::single(db_string("Person")), serial("A"))
            .unwrap();
        let second = mutator
            .create_node(LabelSet::single(db_string("Person")), serial("A"))
            .unwrap();
        (first, second)
    };

    let err = txn
        .commit()
        .expect_err("duplicate unique value rejects commit");

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::UniquePropertyDuplicate {
            entity_id,
            conflicting_entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(second)
            && conflicting_entity_id == EntityId::Node(first)
            && property == db_string("serial")
    ));
    assert_eq!(shared.read().node_count(), 0);
}

#[test]
fn duplicate_unique_node_property_update_rolls_back() {
    let shared = unique_person_graph(32);
    let (first, second) = {
        let mut txn = shared.begin_write();
        let (first, second) = {
            let mut mutator = txn.mutator();
            let first = mutator
                .create_node(LabelSet::single(db_string("Person")), serial("A"))
                .unwrap();
            let second = mutator
                .create_node(LabelSet::single(db_string("Person")), serial("B"))
                .unwrap();
            (first, second)
        };
        txn.commit().unwrap();
        (first, second)
    };

    let mut txn = shared.begin_write();
    txn.mutator()
        .update_node(
            second,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(db_string("serial"), Value::String(db_string("A")))], []).unwrap(),
        )
        .unwrap();
    let err = txn
        .commit()
        .expect_err("duplicate unique update rejects commit");

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::UniquePropertyDuplicate {
            entity_id,
            conflicting_entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(second)
            && conflicting_entity_id == EntityId::Node(first)
            && property == db_string("serial")
    ));
    assert_eq!(
        shared
            .read()
            .node_properties(second)
            .and_then(|properties| properties.get(&db_string("serial"))),
        Some(&Value::String(db_string("B")))
    );
}

#[test]
fn duplicate_unique_node_property_update_reports_changed_entity() {
    let shared = unique_person_graph(34);
    let (first, second) = {
        let mut txn = shared.begin_write();
        let (first, second) = {
            let mut mutator = txn.mutator();
            let first = mutator
                .create_node(LabelSet::single(db_string("Person")), serial("A"))
                .unwrap();
            let second = mutator
                .create_node(LabelSet::single(db_string("Person")), serial("B"))
                .unwrap();
            (first, second)
        };
        txn.commit().unwrap();
        (first, second)
    };

    let mut txn = shared.begin_write();
    txn.mutator()
        .update_node(
            first,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], []).unwrap(),
        )
        .unwrap();
    let err = txn
        .commit()
        .expect_err("duplicate unique update rejects commit");

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::UniquePropertyDuplicate {
            entity_id,
            conflicting_entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(first)
            && conflicting_entity_id == EntityId::Node(second)
            && property == db_string("serial")
    ));
    assert_eq!(
        shared
            .read()
            .node_properties(first)
            .and_then(|properties| properties.get(&db_string("serial"))),
        Some(&Value::String(db_string("A")))
    );
}

#[test]
fn repeated_unique_node_property_update_to_same_value_commits() {
    let shared = unique_person_graph(35);
    let id = {
        let mut txn = shared.begin_write();
        let id = txn
            .mutator()
            .create_node(LabelSet::single(db_string("Person")), serial("A"))
            .unwrap();
        txn.commit().unwrap();
        id
    };

    let mut txn = shared.begin_write();
    txn.mutator()
        .update_node(
            id,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], []).unwrap(),
        )
        .unwrap();
    txn.mutator()
        .update_node(
            id,
            LabelDiff::new([], []).unwrap(),
            PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], []).unwrap(),
        )
        .unwrap();
    txn.commit().unwrap();

    assert_eq!(
        shared
            .read()
            .node_properties(id)
            .and_then(|properties| properties.get(&db_string("serial"))),
        Some(&Value::String(db_string("B")))
    );
}

#[test]
fn unique_node_property_reuses_value_after_delete() {
    let shared = unique_person_graph(33);
    let first = {
        let mut txn = shared.begin_write();
        let first = txn
            .mutator()
            .create_node(LabelSet::single(db_string("Person")), serial("A"))
            .unwrap();
        txn.commit().unwrap();
        first
    };

    let mut txn = shared.begin_write();
    txn.mutator().delete_node(first).unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    let second = txn
        .mutator()
        .create_node(LabelSet::single(db_string("Person")), serial("A"))
        .unwrap();
    txn.commit().unwrap();

    assert!(shared.read().is_node_alive(second));
}
