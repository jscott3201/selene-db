use super::*;

use smallvec::smallvec;

use crate::{
    EdgeEndpointDef, EdgeTypeDef, NodeTypeDef, RecordFieldType, RecordFieldTypeDef,
    RecordFieldTypes,
};

fn optional_string_property(name: &str) -> PropertyTypeDef {
    PropertyTypeDef {
        name: db_string(name).unwrap(),
        value_type: selene_core::PropertyValueType::String,
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

fn core_string_property(name: &str, nullable: bool) -> PropertyDef {
    PropertyDef {
        name: db_string(name).unwrap(),
        value_type: ValueType::predefined(PredefinedValueType::String),
        nullable,
        default: None,
        immutable: false,
        unique: false,
        record_fields: None,
    }
}

fn core_int_property(name: &str, nullable: bool, default: Option<Value>) -> PropertyDef {
    PropertyDef {
        name: db_string(name).unwrap(),
        value_type: ValueType::predefined(PredefinedValueType::Int),
        nullable,
        default,
        immutable: false,
        unique: false,
        record_fields: None,
    }
}

fn three_node_type_graph() -> GraphTypeDef {
    let a = db_string("A").unwrap();
    let b = db_string("B").unwrap();
    let c = db_string("C").unwrap();
    let b_to_c = db_string("B_TO_C").unwrap();
    GraphTypeDef {
        name: db_string("recovery.alter-node.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: a.clone(),
                key_labels: LabelSet::single(a),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: b.clone(),
                key_labels: LabelSet::single(b),
                properties: vec![optional_string_property("existing")],
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: c.clone(),
                key_labels: LabelSet::single(c),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
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

fn legacy_descriptor_graph() -> GraphTypeDef {
    let mut graph_type = three_node_type_graph();
    let mut native_record = optional_string_property("legacy_record");
    native_record.value_type = selene_core::PropertyValueType::Record;
    let mut untyped_list = optional_string_property("legacy_list");
    untyped_list.value_type = selene_core::PropertyValueType::List;
    graph_type.node_types[1].properties = vec![native_record, untyped_list];
    graph_type
}

fn record_properties() -> [PropertyTypeDef; 2] {
    let mut open = optional_string_property("open_record");
    open.value_type = selene_core::PropertyValueType::RecordTyped;
    let mut typed = optional_string_property("typed_record");
    typed.value_type = selene_core::PropertyValueType::RecordTyped;
    typed.record_field_types = Some(RecordFieldTypes(vec![RecordFieldTypeDef {
        name: db_string("kind").unwrap(),
        field_type: RecordFieldType::Scalar(selene_core::PropertyValueType::String),
        required: true,
    }]));
    [open, typed]
}

#[test]
fn wal_replay_alters_non_last_node_type_without_retargeting_edges() {
    let dir = temp_dir("alter-node-type-preserves-slot");
    let graph_id = GraphId::new(1073);
    let base = three_node_type_graph();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let b = db_string("B").unwrap();
    let nickname = optional_string_property("nickname");
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(b.clone(), vec![nickname.clone()])
            .unwrap();
        txn.commit().unwrap().changes
    };
    assert!(matches!(
        changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAlteredV2 { label, .. },
            ..
        }] if *label == b
    ));
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let names: Vec<_> = graph_type
        .node_types
        .iter()
        .map(|node_type| node_type.name.as_str())
        .collect();
    assert_eq!(names, ["A", "B", "C"]);
    assert_eq!(
        graph_type.node_types[1].properties,
        [optional_string_property("existing"), nickname]
    );
    let edge_type = &graph_type.edge_types[0];
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(1));
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(2));
    assert_eq!(
        graph_type.node_types[edge_type.source_node_type.node_type_index().unwrap() as usize].name,
        db_string("B").unwrap()
    );
    assert_eq!(
        graph_type.node_types[edge_type.target_node_type.node_type_index().unwrap() as usize].name,
        db_string("C").unwrap()
    );
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_replay_rejects_malformed_node_type_alter_deltas() {
    let b = db_string("B").unwrap();
    let mut conflicting = core_string_property("existing", true);
    conflicting.unique = true;
    let cases = vec![
        (
            "unknown-identity",
            db_string("Missing").unwrap(),
            smallvec![core_string_property("new_property", true)],
            "unknown node type",
        ),
        (
            "conflicting-existing-property",
            b.clone(),
            smallvec![conflicting],
            "redefines property existing",
        ),
        (
            "required-appended-property",
            b.clone(),
            smallvec![core_string_property("required_new", false)],
            "appends required property",
        ),
        (
            "mismatched-default",
            b.clone(),
            smallvec![core_int_property(
                "bad_default",
                true,
                Some(Value::String(db_string("not-an-int").unwrap())),
            )],
            "default type String does not match declared Int",
        ),
        (
            "conflicting-delta-duplicates",
            b.clone(),
            smallvec![
                core_string_property("duplicate", true),
                core_int_property("duplicate", true, None),
            ],
            "redefines property duplicate",
        ),
    ];

    for (case, label, properties, expected) in cases {
        let dir = temp_dir(case);
        let graph_id = GraphId::new(1074);
        append_wal(
            &dir,
            0,
            &[Change::SchemaChanged {
                graph: graph_id,
                change: SchemaChange::NodeTypeAlteredV2 {
                    graph_type: GraphTypeId::new(1).unwrap(),
                    label,
                    properties,
                },
            }],
        );
        let error = match SharedGraph::recover_closed(&dir, graph_id, three_node_type_graph()) {
            Ok(_) => panic!("malformed ALTER NODE TYPE WAL payload must be rejected"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "{case}: expected {expected:?}, observed {error}"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn legacy_existing_record_and_untyped_list_survive_unrelated_alter_recovery() {
    let dir = temp_dir("alter-node-type-legacy-existing");
    let graph_id = GraphId::new(1075);
    let base = legacy_descriptor_graph();
    let legacy = base.node_types[1].properties.clone();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    write_snapshot(&dir, &shared, 1);
    let added = optional_string_property("added_later");
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(db_string("B").unwrap(), vec![added.clone()])
            .unwrap();
        txn.commit().unwrap().changes
    };
    assert!(matches!(
        changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::NodeTypeAlteredV2 { properties, .. },
            ..
        }] if properties.len() == 1 && properties[0].name == added.name
    ));
    append_wal(&dir, 1, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let properties = &recovered.graph_type().unwrap().node_types[1].properties;
    assert_eq!(&properties[..legacy.len()], legacy.as_slice());
    assert_eq!(properties.last(), Some(&added));
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn typed_and_open_record_additions_recover_exactly() {
    let dir = temp_dir("alter-node-type-record-delta");
    let graph_id = GraphId::new(1076);
    let base = three_node_type_graph();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let records = record_properties();
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(db_string("B").unwrap(), records.to_vec())
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let properties = &recovered.graph_type().unwrap().node_types[1].properties;
    assert_eq!(&properties[properties.len() - records.len()..], &records);
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn duplicate_node_type_alter_replay_is_idempotent() {
    let dir = temp_dir("alter-node-type-duplicate-replay");
    let graph_id = GraphId::new(1077);
    let property = core_string_property("nickname", true);
    let change = Change::SchemaChanged {
        graph: graph_id,
        change: SchemaChange::NodeTypeAlteredV2 {
            graph_type: GraphTypeId::new(1).unwrap(),
            label: db_string("B").unwrap(),
            properties: smallvec![property],
        },
    };
    append_wal(&dir, 0, &[change.clone(), change]);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, three_node_type_graph()).unwrap();
    let properties = &recovered.graph_type().unwrap().node_types[1].properties;
    assert_eq!(
        properties
            .iter()
            .filter(|property| property.name.as_str() == "nickname")
            .count(),
        1
    );
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recovered_default_descriptor_materializes_on_later_create() {
    let dir = temp_dir("alter-node-type-default-materialization");
    let graph_id = GraphId::new(1078);
    let base = three_node_type_graph();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let mut property = optional_string_property("status");
    property.default = Some(PropertyDefaultValue::String(db_string("new").unwrap()));
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_node_type(db_string("B").unwrap(), vec![property.clone()])
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    assert_eq!(
        recovered.graph_type().unwrap().node_types[1]
            .properties
            .last(),
        Some(&property)
    );
    let node = {
        let mut txn = recovered.begin_write();
        let node = txn
            .mutator()
            .create_node(
                LabelSet::single(db_string("B").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        txn.commit().unwrap();
        node
    };
    assert_eq!(
        recovered
            .read()
            .node_properties(node)
            .unwrap()
            .get(&property.name),
        Some(&Value::String(db_string("new").unwrap()))
    );
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}
