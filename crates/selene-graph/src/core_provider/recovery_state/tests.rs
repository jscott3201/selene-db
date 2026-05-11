use std::sync::Arc;

use selene_core::{
    Change, GraphId, GraphTypeId, LabelSet, NodeId, NodeTypeRef, PackLifecycleEvent,
    PredefinedValueType, PropertyDef, PropertyMap, SchemaChange, SchemaPropertyIndexKind, Value,
    ValueType, ValueTypeCardinality, intern,
};
use selene_persist::RecoveryProvider;
use smallvec::smallvec;

use crate::core_provider::sections::{encode_graph_types, encode_meta, encode_schemas};
use crate::core_provider::{CORE_GTYP_SUB, CORE_META_SUB, CORE_SCMA_SUB, CoreProvider};
use crate::{
    EdgeTypeDef, GraphError, GraphTypeDef, IndexProvider, NodeTypeDef, ProviderError, SharedGraph,
    SubTag, TypedIndexKind,
};

fn test_graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).unwrap()
}

fn empty_runtime_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: intern("core.recovery.graph").unwrap(),
        node_types: Vec::new(),
        edge_types: Vec::new(),
    }
}

fn person_runtime_graph_type() -> GraphTypeDef {
    let person = intern("Person").unwrap();
    GraphTypeDef {
        name: intern("core.recovery.person.graph").unwrap(),
        node_types: vec![NodeTypeDef {
            name: person,
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
        }],
        edge_types: Vec::new(),
    }
}

fn person_knows_runtime_graph_type() -> GraphTypeDef {
    let knows = intern("KNOWS").unwrap();
    let mut graph_type = person_runtime_graph_type();
    graph_type.edge_types.push(EdgeTypeDef {
        name: knows,
        label: knows,
        source_node_type: 0,
        target_node_type: 0,
        properties: Vec::new(),
    });
    graph_type
}

fn closed_graph_snapshot(graph_type: GraphTypeDef) -> crate::SeleneGraph {
    SharedGraph::builder(GraphId::new(1))
        .bound_to(graph_type)
        .unwrap()
        .build()
        .unwrap()
        .read()
        .as_ref()
        .clone()
}

fn load_closed_snapshot(provider: &CoreProvider, graph: &crate::SeleneGraph) {
    IndexProvider::read_section(
        provider,
        SubTag(CORE_GTYP_SUB),
        &encode_graph_types(graph).unwrap(),
    )
    .unwrap();
    IndexProvider::read_section(
        provider,
        SubTag(CORE_META_SUB),
        &encode_meta(&graph.meta, 0).unwrap(),
    )
    .unwrap();
}

fn core_string_property(name: &str, required: bool) -> PropertyDef {
    PropertyDef {
        name: intern(name).unwrap(),
        value_type: ValueType {
            predefined: Some(PredefinedValueType::String),
            union: None,
            list_of: None,
            record: None,
            not_null: required,
            cardinality: ValueTypeCardinality::ExactlyOne,
        },
        nullable: !required,
        default: None,
    }
}

fn props(pairs: impl IntoIterator<Item = (selene_core::IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

#[test]
fn wal_replay_applies_node_type_added_to_graph_type() {
    let base = empty_runtime_graph_type();
    let snapshot = closed_graph_snapshot(base.clone());
    let provider = CoreProvider::new_for_recovery();
    load_closed_snapshot(provider.as_ref(), &snapshot);
    let sensor = intern("Sensor").unwrap();

    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::NodeTypeAdded {
                graph_type: test_graph_type_id(),
                label: sensor,
                def: selene_core::NodeTypeDef {
                    labels: LabelSet::single(sensor),
                    properties: smallvec![core_string_property("serial", true)],
                    key: None,
                },
            },
        },
    )
    .unwrap();

    let recovered = provider
        .finish_recovery(GraphId::new(1), Some(Arc::new(base)))
        .unwrap();
    let graph_type = recovered.meta.bound_type.as_ref().unwrap();
    assert_eq!(graph_type.node_types[0].name, sensor);
    assert_eq!(
        graph_type.node_types[0].properties[0].name.as_str(),
        "serial"
    );
    assert!(graph_type.node_types[0].properties[0].required);
}

#[test]
fn wal_replay_applies_edge_type_added() {
    let base = person_runtime_graph_type();
    let snapshot = closed_graph_snapshot(base.clone());
    let provider = CoreProvider::new_for_recovery();
    load_closed_snapshot(provider.as_ref(), &snapshot);
    let knows = intern("KNOWS").unwrap();
    let since = core_string_property("since", false);

    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::EdgeTypeAdded {
                graph_type: test_graph_type_id(),
                label: knows,
                def: selene_core::EdgeTypeDef {
                    label: knows,
                    source_node_type: NodeTypeRef(intern("Person").unwrap()),
                    target_node_type: NodeTypeRef(intern("Person").unwrap()),
                    properties: smallvec![since],
                },
            },
        },
    )
    .unwrap();

    let recovered = provider
        .finish_recovery(GraphId::new(1), Some(Arc::new(base)))
        .unwrap();
    let edge_type = &recovered.meta.bound_type.as_ref().unwrap().edge_types[0];
    assert_eq!(edge_type.name, knows);
    assert_eq!(edge_type.label, knows);
    assert_eq!(edge_type.source_node_type, 0);
    assert_eq!(edge_type.target_node_type, 0);
    assert_eq!(edge_type.properties[0].name.as_str(), "since");
}

#[test]
fn wal_replay_applies_node_type_dropped() {
    let base = person_runtime_graph_type();
    let snapshot = closed_graph_snapshot(base.clone());
    let provider = CoreProvider::new_for_recovery();
    load_closed_snapshot(provider.as_ref(), &snapshot);

    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::NodeTypeDropped {
                graph_type: test_graph_type_id(),
                name: intern("Person").unwrap(),
            },
        },
    )
    .unwrap();

    let recovered = provider
        .finish_recovery(GraphId::new(1), Some(Arc::new(base)))
        .unwrap();
    assert!(
        recovered
            .meta
            .bound_type
            .as_ref()
            .unwrap()
            .node_types
            .is_empty()
    );
}

#[test]
fn wal_replay_applies_edge_type_dropped() {
    let base = person_knows_runtime_graph_type();
    let snapshot = closed_graph_snapshot(base.clone());
    let provider = CoreProvider::new_for_recovery();
    load_closed_snapshot(provider.as_ref(), &snapshot);

    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::EdgeTypeDropped {
                graph_type: test_graph_type_id(),
                name: intern("KNOWS").unwrap(),
            },
        },
    )
    .unwrap();

    let recovered = provider
        .finish_recovery(GraphId::new(1), Some(Arc::new(base)))
        .unwrap();
    assert!(
        recovered
            .meta
            .bound_type
            .as_ref()
            .unwrap()
            .edge_types
            .is_empty()
    );
}

#[test]
fn wal_replay_node_type_added_against_open_snapshot_returns_inconsistent() {
    let provider = CoreProvider::new_for_recovery();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::NodeTypeAdded {
                graph_type: test_graph_type_id(),
                label: intern("Sensor").unwrap(),
                def: selene_core::NodeTypeDef::new(LabelSet::single(intern("Sensor").unwrap())),
            },
        },
    )
    .unwrap();

    let err = provider
        .finish_recovery(GraphId::new(1), None)
        .expect_err("open graph cannot replay catalog DDL");

    assert!(matches!(
        err,
        GraphError::Provider(ProviderError::Inconsistent { reason })
            if reason.contains("WAL NodeTypeAdded references missing graph type index 0")
    ));
}

#[test]
fn wal_replay_procedure_pack_lifecycle_is_graph_state_noop() {
    let provider = CoreProvider::new_for_recovery();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::ProcedurePackLifecycle {
                event: PackLifecycleEvent::Activated {
                    pack_name: intern("demo_pack").unwrap(),
                    content_hash: [0_u8; 32],
                    principal: intern("recovery.principal").unwrap(),
                    at: jiff::Timestamp::new(1, 0).unwrap(),
                },
            },
        },
    )
    .unwrap();

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    assert_eq!(recovered.node_count(), 0);
    assert_eq!(recovered.edge_count(), 0);
    assert_eq!(recovered.property_index_count(), 0);
}

#[test]
fn wal_replay_restores_property_index_created_after_node_state() {
    let provider = CoreProvider::new_for_recovery();
    let label = intern("RecoveredPerson").unwrap();
    let property = intern("age").unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(label),
            properties: props([(property, Value::Int(42))]),
        },
    )
    .unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::PropertyIndexCreated {
                label,
                property,
                kind: SchemaPropertyIndexKind::I64,
            },
        },
    )
    .unwrap();

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    let rows = recovered
        .nodes_with_property_eq(&label, &property, &Value::Int(42))
        .unwrap();
    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
    assert_eq!(recovered.property_index_count(), 1);
}

#[test]
fn wal_replay_drops_property_index_registered_in_snapshot_scma() {
    let shared = SharedGraph::new(GraphId::new(1));
    let label = intern("SnapshotPerson").unwrap();
    let property = intern("age").unwrap();
    shared
        .create_property_index(label, property, TypedIndexKind::I64)
        .unwrap();
    let snapshot = shared.read();
    let provider = CoreProvider::new_for_recovery();
    IndexProvider::read_section(
        provider.as_ref(),
        SubTag(CORE_SCMA_SUB),
        &encode_schemas(&snapshot).unwrap(),
    )
    .unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::PropertyIndexDropped { label, property },
        },
    )
    .unwrap();

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    assert_eq!(recovered.property_index_count(), 0);
    assert!(recovered.property_index_for(&label, &property).is_none());
}

#[test]
fn wal_replay_property_index_create_drop_create_sequence_uses_last_event() {
    let provider = CoreProvider::new_for_recovery();
    let label = intern("SequencePerson").unwrap();
    let property = intern("age").unwrap();
    for change in [
        SchemaChange::PropertyIndexCreated {
            label,
            property,
            kind: SchemaPropertyIndexKind::I64,
        },
        SchemaChange::PropertyIndexDropped { label, property },
        SchemaChange::PropertyIndexCreated {
            label,
            property,
            kind: SchemaPropertyIndexKind::I64,
        },
    ] {
        RecoveryProvider::on_change(
            provider.as_ref(),
            &Change::SchemaChanged {
                graph: GraphId::new(1),
                change,
            },
        )
        .unwrap();
    }

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    assert!(recovered.property_index_for(&label, &property).is_some());
    assert_eq!(recovered.property_index_count(), 1);
}

#[test]
fn wal_replay_applies_catalog_ddl_before_property_index_queue() {
    let base = empty_runtime_graph_type();
    let snapshot = closed_graph_snapshot(base.clone());
    let provider = CoreProvider::new_for_recovery();
    load_closed_snapshot(provider.as_ref(), &snapshot);
    let label = intern("IndexedSensor").unwrap();
    let property = intern("reading").unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::NodeTypeAdded {
                graph_type: test_graph_type_id(),
                label,
                def: selene_core::NodeTypeDef::new(LabelSet::single(label)),
            },
        },
    )
    .unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::PropertyIndexCreated {
                label,
                property,
                kind: SchemaPropertyIndexKind::I64,
            },
        },
    )
    .unwrap();

    let recovered = provider
        .finish_recovery(GraphId::new(1), Some(Arc::new(base)))
        .unwrap();
    assert_eq!(
        recovered.meta.bound_type.as_ref().unwrap().node_types[0].name,
        label
    );
    assert!(recovered.property_index_for(&label, &property).is_some());
}

#[test]
fn wal_replay_property_index_create_is_lenient_for_later_kind_drift() {
    let provider = CoreProvider::new_for_recovery();
    let label = intern("DriftPerson").unwrap();
    let property = intern("age").unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::PropertyIndexCreated {
                label,
                property,
                kind: SchemaPropertyIndexKind::I64,
            },
        },
    )
    .unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(label),
            properties: props([(property, Value::String(intern("not-an-int").unwrap()))]),
        },
    )
    .unwrap();

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    let rows = recovered
        .nodes_with_property_eq(&label, &property, &Value::Int(42))
        .unwrap();
    assert!(rows.is_empty());
    assert!(recovered.property_index_for(&label, &property).is_some());
}
