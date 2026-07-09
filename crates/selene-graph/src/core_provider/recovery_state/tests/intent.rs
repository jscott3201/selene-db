use selene_core::{
    Change, EdgeTypeDef, EdgeTypeDefV1, GraphId, GraphType, LabelSet, NodeTypeDef, NodeTypeDefV1,
    NodeTypeRef, RecordTypeDef, RecordTypeId, SchemaChange, SchemaPropertyIndexKind,
    SchemaVectorIndexKind, db_string,
};
use smallvec::smallvec;

use super::test_graph_type_id;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Intent {
    Apply,
    Reject(&'static str),
}

type SchemaChangeIntent = (fn() -> SchemaChange, Intent);

const UNSUPPORTED_CORE_RECOVERY: &str = "unsupported by CORE graph recovery";

macro_rules! schema_intent {
    (apply $factory:ident) => {
        ($factory, Intent::Apply)
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
    schema_intent!(apply intent_property_index_created),
    schema_intent!(apply intent_property_index_dropped),
    schema_intent!(apply intent_property_index_created_named),
    schema_intent!(apply intent_edge_property_index_created),
    schema_intent!(apply intent_edge_property_index_dropped),
    schema_intent!(apply intent_node_type_added_v2),
    schema_intent!(apply intent_edge_type_added_v2),
    schema_intent!(apply intent_composite_property_index_created),
    schema_intent!(apply intent_composite_property_index_dropped),
    schema_intent!(apply intent_vector_index_created),
    schema_intent!(apply intent_vector_index_dropped),
    schema_intent!(apply intent_text_index_created),
    schema_intent!(apply intent_text_index_dropped),
    schema_intent!(apply intent_node_type_altered_v2),
];

fn intent_graph_created() -> SchemaChange {
    SchemaChange::GraphCreated {
        id: GraphId::new(101),
        name: db_string("intent.graph").unwrap(),
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
        graph_type: GraphType::new(
            test_graph_type_id(),
            db_string("intent.graph.type").unwrap(),
        ),
    }
}

fn intent_graph_type_dropped() -> SchemaChange {
    SchemaChange::GraphTypeDropped {
        id: test_graph_type_id(),
    }
}

fn intent_node_type_added() -> SchemaChange {
    let label = db_string("IntentNode").unwrap();
    SchemaChange::NodeTypeAdded {
        graph_type: test_graph_type_id(),
        label: label.clone(),
        def: NodeTypeDefV1::new(LabelSet::single(label)),
    }
}

fn intent_edge_type_added() -> SchemaChange {
    let label = db_string("INTENT_EDGE").unwrap();
    let endpoint = db_string("IntentNode").unwrap();
    SchemaChange::EdgeTypeAdded {
        graph_type: test_graph_type_id(),
        label: label.clone(),
        def: EdgeTypeDefV1 {
            label,
            source_node_type: NodeTypeRef(endpoint.clone()),
            target_node_type: NodeTypeRef(endpoint),
            properties: smallvec![],
        },
    }
}

fn intent_node_type_dropped() -> SchemaChange {
    SchemaChange::NodeTypeDropped {
        graph_type: test_graph_type_id(),
        name: db_string("IntentNode").unwrap(),
    }
}

fn intent_edge_type_dropped() -> SchemaChange {
    SchemaChange::EdgeTypeDropped {
        graph_type: test_graph_type_id(),
        name: db_string("INTENT_EDGE").unwrap(),
    }
}

fn intent_record_type_added() -> SchemaChange {
    SchemaChange::RecordTypeAdded {
        graph_type: test_graph_type_id(),
        def: RecordTypeDef {
            id: RecordTypeId::new(1),
            name: db_string("IntentRecord").unwrap(),
            fields: smallvec![],
        },
    }
}

fn intent_property_index_created() -> SchemaChange {
    SchemaChange::PropertyIndexCreated {
        label: db_string("IntentIndexedNode").unwrap(),
        property: db_string("intentIndexedProperty").unwrap(),
        kind: SchemaPropertyIndexKind::I64,
    }
}

fn intent_property_index_dropped() -> SchemaChange {
    SchemaChange::PropertyIndexDropped {
        label: db_string("IntentIndexedNode").unwrap(),
        property: db_string("intentIndexedProperty").unwrap(),
    }
}

fn intent_property_index_created_named() -> SchemaChange {
    SchemaChange::PropertyIndexCreatedNamed {
        label: db_string("IntentNamedIndexedNode").unwrap(),
        property: db_string("intentNamedIndexedProperty").unwrap(),
        kind: SchemaPropertyIndexKind::String,
        name: Some(db_string("intent_named_index").unwrap()),
    }
}

fn intent_edge_property_index_created() -> SchemaChange {
    SchemaChange::EdgePropertyIndexCreated {
        label: db_string("INTENT_INDEXED_EDGE").unwrap(),
        property: db_string("intentIndexedProperty").unwrap(),
        kind: SchemaPropertyIndexKind::I64,
        name: Some(db_string("intent_edge_index").unwrap()),
    }
}

fn intent_edge_property_index_dropped() -> SchemaChange {
    SchemaChange::EdgePropertyIndexDropped {
        label: db_string("INTENT_INDEXED_EDGE").unwrap(),
        property: db_string("intentIndexedProperty").unwrap(),
    }
}

fn intent_node_type_added_v2() -> SchemaChange {
    let label = db_string("IntentNodeV2").unwrap();
    SchemaChange::NodeTypeAddedV2 {
        graph_type: test_graph_type_id(),
        label: label.clone(),
        def: NodeTypeDef::new(LabelSet::single(label)),
    }
}

fn intent_edge_type_added_v2() -> SchemaChange {
    let label = db_string("INTENT_EDGE_V2").unwrap();
    let endpoint = db_string("IntentNode").unwrap();
    SchemaChange::EdgeTypeAddedV2 {
        graph_type: test_graph_type_id(),
        label: label.clone(),
        def: EdgeTypeDef::new(label, NodeTypeRef(endpoint.clone()), NodeTypeRef(endpoint)),
    }
}

fn intent_node_type_altered_v2() -> SchemaChange {
    SchemaChange::NodeTypeAlteredV2 {
        graph_type: test_graph_type_id(),
        label: db_string("IntentNode").unwrap(),
        properties: smallvec![],
    }
}

fn intent_composite_property_index_created() -> SchemaChange {
    SchemaChange::CompositePropertyIndexCreated {
        label: db_string("IntentCompositeIndexedNode").unwrap(),
        properties: smallvec![
            db_string("intentCompositeA").unwrap(),
            db_string("intentCompositeB").unwrap()
        ],
        kinds: smallvec![
            SchemaPropertyIndexKind::I64,
            SchemaPropertyIndexKind::String
        ],
        name: Some(db_string("intent_composite_index").unwrap()),
    }
}

fn intent_composite_property_index_dropped() -> SchemaChange {
    SchemaChange::CompositePropertyIndexDropped {
        label: db_string("IntentCompositeIndexedNode").unwrap(),
        properties: smallvec![
            db_string("intentCompositeA").unwrap(),
            db_string("intentCompositeB").unwrap()
        ],
    }
}

fn intent_vector_index_created() -> SchemaChange {
    SchemaChange::VectorIndexCreated {
        label: db_string("IntentVectorIndexedNode").unwrap(),
        property: db_string("intentEmbedding").unwrap(),
        kind: SchemaVectorIndexKind::Flat,
        dimension: 3,
        name: Some(db_string("intent_vector_index").unwrap()),
        hnsw_config: None,
        ivf_config: None,
    }
}

fn intent_vector_index_dropped() -> SchemaChange {
    SchemaChange::VectorIndexDropped {
        label: db_string("IntentVectorIndexedNode").unwrap(),
        property: db_string("intentEmbedding").unwrap(),
    }
}

fn intent_text_index_created() -> SchemaChange {
    SchemaChange::TextIndexCreated {
        label: db_string("IntentTextIndexedNode").unwrap(),
        property: db_string("intentBody").unwrap(),
        name: Some(db_string("intent_text_index").unwrap()),
    }
}

fn intent_text_index_dropped() -> SchemaChange {
    SchemaChange::TextIndexDropped {
        label: db_string("IntentTextIndexedNode").unwrap(),
        property: db_string("intentBody").unwrap(),
    }
}

fn drive_handler_and_observe(change: SchemaChange) -> Intent {
    let mut state = super::super::RecoveryState::new();
    let result = state.apply_change(&Change::SchemaChanged {
        graph: GraphId::new(1),
        change: change.clone(),
    });
    match result {
        Err(_) => Intent::Reject(UNSUPPORTED_CORE_RECOVERY),
        Ok(())
            if !state.pending_schema_changes.is_empty()
                || !state.pending_property_index_changes.is_empty()
                || !state.pending_composite_property_index_changes.is_empty()
                || !state.pending_vector_index_changes.is_empty()
                || !state.pending_text_index_changes.is_empty() =>
        {
            Intent::Apply
        }
        // Every recoverable schema change either queues pending state (Apply) or
        // is rejected. No variant is a silent no-op after the procedure-pack
        // teardown, so an `Ok(())` with nothing queued is a recovery bug.
        Ok(()) => panic!(
            "{} produced no recovery intent (neither applied nor rejected)",
            super::super::schema_replay::schema_change_variant(&change)
        ),
    }
}

#[test]
fn recovery_intent_table_covers_every_schema_change_variant() {
    let mut seen = std::collections::BTreeSet::new();
    assert_eq!(SCHEMA_CHANGE_INTENT.len(), SchemaChange::VARIANT_COUNT);

    for (factory, expected_intent) in SCHEMA_CHANGE_INTENT {
        let change = factory();
        let variant = super::super::schema_replay::schema_change_variant(&change);
        assert!(seen.insert(variant), "duplicate intent row for {variant}");
        let actual = drive_handler_and_observe(change.clone());
        assert_eq!(
            actual, *expected_intent,
            "intent mismatch for {variant}: {change:?}",
        );
    }
}
