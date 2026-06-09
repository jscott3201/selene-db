use std::fs;

use selene_core::{Change, JsonValue, LabelSet, NodeId, PropertyValueType, Value, db_string};

use super::*;

fn json_value() -> Value {
    Value::Json(
        JsonValue::new(serde_json::json!({
            "kind": "episodic",
            "score": 7,
            "tags": ["agent", "memory"],
        }))
        .unwrap(),
    )
}

#[test]
fn recover_snapshot_preserves_json_property() {
    let dir = temp_dir("snapshot-json");
    let shared = SharedGraph::builder(GraphId::new(48)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(db_string("recover.json.node").unwrap()),
                prop("recover.json", json_value()),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(48)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    expect_prop(
        snapshot.node_properties(NodeId::new(1)).unwrap(),
        "recover.json",
        &json_value(),
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_wal_only_replays_json_property() {
    let dir = temp_dir("wal-json");
    append_wal(
        &dir,
        0,
        &[Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(db_string("recover.wal.json.node").unwrap()),
            properties: prop("recover.wal.json", json_value()),
        }],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(49)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    expect_prop(
        snapshot.node_properties(NodeId::new(1)).unwrap(),
        "recover.wal.json",
        &json_value(),
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_wal_only_preserves_json_property_type() {
    let dir = temp_dir("closed-schema-json-wal-only");
    let graph_id = GraphId::new(50);
    let base = empty_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let memory = db_string("Memory").unwrap();
    let payload = db_string("payload").unwrap();
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node_type(
                memory.clone(),
                LabelSet::single(memory),
                vec![PropertyTypeDef {
                    name: payload.clone(),
                    value_type: PropertyValueType::Json,
                    list_element_type: None,
                    required: true,
                    default: Some(PropertyDefaultValue::from_value(&json_value()).unwrap()),
                    immutable: false,
                    unique: false,
                    decimal_type: None,
                    byte_string_type: None,
                    record_field_types: None,
                }],
                ValidationMode::Strict,
            )
            .unwrap();
        txn.commit().unwrap().changes
    };
    append_wal(&dir, 0, &changes);

    let recovered = SharedGraph::recover_closed(&dir, graph_id, base).unwrap();
    let graph_type = recovered.graph_type().unwrap();
    let property = &graph_type.node_types[0].properties[0];
    assert_eq!(property.name, payload);
    assert_eq!(property.value_type, PropertyValueType::Json);
    assert_eq!(property.list_element_type, None);
    assert_eq!(
        property.default,
        Some(PropertyDefaultValue::Json(
            db_string(r#"{"kind":"episodic","score":7,"tags":["agent","memory"]}"#).unwrap()
        ))
    );
    assert!(property.required);
    let _ = fs::remove_dir_all(dir);
}
