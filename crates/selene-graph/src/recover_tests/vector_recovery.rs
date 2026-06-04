use std::fs;

use selene_core::{
    Change, HnswIndexConfig, IvfIndexConfig, LabelSet, NodeId, PropertyValueType, SchemaChange,
    SchemaVectorIndexKind, Value, VectorValue, intern,
};

use super::*;
use crate::{VectorIndexConfig, VectorIndexKind};

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
fn recover_snapshot_preserves_vector_index_registration() {
    let dir = temp_dir("snapshot-vector-index");
    let label = intern("recover.vector.index.node").unwrap();
    let property = intern("recover.vector.index.embedding").unwrap();
    let shared = SharedGraph::builder(GraphId::new(40)).build().unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                prop(
                    "recover.vector.index.embedding",
                    Value::Vector(VectorValue::new(vec![1.0, 2.0, 3.0]).unwrap()),
                ),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_vector_index(label.clone(), property.clone(), VectorIndexKind::Flat, 3)
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(40)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.kind(), VectorIndexKind::Flat);
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_snapshot_preserves_hnsw_vector_index_registration() {
    let dir = temp_dir("snapshot-hnsw-vector-index");
    let label = intern("recover.hnsw.vector.index.node").unwrap();
    let property = intern("recover.hnsw.vector.index.embedding").unwrap();
    let config = HnswIndexConfig::new(24, 128);
    let shared = SharedGraph::builder(GraphId::new(42)).build().unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                prop(
                    "recover.hnsw.vector.index.embedding",
                    Value::Vector(VectorValue::new(vec![1.0, 0.0, 0.0]).unwrap()),
                ),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_vector_index_named_with_config(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswCosine,
            3,
            None,
            Some(config),
        )
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(42)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.kind(), VectorIndexKind::HnswCosine);
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.hnsw_config(), Some(config));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_snapshot_preserves_ivf_vector_index_registration() {
    let dir = temp_dir("snapshot-ivf-vector-index");
    let label = intern("recover.ivf.vector.index.node").unwrap();
    let property = intern("recover.ivf.vector.index.embedding").unwrap();
    let config = IvfIndexConfig::new(4);
    let shared = SharedGraph::builder(GraphId::new(43)).build().unwrap();
    {
        let mut txn = shared.begin_write();
        txn.mutator()
            .create_node(
                LabelSet::single(label.clone()),
                prop(
                    "recover.ivf.vector.index.embedding",
                    Value::Vector(VectorValue::new(vec![1.0, 0.0, 0.0]).unwrap()),
                ),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    shared
        .create_vector_index_named_with_configs(
            label.clone(),
            property.clone(),
            VectorIndexKind::IvfCosine,
            3,
            None,
            VectorIndexConfig::ivf(config),
        )
        .unwrap();
    write_snapshot(&dir, &shared, 1);

    let recovered = SharedGraph::recover(&dir, GraphId::new(43)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.kind(), VectorIndexKind::IvfCosine);
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.hnsw_config(), None);
    assert_eq!(index.ivf_config(), Some(config));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    assert_eq!(index.memory_usage().ivf_centroids, 1);
    assert_eq!(index.memory_usage().ivf_live_entries, 1);
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
fn recover_wal_only_replays_vector_index_registration() {
    let dir = temp_dir("wal-vector-index");
    let label = intern("recover.wal.vector.index.node").unwrap();
    let property = intern("recover.wal.vector.index.embedding").unwrap();
    let config = HnswIndexConfig::new(24, 128);
    append_wal(
        &dir,
        0,
        &[
            Change::NodeCreated {
                id: NodeId::new(1),
                labels: LabelSet::single(label.clone()),
                properties: prop(
                    "recover.wal.vector.index.embedding",
                    Value::Vector(VectorValue::new(vec![0.25, 0.5, 0.75]).unwrap()),
                ),
            },
            Change::SchemaChanged {
                graph: GraphId::new(41),
                change: SchemaChange::VectorIndexCreated {
                    label: label.clone(),
                    property: property.clone(),
                    kind: SchemaVectorIndexKind::HnswSquaredEuclidean,
                    dimension: 3,
                    name: None,
                    hnsw_config: Some(config),
                    ivf_config: None,
                },
            },
        ],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(41)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.kind(), VectorIndexKind::HnswSquaredEuclidean);
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.hnsw_config(), Some(config));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recover_wal_only_replays_ivf_vector_index_registration() {
    let dir = temp_dir("wal-ivf-vector-index");
    let label = intern("recover.wal.ivf.vector.index.node").unwrap();
    let property = intern("recover.wal.ivf.vector.index.embedding").unwrap();
    let config = IvfIndexConfig::new(4);
    append_wal(
        &dir,
        0,
        &[
            Change::NodeCreated {
                id: NodeId::new(1),
                labels: LabelSet::single(label.clone()),
                properties: prop(
                    "recover.wal.ivf.vector.index.embedding",
                    Value::Vector(VectorValue::new(vec![0.25, 0.5, 0.75]).unwrap()),
                ),
            },
            Change::SchemaChanged {
                graph: GraphId::new(44),
                change: SchemaChange::VectorIndexCreated {
                    label: label.clone(),
                    property: property.clone(),
                    kind: SchemaVectorIndexKind::IvfSquaredEuclidean,
                    dimension: 3,
                    name: None,
                    hnsw_config: None,
                    ivf_config: Some(config),
                },
            },
        ],
    );

    let recovered = SharedGraph::recover(&dir, GraphId::new(44)).unwrap();
    let snapshot = recovered.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.kind(), VectorIndexKind::IvfSquaredEuclidean);
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.hnsw_config(), None);
    assert_eq!(index.ivf_config(), Some(config));
    assert_eq!(index.rows().iter().collect::<Vec<_>>(), vec![0]);
    assert_eq!(index.memory_usage().ivf_live_entries, 1);
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
