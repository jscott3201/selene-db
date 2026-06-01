use std::sync::Arc;

use selene_core::{ExtensionTypeId, GraphId, Record, RecordTypeId, RecordTyped, intern};

use super::*;
use crate::{GraphError, RecordFieldType, RecordFieldTypeDef, RecordFieldTypes, SharedGraph};

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(istr(name), value)]).unwrap()
}

fn graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: istr("validator.graph"),
        node_types: vec![
            crate::NodeTypeDef {
                name: istr("validator.person"),
                key_labels: LabelSet::single(istr("Person")),
                properties: vec![PropertyTypeDef {
                    name: istr("name"),
                    value_type: PropertyValueType::String,
                    list_element_type: None,
                    required: true,
                    default: None,
                    immutable: false,
                    record_field_types: None,
                }],
                validation_mode: ValidationMode::Strict,
            },
            crate::NodeTypeDef {
                name: istr("validator.company"),
                key_labels: LabelSet::single(istr("Company")),
                properties: vec![PropertyTypeDef {
                    name: istr("name"),
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
        edge_types: vec![crate::EdgeTypeDef {
            name: istr("validator.works_at"),
            label: istr("WORKS_AT"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(1),
            properties: vec![PropertyTypeDef {
                name: istr("since"),
                value_type: PropertyValueType::Int,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn valid_graph() -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let person = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("Alice"))),
            )
            .unwrap();
        let company = mutator
            .create_node(
                LabelSet::single(istr("Company")),
                prop("name", Value::String(istr("Acme"))),
            )
            .unwrap();
        mutator
            .create_edge(
                istr("WORKS_AT"),
                person,
                company,
                prop("since", Value::Int(2026)),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    shared.read().as_ref().clone()
}

#[test]
fn validate_entity_state_accepts_valid_graph() {
    validate_entity_state(&valid_graph(), &graph_type()).unwrap();
}

#[test]
fn validate_change_accepts_applied_node_created() {
    let graph = valid_graph();
    validate_change(
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(istr("Person")),
            properties: prop("name", Value::String(istr("Alice"))),
        },
        &graph,
        &graph_type(),
    )
    .unwrap();
}

#[test]
fn rejects_unknown_node_label() {
    let shared = SharedGraph::builder(GraphId::new(2)).build().unwrap();
    let mut txn = shared.begin_write();
    let node = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("Project")), PropertyMap::new())
            .unwrap()
    };
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::UnknownNodeLabel { id, .. }) if id == node
    ));
}

#[test]
fn rejects_unknown_edge_label() {
    let mut graph = valid_graph();
    graph.edge_store.label.set(0, istr("KNOWS"));
    assert!(matches!(
        validate_entity_state(&graph, &graph_type()),
        Err(TypeViolation::UnknownEdgeLabel { id, label })
            if id == EdgeId::new(1) && label == istr("KNOWS")
    ));
}

#[test]
fn rejects_edge_endpoint_mismatch() {
    let shared = SharedGraph::builder(GraphId::new(3)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(
                LabelSet::single(istr("Company")),
                prop("name", Value::String(istr("A"))),
            )
            .unwrap();
        let b = mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::String(istr("B"))),
            )
            .unwrap();
        mutator
            .create_edge(istr("WORKS_AT"), a, b, PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::EdgeEndpointTypeMismatch {
            observed_source_type: 1,
            observed_target_type: 0,
            ..
        })
    ));
}

#[test]
fn rejects_missing_required_property() {
    let shared = SharedGraph::builder(GraphId::new(4)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::MissingRequiredProperty { property, .. }) if property == istr("name")
    ));
}

#[test]
fn rejects_property_type_mismatch() {
    let shared = SharedGraph::builder(GraphId::new(5)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop("name", Value::Int(7)),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::PropertyTypeMismatch {
            expected: PropertyValueType::String,
            observed: "Int",
            ..
        })
    ));
}

#[test]
fn legacy_untyped_list_declaration_accepts_any_list_elements() {
    let declaration = PropertyTypeDef {
        name: istr("legacy"),
        value_type: PropertyValueType::List,
        list_element_type: None,
        required: false,
        default: None,
        immutable: false,
        record_field_types: None,
    };

    assert!(property_value_matches(
        &declaration,
        &Value::List(vec![Value::Int(1), Value::String(istr("two"))])
    ));
    assert!(!property_value_matches(&declaration, &Value::Int(1)));
}

#[test]
fn rejects_extension_value() {
    let shared = SharedGraph::builder(GraphId::new(6)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Person")),
                prop(
                    "name",
                    Value::Extended {
                        type_id: ExtensionTypeId(0x100),
                        payload: Arc::from([1_u8]),
                    },
                ),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::ExtensionValueRejected { property, .. }) if property == istr("name")
    ));
}

#[test]
fn rejects_undeclared_property() {
    let shared = SharedGraph::builder(GraphId::new(7)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let mut props = prop("name", Value::String(istr("Alice")));
        props.set(istr("extra"), Value::Bool(true)).unwrap();
        mutator
            .create_node(LabelSet::single(istr("Person")), props)
            .unwrap();
    }
    txn.commit().unwrap();
    assert!(matches!(
        validate_entity_state(shared.read().as_ref(), &graph_type()),
        Err(TypeViolation::UndeclaredProperty { property, .. }) if property == istr("extra")
    ));
}

#[test]
fn graph_error_wraps_type_violation() {
    let error = GraphError::from(TypeViolation::UnknownEdgeLabel {
        id: EdgeId::new(1),
        label: istr("BAD"),
    });
    assert_eq!(error.gqlstatus(), "G2000");
}

// --- C7: closed/typed RECORD property validation (ISO 39075:2024 §4.15.4) ---

fn open_record(fields: &[(&str, Value)]) -> Value {
    Value::Record(Box::new(Record::Open(
        fields
            .iter()
            .map(|(name, value)| (istr(name), value.clone()))
            .collect(),
    )))
}

/// A `config` RECORD descriptor with a required `host :: STRING` and an optional
/// `port :: INT`. The optional field is set directly on the descriptor — GQL DDL has no
/// field-nullability syntax today (the grammar's `record_field_type` is `name :: type`
/// only), so the `required: false` path is reachable only via a hand-built descriptor.
fn closed_record_declaration() -> PropertyTypeDef {
    PropertyTypeDef {
        name: istr("config"),
        value_type: PropertyValueType::RecordTyped,
        list_element_type: None,
        required: false,
        default: None,
        immutable: false,
        record_field_types: Some(RecordFieldTypes(vec![
            RecordFieldTypeDef {
                name: istr("host"),
                field_type: RecordFieldType::Scalar(PropertyValueType::String),
                required: true,
            },
            RecordFieldTypeDef {
                name: istr("port"),
                field_type: RecordFieldType::Scalar(PropertyValueType::Int),
                required: false,
            },
        ])),
    }
}

#[test]
fn closed_record_accepts_conforming_value() {
    let declaration = closed_record_declaration();
    assert!(property_value_matches(
        &declaration,
        &open_record(&[
            ("host", Value::String(istr("h"))),
            ("port", Value::Int(8080)),
        ])
    ));
    // Optional field may be omitted.
    assert!(property_value_matches(
        &declaration,
        &open_record(&[("host", Value::String(istr("h")))])
    ));
}

#[test]
fn closed_record_rejects_missing_required_field() {
    let declaration = closed_record_declaration();
    assert!(!property_value_matches(
        &declaration,
        &open_record(&[("port", Value::Int(8080))])
    ));
}

#[test]
fn closed_record_rejects_wrong_field_type() {
    let declaration = closed_record_declaration();
    assert!(!property_value_matches(
        &declaration,
        &open_record(&[("host", Value::Int(1))])
    ));
}

#[test]
fn closed_record_rejects_extra_undeclared_field() {
    // ISO §4.15.4 closed record = field-name-set EQUALITY: an extra field is rejected.
    let declaration = closed_record_declaration();
    assert!(!property_value_matches(
        &declaration,
        &open_record(&[
            ("host", Value::String(istr("h"))),
            ("port", Value::Int(8080)),
            ("extra", Value::Bool(true)),
        ])
    ));
}

#[test]
fn closed_record_accepts_and_rejects_nested() {
    // config :: RECORD { id :: INT, tags :: LIST<STRING>, meta :: RECORD { live :: BOOL } }
    let declaration = PropertyTypeDef {
        name: istr("config"),
        value_type: PropertyValueType::RecordTyped,
        list_element_type: None,
        required: true,
        default: None,
        immutable: false,
        record_field_types: Some(RecordFieldTypes(vec![
            RecordFieldTypeDef {
                name: istr("id"),
                field_type: RecordFieldType::Scalar(PropertyValueType::Int),
                required: true,
            },
            RecordFieldTypeDef {
                name: istr("tags"),
                field_type: RecordFieldType::List(Box::new(RecordFieldType::Scalar(
                    PropertyValueType::String,
                ))),
                required: true,
            },
            RecordFieldTypeDef {
                name: istr("meta"),
                field_type: RecordFieldType::Record(Box::new(RecordFieldTypes(vec![
                    RecordFieldTypeDef {
                        name: istr("live"),
                        field_type: RecordFieldType::Scalar(PropertyValueType::Bool),
                        required: true,
                    },
                ]))),
                required: true,
            },
        ])),
    };
    let conforming = open_record(&[
        ("id", Value::Int(1)),
        (
            "tags",
            Value::List(vec![Value::String(istr("a")), Value::String(istr("b"))]),
        ),
        ("meta", open_record(&[("live", Value::Bool(true))])),
    ]);
    assert!(property_value_matches(&declaration, &conforming));

    // Nested record-of-list violation: a tag is not a string.
    let bad_list = open_record(&[
        ("id", Value::Int(1)),
        ("tags", Value::List(vec![Value::Int(2)])),
        ("meta", open_record(&[("live", Value::Bool(true))])),
    ]);
    assert!(!property_value_matches(&declaration, &bad_list));

    // Nested record-of-record violation: inner field wrong type.
    let bad_inner = open_record(&[
        ("id", Value::Int(1)),
        ("tags", Value::List(vec![Value::String(istr("a"))])),
        ("meta", open_record(&[("live", Value::Int(0))])),
    ]);
    assert!(!property_value_matches(&declaration, &bad_inner));
}

#[test]
fn closed_record_validates_recordtyped_positionally() {
    let declaration = closed_record_declaration();
    // Positional [host=String, port=Int] conforms.
    let conforming = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(Value::String(istr("h"))), Some(Value::Int(80))]
            .into_iter()
            .collect(),
    }));
    assert!(property_value_matches(&declaration, &conforming));

    // Optional port omitted (None at its position) conforms.
    let optional_omitted = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(Value::String(istr("h"))), None].into_iter().collect(),
    }));
    assert!(property_value_matches(&declaration, &optional_omitted));

    // Required host omitted (None at position 0) is rejected.
    let required_omitted = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [None, Some(Value::Int(80))].into_iter().collect(),
    }));
    assert!(!property_value_matches(&declaration, &required_omitted));

    // Wrong arity is rejected.
    let wrong_arity = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(Value::String(istr("h")))].into_iter().collect(),
    }));
    assert!(!property_value_matches(&declaration, &wrong_arity));
}

#[test]
fn closed_record_optional_field_accepts_explicit_null() {
    // An optional field set to an explicit NULL conforms — consistent with the absent and
    // positional-`None` cases — while a required field set to NULL is rejected.
    let declaration = closed_record_declaration();

    // Open form: optional `port` present as explicit NULL conforms.
    let port_null = open_record(&[("host", Value::String(istr("h"))), ("port", Value::Null)]);
    assert!(property_value_matches(&declaration, &port_null));

    // Open form: required `host` present as explicit NULL is rejected.
    let host_null = open_record(&[("host", Value::Null), ("port", Value::Int(80))]);
    assert!(!property_value_matches(&declaration, &host_null));

    // Positional form: optional `port` slot = Some(NULL) conforms.
    let positional_null = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(Value::String(istr("h"))), Some(Value::Null)]
            .into_iter()
            .collect(),
    }));
    assert!(property_value_matches(&declaration, &positional_null));
}

#[test]
fn open_bare_record_declaration_accepts_any_record() {
    // value_type RecordTyped with no declared field structure is permissive (mirrors the
    // legacy untyped LIST path).
    let declaration = PropertyTypeDef {
        name: istr("anything"),
        value_type: PropertyValueType::RecordTyped,
        list_element_type: None,
        required: false,
        default: None,
        immutable: false,
        record_field_types: None,
    };
    assert!(property_value_matches(
        &declaration,
        &open_record(&[("whatever", Value::Int(1))])
    ));
    assert!(property_value_matches(&declaration, &open_record(&[])));
    // A non-record value is still rejected by the coarse tag gate.
    assert!(!property_value_matches(&declaration, &Value::Int(1)));
}

#[test]
fn closed_record_violation_is_graph_type_violation_g2000() {
    // End-to-end: a non-conforming record property surfaces as G2000.
    let record_graph_type = GraphTypeDef {
        name: istr("record.validator.graph"),
        node_types: vec![crate::NodeTypeDef {
            name: istr("record.host"),
            key_labels: LabelSet::single(istr("Host")),
            properties: vec![closed_record_declaration()],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let shared = SharedGraph::builder(GraphId::new(8)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(istr("Host")),
                // host has the wrong type → closed-record violation.
                prop("config", open_record(&[("host", Value::Int(1))])),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    let violation = validate_entity_state(shared.read().as_ref(), &record_graph_type)
        .expect_err("non-conforming record must be rejected");
    assert!(matches!(
        violation,
        TypeViolation::PropertyTypeMismatch { ref property, .. } if *property == istr("config")
    ));
    assert_eq!(GraphError::from(violation).gqlstatus(), "G2000");
}
