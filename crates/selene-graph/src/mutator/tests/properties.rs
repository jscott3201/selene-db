use super::*;

#[test]
fn remove_node_property_removes_value_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (id, prop) = {
        let mut mutator = txn.mutator();
        let prop = db_string("node.remove.prop").unwrap();
        let id = mutator
            .create_node(
                LabelSet::new(),
                PropertyMap::from_pairs([(prop.clone(), Value::Int(7))]).unwrap(),
            )
            .unwrap();
        mutator.remove_node_property(id, prop.clone()).unwrap();
        assert!(
            mutator
                .read()
                .node_properties(id)
                .unwrap()
                .get(&prop)
                .is_none()
        );
        (id, prop)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes[1],
        Change::NodePropertyRemoved {
            id,
            property: prop.clone()
        }
    );
    assert!(
        shared
            .read()
            .node_properties(id)
            .unwrap()
            .get(&prop)
            .is_none()
    );
}

#[test]
fn remove_node_property_absent_is_noop() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let id = empty_node(&mut mutator);
        mutator
            .remove_node_property(id, db_string("node.remove.absent").unwrap())
            .unwrap();
    }
    let outcome = txn.commit().unwrap();
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::NodeCreated { .. }]
    ));
}

#[test]
fn remove_node_label_updates_index_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (id, label) = {
        let mut mutator = txn.mutator();
        let label = db_string("node.remove.label").unwrap();
        let id = mutator
            .create_node(LabelSet::single(label.clone()), PropertyMap::new())
            .unwrap();
        mutator.remove_node_label(id, label.clone()).unwrap();
        assert!(mutator.read().nodes_with_label(&label).is_none());
        (id, label)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes[1],
        Change::NodeLabelRemoved {
            id,
            label: label.clone()
        }
    );
    assert!(shared.read().nodes_with_label(&label).is_none());
}

#[test]
fn remove_edge_property_removes_value_and_emits_change() {
    let shared = SharedGraph::new(GraphId::new(1));
    let mut txn = shared.begin_write();
    let (edge, prop) = {
        let mut mutator = txn.mutator();
        let left = empty_node(&mut mutator);
        let right = empty_node(&mut mutator);
        let prop = db_string("edge.remove.prop").unwrap();
        let edge = mutator
            .create_edge(
                db_string("edge.remove").unwrap(),
                left,
                right,
                PropertyMap::from_pairs([(prop.clone(), Value::Int(9))]).unwrap(),
            )
            .unwrap();
        mutator.remove_edge_property(edge, prop.clone()).unwrap();
        assert!(
            mutator
                .read()
                .edge_properties(edge)
                .unwrap()
                .get(&prop)
                .is_none()
        );
        (edge, prop)
    };
    let outcome = txn.commit().unwrap();
    assert_eq!(
        outcome.changes.last(),
        Some(&Change::EdgePropertyRemoved {
            id: edge,
            property: prop.clone(),
        })
    );
    assert!(
        shared
            .read()
            .edge_properties(edge)
            .unwrap()
            .get(&prop)
            .is_none()
    );
}

#[test]
fn remove_node_property_rejects_immutable_property() {
    let serial = db_string("node.remove.immutable.serial").unwrap();
    let person = db_string("node.remove.immutable.person").unwrap();
    let graph_type = GraphTypeDef {
        name: db_string("node.remove.immutable.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person.clone(),
            key_labels: LabelSet::single(person.clone()),
            properties: vec![PropertyTypeDef {
                name: serial.clone(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: true,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(1))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(
            LabelSet::single(person),
            PropertyMap::from_pairs([(serial.clone(), Value::String(serial.clone()))]).unwrap(),
        )
        .unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    let err = txn
        .mutator()
        .remove_node_property(id, serial.clone())
        .expect_err("immutable property removal is rejected");
    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(id) && property == serial
    ));
}
