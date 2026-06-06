use super::*;

fn unique_node_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("validator.unique.node.graph"),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("validator.device"),
            key_labels: LabelSet::single(db_string("Device")),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

#[test]
fn validate_entity_state_rejects_duplicate_unique_node_property() {
    let shared = SharedGraph::builder(GraphId::new(12)).build().unwrap();
    let mut txn = shared.begin_write();
    let (first, second) = {
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(
                LabelSet::single(db_string("Device")),
                prop("serial", Value::String(db_string("dup"))),
            )
            .unwrap();
        let second = mutator
            .create_node(
                LabelSet::single(db_string("Device")),
                prop("serial", Value::String(db_string("dup"))),
            )
            .unwrap();
        (first, second)
    };
    txn.commit().unwrap();

    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &unique_node_graph_type()),
        Err(TypeViolation::UniquePropertyDuplicate {
            entity_id,
            conflicting_entity_id,
            property,
            ..
        }) if entity_id == EntityId::Node(second)
            && conflicting_entity_id == EntityId::Node(first)
            && property == db_string("serial")
    ));
}

#[test]
fn validate_entity_state_exempts_missing_and_null_unique_node_values() {
    let shared = SharedGraph::builder(GraphId::new(13)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(db_string("Device")), PropertyMap::new())
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(db_string("Device")),
                prop("serial", Value::Null),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(db_string("Device")),
                prop("serial", Value::Null),
            )
            .unwrap();
    }
    txn.commit().unwrap();

    validate_entity_state(shared.read().as_ref(), &unique_node_graph_type()).unwrap();
}

#[test]
fn validate_entity_state_treats_signed_zero_float_values_as_duplicate_unique_values() {
    let graph_type = GraphTypeDef {
        name: db_string("validator.unique.float.graph"),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("validator.metric"),
            key_labels: LabelSet::single(db_string("Metric")),
            properties: vec![PropertyTypeDef {
                name: db_string("score"),
                value_type: PropertyValueType::Float,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(15)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string("Metric")),
                prop("score", Value::Float(0.0)),
            )
            .unwrap();
        mutator
            .create_node(
                LabelSet::single(db_string("Metric")),
                prop("score", Value::Float(-0.0)),
            )
            .unwrap();
    }
    txn.commit().unwrap();

    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type),
        Err(TypeViolation::UniquePropertyDuplicate { property, .. })
            if property == db_string("score")
    ));
}

fn unique_edge_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("validator.unique.edge.graph"),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("validator.device"),
            key_labels: LabelSet::single(db_string("Device")),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![crate::EdgeTypeDef {
            name: db_string("validator.link"),
            label: db_string("LINK"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

#[test]
fn validate_entity_state_rejects_duplicate_unique_edge_property() {
    let shared = SharedGraph::builder(GraphId::new(14)).build().unwrap();
    let mut txn = shared.begin_write();
    let (first, second) = {
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(LabelSet::single(db_string("Device")), PropertyMap::new())
            .unwrap();
        let b = mutator
            .create_node(LabelSet::single(db_string("Device")), PropertyMap::new())
            .unwrap();
        let first = mutator
            .create_edge(
                db_string("LINK"),
                a,
                b,
                prop("serial", Value::String(db_string("dup"))),
            )
            .unwrap();
        let second = mutator
            .create_edge(
                db_string("LINK"),
                b,
                a,
                prop("serial", Value::String(db_string("dup"))),
            )
            .unwrap();
        (first, second)
    };
    txn.commit().unwrap();

    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &unique_edge_graph_type()),
        Err(TypeViolation::UniquePropertyDuplicate {
            entity_id,
            conflicting_entity_id,
            property,
            ..
        }) if entity_id == EntityId::Edge(second)
            && conflicting_entity_id == EntityId::Edge(first)
            && property == db_string("serial")
    ));
}

#[test]
fn validate_entity_state_keeps_node_and_edge_unique_domains_separate() {
    let graph_type = GraphTypeDef {
        name: db_string("validator.unique.shared-name.graph"),
        node_types: vec![crate::NodeTypeDef {
            name: db_string("validator.shared"),
            key_labels: LabelSet::single(db_string("Shared")),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![crate::EdgeTypeDef {
            name: db_string("validator.shared"),
            label: db_string("Shared"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: vec![PropertyTypeDef {
                name: db_string("serial"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: true,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    };
    let shared = SharedGraph::builder(GraphId::new(16)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let first = mutator
            .create_node(
                LabelSet::single(db_string("Shared")),
                prop("serial", Value::String(db_string("same"))),
            )
            .unwrap();
        let second = mutator
            .create_node(LabelSet::single(db_string("Shared")), PropertyMap::new())
            .unwrap();
        mutator
            .create_edge(
                db_string("Shared"),
                first,
                second,
                prop("serial", Value::String(db_string("same"))),
            )
            .unwrap();
    }
    txn.commit().unwrap();

    validate_entity_state(shared.read().as_ref(), &graph_type).unwrap();
}
