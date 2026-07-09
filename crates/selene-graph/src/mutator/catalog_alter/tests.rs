use selene_core::{
    Change, GraphId, LabelSet, PropertyMap, PropertyValueType, SchemaChange, Value, db_string,
};

use crate::{
    EdgeEndpointDef, EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef,
    SharedGraph, TypeViolation, ValidationMode,
};

fn three_node_type_graph() -> GraphTypeDef {
    let a = db_string("A").unwrap();
    let b = db_string("B").unwrap();
    let c = db_string("C").unwrap();
    let b_to_c = db_string("B_TO_C").unwrap();
    GraphTypeDef {
        name: db_string("catalog.three-node.graph").unwrap(),
        node_types: [a, b, c]
            .into_iter()
            .map(|name| NodeTypeDef {
                name: name.clone(),
                key_labels: LabelSet::single(name),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            })
            .collect(),
        edge_types: vec![EdgeTypeDef {
            name: b_to_c.clone(),
            label: b_to_c,
            source_node_type: EdgeEndpointDef::NodeType(1),
            target_node_type: EdgeEndpointDef::NodeType(2),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn optional_string_property(name: &str) -> PropertyTypeDef {
    PropertyTypeDef {
        name: db_string(name).unwrap(),
        value_type: PropertyValueType::String,
        list_element_type: None,
        required: false,
        default: None,
        immutable: false,
        unique: false,
        decimal_type: None,
        character_string_type: None,
        byte_string_type: None,
        record_field_types: None,
    }
}

fn assert_lossy_property_rejected(shared: &SharedGraph, property: PropertyTypeDef, expected: &str) {
    let before = shared.graph_type().unwrap();
    let mut txn = shared.begin_write();
    let error = {
        let mut mutator = txn.mutator();
        let error = mutator
            .alter_node_type(db_string("B").unwrap(), vec![property])
            .unwrap_err();
        assert_eq!(
            mutator.read().meta.bound_type.as_deref(),
            Some(before.as_ref())
        );
        error
    };
    assert!(
        error.to_string().contains(expected),
        "expected {expected:?}, observed {error}"
    );
    assert_eq!(txn.change_count(), 0);
    drop(txn);
    assert_eq!(shared.graph_type().as_deref(), Some(before.as_ref()));
}

#[test]
fn alter_node_type_preserves_slot_and_emits_one_schema_change() {
    let shared = SharedGraph::builder(GraphId::new(1010))
        .bound_to(three_node_type_graph())
        .unwrap()
        .build()
        .unwrap();
    let b = db_string("B").unwrap();
    let nickname = optional_string_property("nickname");

    let outcome = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(b.clone(), vec![nickname.clone(), nickname.clone()])
            .unwrap();
        txn.commit().unwrap()
    };

    let graph_type = shared.graph_type().unwrap();
    let names: Vec<_> = graph_type
        .node_types
        .iter()
        .map(|node_type| node_type.name.as_str())
        .collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert_eq!(graph_type.node_types[1].properties, [nickname]);
    assert_eq!(
        graph_type.edge_types[0].source_node_type,
        EdgeEndpointDef::NodeType(1)
    );
    assert_eq!(
        graph_type.edge_types[0].target_node_type,
        EdgeEndpointDef::NodeType(2)
    );
    assert!(matches!(
        outcome.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAlteredV2 {
                label, properties, ..
            },
            ..
        }] if *label == b && properties.len() == 1 && properties[0].nullable
    ));
}

#[test]
fn alter_node_type_exact_repeat_is_idempotent() {
    let shared = SharedGraph::builder(GraphId::new(1011))
        .bound_to(three_node_type_graph())
        .unwrap()
        .build()
        .unwrap();
    let b = db_string("B").unwrap();
    let nickname = optional_string_property("nickname");

    let first = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(b.clone(), vec![nickname.clone()])
            .unwrap();
        txn.commit().unwrap()
    };
    assert!(matches!(
        first.changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAlteredV2 { .. },
            ..
        }]
    ));

    let repeated = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(b, vec![nickname.clone()])
            .unwrap();
        txn.commit().unwrap()
    };
    assert!(repeated.changes.is_empty());
    assert_eq!(
        shared.graph_type().unwrap().node_types[1].properties,
        [nickname]
    );
}

#[test]
fn alter_node_type_rejects_missing_required_and_conflicting_properties() {
    let shared = SharedGraph::builder(GraphId::new(1012))
        .bound_to(three_node_type_graph())
        .unwrap()
        .build()
        .unwrap();

    let mut txn = shared.begin_write();
    let missing = txn
        .mutator()
        .alter_node_type(db_string("Missing").unwrap(), Vec::new())
        .unwrap_err();
    assert!(matches!(
        missing,
        GraphError::Inconsistent { reason } if reason.contains("does not exist")
    ));
    drop(txn);

    let mut required = optional_string_property("required_name");
    required.required = true;
    let mut txn = shared.begin_write();
    let error = txn
        .mutator()
        .alter_node_type(db_string("B").unwrap(), vec![required])
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::Inconsistent { reason } if reason.contains("cannot add required property")
    ));
    drop(txn);

    let nickname = optional_string_property("nickname");
    let mut conflicting = nickname.clone();
    conflicting.value_type = PropertyValueType::Int;
    let mut txn = shared.begin_write();
    let error = txn
        .mutator()
        .alter_node_type(db_string("B").unwrap(), vec![nickname, conflicting])
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::Inconsistent { reason } if reason.contains("cannot redefine property")
    ));
}

#[test]
fn alter_node_type_rejects_lossy_delta_descriptors_without_state_change() {
    let shared = SharedGraph::builder(GraphId::new(1013))
        .bound_to(three_node_type_graph())
        .unwrap()
        .build()
        .unwrap();

    let mut native_record = optional_string_property("native_record");
    native_record.value_type = PropertyValueType::Record;
    assert_lossy_property_rejected(&shared, native_record, "losslessly");

    let mut untyped_list = optional_string_property("untyped_list");
    untyped_list.value_type = PropertyValueType::List;
    assert_lossy_property_rejected(&shared, untyped_list, "missing element type");

    let mut noncanonical_default = optional_string_property("signed_zero");
    noncanonical_default.value_type = PropertyValueType::Float;
    noncanonical_default.default = Some(crate::PropertyDefaultValue::Float((-0.0_f64).to_bits()));
    assert_lossy_property_rejected(&shared, noncanonical_default, "losslessly");
}

#[test]
fn alter_node_type_accepts_null_default_for_nullable_property() {
    let shared = SharedGraph::builder(GraphId::new(1014))
        .bound_to(three_node_type_graph())
        .unwrap()
        .build()
        .unwrap();
    let mut property = optional_string_property("nullable_default");
    property.default = Some(crate::PropertyDefaultValue::Null);

    let mut txn = shared.begin_write();
    txn.mutator()
        .alter_node_type(db_string("B").unwrap(), vec![property.clone()])
        .unwrap();
    txn.commit().unwrap();

    assert_eq!(
        shared.graph_type().unwrap().node_types[1].properties,
        [property]
    );
}

#[test]
fn alter_node_type_validation_failure_rolls_back_schema_and_version() {
    let b = db_string("B").unwrap();
    let graph_type = GraphTypeDef {
        name: db_string("catalog.warn-alter.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: b.clone(),
            key_labels: LabelSet::single(b.clone()),
            properties: Vec::new(),
            validation_mode: ValidationMode::Warn,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(1015))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap();
    let property = db_string("observed").unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(b.clone()),
                PropertyMap::from_pairs([(
                    property.clone(),
                    Value::String(db_string("string-before-schema").unwrap()),
                )])
                .unwrap(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    assert_eq!(shared.schema_version(), 0);

    let mut declaration = optional_string_property(property.as_str());
    declaration.value_type = PropertyValueType::Int;
    let mut txn = shared.begin_write();
    txn.mutator().alter_node_type(b, vec![declaration]).unwrap();
    let error = txn.commit().unwrap_err();
    assert!(matches!(
        error,
        GraphError::TypeViolation(TypeViolation::PropertyTypeMismatch {
            property: failed_property,
            ..
        }) if failed_property == property
    ));
    assert!(
        shared.graph_type().unwrap().node_types[0]
            .properties
            .is_empty()
    );
    assert_eq!(shared.schema_version(), 0);
}
