use super::*;

use selene_core::{EdgeEndpointDef as CoreEdgeEndpointDef, NodeTypeRef, PropertyValueType};
use smallvec::{SmallVec, smallvec};

use crate::{EdgeEndpointDef, EdgeTypeDef, NodeTypeDef};

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

fn core_property(
    name: &str,
    value_type: PredefinedValueType,
    nullable: bool,
    default: Option<Value>,
) -> PropertyDef {
    PropertyDef {
        name: db_string(name).unwrap(),
        value_type: ValueType::predefined(value_type),
        nullable,
        default,
        immutable: false,
        unique: false,
        record_fields: None,
    }
}

fn edge_type(name: &str, source: u32, target: u32) -> EdgeTypeDef {
    let name = db_string(name).unwrap();
    EdgeTypeDef {
        name: name.clone(),
        label: name,
        source_node_type: EdgeEndpointDef::NodeType(source),
        target_node_type: EdgeEndpointDef::NodeType(target),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
    }
}

fn three_edge_type_graph() -> GraphTypeDef {
    let mut b = edge_type("EDGE_B", 1, 2);
    b.properties.push(optional_string_property("existing"));
    GraphTypeDef {
        name: db_string("recovery.alter-edge.graph").unwrap(),
        node_types: ["A", "B", "C"]
            .into_iter()
            .map(|name| {
                let name = db_string(name).unwrap();
                NodeTypeDef {
                    name: name.clone(),
                    key_labels: LabelSet::single(name),
                    properties: Vec::new(),
                    validation_mode: ValidationMode::Strict,
                }
            })
            .collect(),
        edge_types: vec![edge_type("EDGE_A", 0, 1), b, edge_type("EDGE_C", 2, 0)],
    }
}

fn alter_change(
    graph_id: GraphId,
    name: &str,
    source_node_type: Option<CoreEdgeEndpointDef>,
    target_node_type: Option<CoreEdgeEndpointDef>,
    properties: SmallVec<[PropertyDef; 4]>,
) -> Change {
    Change::SchemaChanged {
        graph: graph_id,
        change: SchemaChange::EdgeTypeAlteredV2 {
            graph_type: GraphTypeId::new(1).unwrap(),
            name: db_string(name).unwrap(),
            source_node_type,
            target_node_type,
            properties,
        },
    }
}

#[test]
fn wal_replay_alters_middle_edge_type_in_place() {
    let dir = temp_dir("alter-edge-type-preserves-slot");
    let graph_id = GraphId::new(1080);
    let base = three_edge_type_graph();
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
            .alter_edge_type(
                db_string("EDGE_B").unwrap(),
                Some(EdgeEndpointDef::one_of([0, 1])),
                Some(EdgeEndpointDef::Any),
                vec![property],
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    assert!(matches!(
        changes.as_slice(),
        [Change::SchemaChanged {
            change: SchemaChange::EdgeTypeAlteredV2 { .. },
            ..
        }]
    ));
    let live_type = shared.graph_type().unwrap();
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let recovered_type = recovered.graph_type().unwrap();
    let names: Vec<_> = recovered_type
        .edge_types
        .iter()
        .map(|edge_type| edge_type.name.as_str())
        .collect();
    assert_eq!(names, ["EDGE_A", "EDGE_B", "EDGE_C"]);
    assert_eq!(recovered_type.as_ref(), live_type.as_ref());
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn duplicate_edge_type_alter_replay_is_idempotent_and_canonicalizes_endpoints() {
    let dir = temp_dir("alter-edge-type-duplicate-replay");
    let graph_id = GraphId::new(1081);
    let change = alter_change(
        graph_id,
        "EDGE_B",
        Some(CoreEdgeEndpointDef::OneOf(smallvec![
            NodeTypeRef(db_string("B").unwrap()),
            NodeTypeRef(db_string("A").unwrap()),
            NodeTypeRef(db_string("B").unwrap()),
        ])),
        None,
        smallvec![core_property(
            "since",
            PredefinedValueType::String,
            true,
            None,
        )],
    );
    append_wal(&dir, 0, &[change.clone(), change]);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, three_edge_type_graph()).unwrap();
    let edge_type = &recovered.graph_type().unwrap().edge_types[1];
    assert_eq!(
        edge_type.source_node_type,
        EdgeEndpointDef::OneOf(vec![0, 1])
    );
    assert_eq!(
        edge_type
            .properties
            .iter()
            .filter(|property| property.name.as_str() == "since")
            .count(),
        1
    );
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_replay_rejects_malformed_edge_type_alter_deltas() {
    let graph_id = GraphId::new(1082);
    let mut conflicting = core_property("existing", PredefinedValueType::String, true, None);
    conflicting.unique = true;
    let cases = vec![
        (
            "unknown-edge",
            alter_change(graph_id, "MISSING", None, None, smallvec![]),
            "unknown edge type",
        ),
        (
            "unknown-source",
            alter_change(
                graph_id,
                "EDGE_B",
                Some(CoreEdgeEndpointDef::NodeType(NodeTypeRef(
                    db_string("MISSING").unwrap(),
                ))),
                None,
                smallvec![],
            ),
            "unknown source node type",
        ),
        (
            "empty-one-of",
            alter_change(
                graph_id,
                "EDGE_B",
                Some(CoreEdgeEndpointDef::OneOf(smallvec![])),
                None,
                smallvec![],
            ),
            "empty node-type set",
        ),
        (
            "narrowing-source",
            alter_change(
                graph_id,
                "EDGE_B",
                Some(CoreEdgeEndpointDef::NodeType(NodeTypeRef(
                    db_string("A").unwrap(),
                ))),
                None,
                smallvec![],
            ),
            "source endpoint narrows",
        ),
        (
            "conflicting-existing-property",
            alter_change(graph_id, "EDGE_B", None, None, smallvec![conflicting]),
            "redefines property existing",
        ),
        (
            "required-property",
            alter_change(
                graph_id,
                "EDGE_B",
                None,
                None,
                smallvec![core_property(
                    "required_new",
                    PredefinedValueType::String,
                    false,
                    None,
                )],
            ),
            "appends required property",
        ),
        (
            "invalid-default",
            alter_change(
                graph_id,
                "EDGE_B",
                None,
                None,
                smallvec![core_property(
                    "bad_default",
                    PredefinedValueType::Int,
                    true,
                    Some(Value::String(db_string("not-an-int").unwrap())),
                )],
            ),
            "default type String does not match declared Int",
        ),
        (
            "conflicting-delta-duplicates",
            alter_change(
                graph_id,
                "EDGE_B",
                None,
                None,
                smallvec![
                    core_property("duplicate", PredefinedValueType::String, true, None),
                    core_property("duplicate", PredefinedValueType::Int, true, None),
                ],
            ),
            "redefines property duplicate",
        ),
    ];

    for (case, change, expected) in cases {
        let dir = temp_dir(case);
        append_wal(&dir, 0, &[change]);
        let error = match SharedGraph::recover_closed(&dir, graph_id, three_edge_type_graph()) {
            Ok(_) => panic!("malformed ALTER EDGE TYPE WAL payload must be rejected"),
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
fn unrelated_edge_alter_preserves_legacy_property_descriptors() {
    let dir = temp_dir("alter-edge-type-legacy-properties");
    let graph_id = GraphId::new(1083);
    let mut base = three_edge_type_graph();
    let mut record = optional_string_property("legacy_record");
    record.value_type = PropertyValueType::Record;
    let mut list = optional_string_property("legacy_list");
    list.value_type = PropertyValueType::List;
    base.edge_types[1].properties = vec![record, list];
    let legacy = base.edge_types[1].properties.clone();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let added = optional_string_property("added");
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .alter_edge_type(
                db_string("EDGE_B").unwrap(),
                Some(EdgeEndpointDef::Any),
                None,
                vec![added.clone()],
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let properties = &recovered.graph_type().unwrap().edge_types[1].properties;
    assert_eq!(&properties[..legacy.len()], legacy.as_slice());
    assert_eq!(properties.last(), Some(&added));
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn altered_endpoint_references_resolve_by_node_type_name() {
    let dir = temp_dir("alter-edge-type-name-resolution");
    let graph_id = GraphId::new(1084);
    let colliding_label = db_string("collision").unwrap();
    let first_name = db_string("TypeA").unwrap();
    let second_name = colliding_label.clone();
    let base = GraphTypeDef {
        name: db_string("recovery.alter-edge.collision.graph").unwrap(),
        node_types: vec![
            NodeTypeDef {
                name: first_name.clone(),
                key_labels: LabelSet::single(colliding_label),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
            NodeTypeDef {
                name: second_name.clone(),
                key_labels: LabelSet::single(db_string("Other").unwrap()),
                properties: Vec::new(),
                validation_mode: ValidationMode::Strict,
            },
        ],
        edge_types: vec![edge_type("EDGE_B", 0, 0)],
    };
    let change = alter_change(
        graph_id,
        "EDGE_B",
        Some(CoreEdgeEndpointDef::OneOf(smallvec![
            NodeTypeRef(first_name),
            NodeTypeRef(second_name),
        ])),
        None,
        smallvec![],
    );
    append_wal(&dir, 0, &[change]);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    assert_eq!(
        recovered.graph_type().unwrap().edge_types[0].source_node_type,
        EdgeEndpointDef::OneOf(vec![0, 1])
    );
    drop(recovered);
    let _ = fs::remove_dir_all(dir);
}
