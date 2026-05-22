use std::sync::Arc;

use selene_core::{
    Change, EdgeTypeDefV1, GraphId, GraphType, GraphTypeId, LabelSet, NodeId, NodeTypeDefV1,
    NodeTypeRef, PackLifecycleEvent, PredefinedValueType, PropertyDefV1, PropertyMap,
    RecordTypeDef, RecordTypeId, SchemaChange, SchemaPropertyIndexKind, Value, ValueType,
    ValueTypeCardinality, intern,
};
use selene_persist::RecoveryProvider;
use smallvec::smallvec;

use crate::core_provider::sections::{encode_graph_types, encode_meta, encode_schemas};
use crate::core_provider::{CORE_GTYP_SUB, CORE_META_SUB, CORE_SCMA_SUB, CoreProvider};
use crate::{
    EdgeEndpointDef, EdgeTypeDef, GraphError, GraphTypeDef, IndexProvider, NodeTypeDef,
    ProviderError, SharedGraph, SubTag, TypedIndexKind, ValidationMode,
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
            validation_mode: ValidationMode::Strict,
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
        source_node_type: EdgeEndpointDef::NodeType(0),
        target_node_type: EdgeEndpointDef::NodeType(0),
        properties: Vec::new(),
        validation_mode: ValidationMode::Strict,
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

fn core_string_property(name: &str, required: bool) -> PropertyDefV1 {
    PropertyDefV1 {
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Intent {
    Apply,
    NoopWithReason(&'static str),
    Reject(&'static str),
}

type SchemaChangeIntent = (fn() -> SchemaChange, Intent);

const LEGACY_PACK_NOOP: &str = "legacy; emitted pre-v1.0";
const PACK_LIFECYCLE_NOOP: &str = "audit history consumed by selene-pack";
const UNSUPPORTED_CORE_RECOVERY: &str = "unsupported by CORE graph recovery";

macro_rules! schema_intent {
    (apply $factory:ident) => {
        ($factory, Intent::Apply)
    };
    (noop $factory:ident, $reason:ident) => {
        ($factory, Intent::NoopWithReason($reason))
    };
    (reject $factory:ident, $reason:ident) => {
        ($factory, Intent::Reject($reason))
    };
}

const SCHEMA_CHANGE_INTENT: &[SchemaChangeIntent] = &[
    schema_intent!(reject intent_graph_created, UNSUPPORTED_CORE_RECOVERY),
    schema_intent!(reject intent_graph_dropped, UNSUPPORTED_CORE_RECOVERY),
    schema_intent!(reject intent_graph_type_created, UNSUPPORTED_CORE_RECOVERY),
    schema_intent!(reject intent_graph_type_dropped, UNSUPPORTED_CORE_RECOVERY),
    schema_intent!(apply intent_node_type_added),
    schema_intent!(apply intent_edge_type_added),
    schema_intent!(apply intent_node_type_dropped),
    schema_intent!(apply intent_edge_type_dropped),
    schema_intent!(reject intent_record_type_added, UNSUPPORTED_CORE_RECOVERY),
    schema_intent!(noop intent_pack_activated, LEGACY_PACK_NOOP),
    schema_intent!(noop intent_pack_deprecated, LEGACY_PACK_NOOP),
    schema_intent!(noop intent_pack_disabled, LEGACY_PACK_NOOP),
    schema_intent!(apply intent_property_index_created),
    schema_intent!(apply intent_property_index_dropped),
    schema_intent!(noop intent_pack_lifecycle, PACK_LIFECYCLE_NOOP),
    schema_intent!(apply intent_property_index_created_named),
];

fn intent_graph_created() -> SchemaChange {
    SchemaChange::GraphCreated {
        id: GraphId::new(101),
        name: intern("intent.graph").unwrap(),
        graph_type: Some(test_graph_type_id()),
    }
}

fn intent_graph_dropped() -> SchemaChange {
    SchemaChange::GraphDropped {
        id: GraphId::new(101),
    }
}

fn intent_graph_type_created() -> SchemaChange {
    SchemaChange::GraphTypeCreated {
        graph_type: GraphType::new(test_graph_type_id(), intern("intent.graph.type").unwrap()),
    }
}

fn intent_graph_type_dropped() -> SchemaChange {
    SchemaChange::GraphTypeDropped {
        id: test_graph_type_id(),
    }
}

fn intent_node_type_added() -> SchemaChange {
    let label = intern("IntentNode").unwrap();
    SchemaChange::NodeTypeAdded {
        graph_type: test_graph_type_id(),
        label,
        def: NodeTypeDefV1::new(LabelSet::single(label)),
    }
}

fn intent_edge_type_added() -> SchemaChange {
    let label = intern("INTENT_EDGE").unwrap();
    let endpoint = intern("IntentNode").unwrap();
    SchemaChange::EdgeTypeAdded {
        graph_type: test_graph_type_id(),
        label,
        def: EdgeTypeDefV1 {
            label,
            source_node_type: NodeTypeRef(endpoint),
            target_node_type: NodeTypeRef(endpoint),
            properties: smallvec![],
        },
    }
}

fn intent_node_type_dropped() -> SchemaChange {
    SchemaChange::NodeTypeDropped {
        graph_type: test_graph_type_id(),
        name: intern("IntentNode").unwrap(),
    }
}

fn intent_edge_type_dropped() -> SchemaChange {
    SchemaChange::EdgeTypeDropped {
        graph_type: test_graph_type_id(),
        name: intern("INTENT_EDGE").unwrap(),
    }
}

fn intent_record_type_added() -> SchemaChange {
    SchemaChange::RecordTypeAdded {
        graph_type: test_graph_type_id(),
        def: RecordTypeDef {
            id: RecordTypeId::new(1),
            name: intern("IntentRecord").unwrap(),
            fields: smallvec![],
        },
    }
}

fn intent_pack_activated() -> SchemaChange {
    SchemaChange::ProcedurePackActivated {
        pack_name: intern("intent_pack").unwrap(),
        version: intern("1.0.0").unwrap(),
    }
}

fn intent_pack_deprecated() -> SchemaChange {
    SchemaChange::ProcedurePackDeprecated {
        pack_name: intern("intent_pack").unwrap(),
        version: intern("1.0.0").unwrap(),
        reason: intern("intent reason").unwrap(),
    }
}

fn intent_pack_disabled() -> SchemaChange {
    SchemaChange::ProcedurePackDisabled {
        pack_name: intern("intent_pack").unwrap(),
        version: intern("1.0.0").unwrap(),
        reason: intern("intent reason").unwrap(),
    }
}

fn intent_property_index_created() -> SchemaChange {
    SchemaChange::PropertyIndexCreated {
        label: intern("IntentIndexedNode").unwrap(),
        property: intern("intentIndexedProperty").unwrap(),
        kind: SchemaPropertyIndexKind::I64,
    }
}

fn intent_property_index_dropped() -> SchemaChange {
    SchemaChange::PropertyIndexDropped {
        label: intern("IntentIndexedNode").unwrap(),
        property: intern("intentIndexedProperty").unwrap(),
    }
}

fn intent_property_index_created_named() -> SchemaChange {
    SchemaChange::PropertyIndexCreatedNamed {
        label: intern("IntentNamedIndexedNode").unwrap(),
        property: intern("intentNamedIndexedProperty").unwrap(),
        kind: SchemaPropertyIndexKind::String,
        name: Some(intern("intent_named_index").unwrap()),
    }
}

fn intent_pack_lifecycle() -> SchemaChange {
    SchemaChange::ProcedurePackLifecycle {
        event: PackLifecycleEvent::Activated {
            pack_name: intern("intent_pack").unwrap(),
            content_hash: [1_u8; 32],
            principal: intern("intent.principal").unwrap(),
            at: jiff::Timestamp::new(10, 0).unwrap(),
        },
    }
}

fn drive_handler_and_observe(change: SchemaChange) -> Intent {
    let mut state = super::RecoveryState::new();
    let result = state.apply_change(&Change::SchemaChanged {
        graph: GraphId::new(1),
        change: change.clone(),
    });
    match result {
        Err(_) => Intent::Reject(UNSUPPORTED_CORE_RECOVERY),
        Ok(())
            if !state.pending_schema_changes.is_empty()
                || !state.pending_property_index_changes.is_empty() =>
        {
            Intent::Apply
        }
        Ok(()) => noop_intent(&change),
    }
}

fn noop_intent(change: &SchemaChange) -> Intent {
    match change {
        SchemaChange::ProcedurePackActivated { .. }
        | SchemaChange::ProcedurePackDeprecated { .. }
        | SchemaChange::ProcedurePackDisabled { .. } => Intent::NoopWithReason(LEGACY_PACK_NOOP),
        SchemaChange::ProcedurePackLifecycle { .. } => Intent::NoopWithReason(PACK_LIFECYCLE_NOOP),
        SchemaChange::GraphCreated { .. }
        | SchemaChange::GraphDropped { .. }
        | SchemaChange::GraphTypeCreated { .. }
        | SchemaChange::GraphTypeDropped { .. }
        | SchemaChange::NodeTypeAdded { .. }
        | SchemaChange::NodeTypeAddedV2 { .. }
        | SchemaChange::EdgeTypeAdded { .. }
        | SchemaChange::EdgeTypeAddedV2 { .. }
        | SchemaChange::NodeTypeDropped { .. }
        | SchemaChange::EdgeTypeDropped { .. }
        | SchemaChange::RecordTypeAdded { .. }
        | SchemaChange::PropertyIndexCreated { .. }
        | SchemaChange::PropertyIndexDropped { .. }
        | SchemaChange::PropertyIndexCreatedNamed { .. } => {
            panic!(
                "{} is not a no-op schema-change intent",
                super::schema_replay::schema_change_variant(change)
            )
        }
    }
}

#[test]
fn recovery_intent_table_covers_every_schema_change_variant() {
    let mut seen = std::collections::BTreeSet::new();
    assert_eq!(SCHEMA_CHANGE_INTENT.len(), 16);

    for (factory, expected_intent) in SCHEMA_CHANGE_INTENT {
        let change = factory();
        let variant = super::schema_replay::schema_change_variant(&change);
        assert!(seen.insert(variant), "duplicate intent row for {variant}");
        let actual = drive_handler_and_observe(change.clone());
        assert_eq!(
            actual, *expected_intent,
            "intent mismatch for {variant}: {change:?}",
        );
    }
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
                def: NodeTypeDefV1 {
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
                def: EdgeTypeDefV1 {
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
    assert_eq!(edge_type.source_node_type, EdgeEndpointDef::NodeType(0));
    assert_eq!(edge_type.target_node_type, EdgeEndpointDef::NodeType(0));
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
                def: NodeTypeDefV1::new(LabelSet::single(intern("Sensor").unwrap())),
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
fn wal_replay_restores_named_property_index_metadata() {
    let provider = CoreProvider::new_for_recovery();
    let label = intern("NamedWalPerson").unwrap();
    let property = intern("name").unwrap();
    let name = intern("named_wal_person_name_idx").unwrap();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::PropertyIndexCreatedNamed {
                label,
                property,
                kind: SchemaPropertyIndexKind::String,
                name: Some(name),
            },
        },
    )
    .unwrap();

    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    let entries = recovered.iter_property_index_entries().collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![(label, property, TypedIndexKind::String, Some(name))]
    );
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
                def: NodeTypeDefV1::new(LabelSet::single(label)),
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
