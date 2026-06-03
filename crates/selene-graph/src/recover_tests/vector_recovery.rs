use std::fs;

use selene_core::{Change, LabelSet, NodeId, PropertyValueType, Value, VectorValue, intern};

use super::*;

fn vector_value() -> Value {
    Value::Vector(VectorValue::new(vec![0.25, 0.5, 0.75]).unwrap())
}

#[test]
fn recover_snapshot_preserves_vector_property() {
    let dir = temp_dir("snapshot-vector");
    let shared = SharedGraph::builder(GraphId::new(37)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(intern("recover.vector.node").unwrap()),
                prop("recover.vector", vector_value()),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(37)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    expect_prop(
        snapshot.node_properties(NodeId::new(1)).unwrap(),
        "recover.vector",
        &vector_value(),
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_wal_only_replays_vector_property() {
    let dir = temp_dir("wal-vector");
    append_wal(
        &dir,
        0,
        &[Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(intern("recover.wal.vector.node").unwrap()),
            properties: prop("recover.wal.vector", vector_value()),
        }],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(38)).unwrap();
    let snapshot = recovered.read();
    assert_eq!(snapshot.node_count(), 1);
    expect_prop(
        snapshot.node_properties(NodeId::new(1)).unwrap(),
        "recover.wal.vector",
        &vector_value(),
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_closed_wal_only_preserves_vector_property_type() {
    let dir = temp_dir("closed-schema-vector-wal-only");
    let graph_id = GraphId::new(39);
    let base = empty_closed_graph_type();
    let shared = SharedGraph::builder(graph_id)
        .bound_to(base.clone())
        .unwrap()
        .build()
        .unwrap();
    let sensor = intern("VectorSensor").unwrap();
    let embedding = intern("embedding").unwrap();
    let changes = {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node_type(
                sensor.clone(),
                LabelSet::single(sensor),
                vec![PropertyTypeDef {
                    name: embedding.clone(),
                    value_type: PropertyValueType::Vector,
                    list_element_type: None,
                    required: true,
                    default: None,
                    immutable: false,
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
    assert_eq!(property.name, embedding);
    assert_eq!(property.value_type, PropertyValueType::Vector);
    assert_eq!(property.list_element_type, None);
    assert!(property.required);
    let _ = fs::remove_dir_all(dir);
}
