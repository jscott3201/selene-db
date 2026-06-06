use super::*;

#[test]
fn node_property_rejects_relabel_bypass_at_commit() {
    let graph_type = GraphTypeDef {
        name: db_string("closed.immutable.relabel.graph"),
        node_types: vec![
            NodeTypeDef {
                name: db_string("closed.immutable.relabel.person"),
                key_labels: LabelSet::single(db_string("Person")),
                properties: vec![PropertyTypeDef {
                    name: db_string("serial"),
                    value_type: PropertyValueType::String,
                    list_element_type: None,
                    required: true,
                    default: None,
                    immutable: true,

                    record_field_types: None,
                }],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("closed.immutable.relabel.temp"),
                key_labels: LabelSet::single(db_string("Temp")),
                properties: vec![PropertyTypeDef {
                    name: db_string("serial"),
                    value_type: PropertyValueType::String,
                    list_element_type: None,
                    required: true,
                    default: None,
                    immutable: false,

                    record_field_types: None,
                }],
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(22))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let id = txn
        .mutator()
        .create_node(
            LabelSet::single(db_string("Person")),
            prop("serial", Value::String(db_string("A"))),
        )
        .unwrap();
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .update_node(
                id,
                LabelDiff::new([db_string("Temp")], [db_string("Person")]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        mutator
            .update_node(
                id,
                LabelDiff::new([], []).unwrap(),
                PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], [])
                    .unwrap(),
            )
            .unwrap();
        mutator
            .update_node(
                id,
                LabelDiff::new([db_string("Person")], [db_string("Temp")]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
    }
    let err = txn
        .commit()
        .expect_err("commit rejects immutable property changed while relabeled");

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(id) && property == db_string("serial")
    ));
}

#[test]
fn edge_property_rejects_endpoint_relabel_bypass_at_commit() {
    let person = db_string("Person");
    let temp = db_string("Temp");
    let knows = db_string("KNOWS");
    let graph_type = GraphTypeDef {
        name: db_string("closed.immutable.edge.relabel.graph"),
        node_types: vec![
            NodeTypeDef {
                name: db_string("closed.immutable.edge.person"),
                key_labels: LabelSet::single(person.clone()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: db_string("closed.immutable.edge.temp"),
                key_labels: LabelSet::single(temp.clone()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![crate::EdgeTypeDef {
            name: db_string("closed.immutable.edge.knows"),
            label: knows.clone(),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: true,

                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    };
    let shared = SharedGraph::builder(GraphId::new(23))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let mut txn = shared.begin_write();
    let (source, target, edge) = {
        let mut mutator = txn.mutator();
        let source = mutator
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
            .unwrap();
        let target = mutator
            .create_node(LabelSet::single(person.clone()), PropertyMap::new())
            .unwrap();
        let edge = mutator
            .create_edge(
                knows,
                source,
                target,
                prop("serial", Value::String(db_string("A"))),
            )
            .unwrap();
        (source, target, edge)
    };
    txn.commit().unwrap();

    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .update_node(
                source,
                LabelDiff::new([temp.clone()], [person.clone()]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
        mutator
            .update_edge(
                edge,
                PropertyDiff::new([(db_string("serial"), Value::String(db_string("B")))], [])
                    .unwrap(),
            )
            .unwrap();
        mutator
            .update_node(
                source,
                LabelDiff::new([person], [temp]).unwrap(),
                PropertyDiff::new([], []).unwrap(),
            )
            .unwrap();
    }
    let err = txn
        .commit()
        .expect_err("commit rejects immutable edge property changed while endpoint relabeled");

    assert!(matches!(
        err,
        GraphError::TypeViolation(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            ..
        }) if entity_id == EntityId::Edge(edge) && property == db_string("serial")
    ));
    assert!(shared.read().is_node_alive(target));
}
