use std::sync::Arc;

use arc_swap::ArcSwap;
use selene_core::{
    Change, EdgeId, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap, SchemaChange,
    Value, intern,
};

use super::sections::{
    decode_edges, decode_meta, decode_nodes, encode_edges, encode_meta, encode_nodes,
    ensure_section_within_cap,
};
use super::*;
use crate::{GraphError, SharedGraph};

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(name).unwrap(), value)]).unwrap()
}

fn full_value_property_map(prefix: &str) -> PropertyMap {
    PropertyMap::from_pairs([
        (
            intern(&format!("{prefix}.bool")).unwrap(),
            Value::Bool(true),
        ),
        (intern(&format!("{prefix}.int")).unwrap(), Value::Int(-7)),
        (
            intern(&format!("{prefix}.float")).unwrap(),
            Value::Float(1.25),
        ),
        (
            intern(&format!("{prefix}.string")).unwrap(),
            Value::String(intern("core.values.string").unwrap()),
        ),
        (
            intern(&format!("{prefix}.decimal")).unwrap(),
            Value::Decimal("123.45".parse().unwrap()),
        ),
        (
            intern(&format!("{prefix}.bytes")).unwrap(),
            Value::Bytes(Arc::from([1_u8, 2, 3, 4])),
        ),
        (
            intern(&format!("{prefix}.uuid")).unwrap(),
            Value::Uuid(uuid::Uuid::from_u128(42)),
        ),
        (
            intern(&format!("{prefix}.zoned_datetime")).unwrap(),
            Value::ZonedDateTime(
                "2026-05-07T12:34:56-04:00[America/New_York]"
                    .parse()
                    .unwrap(),
            ),
        ),
        (
            intern(&format!("{prefix}.date")).unwrap(),
            Value::Date("2026-05-07".parse().unwrap()),
        ),
        (
            intern(&format!("{prefix}.local_datetime")).unwrap(),
            Value::LocalDateTime("2026-05-07T12:34:56".parse().unwrap()),
        ),
        (
            intern(&format!("{prefix}.local_time")).unwrap(),
            Value::LocalTime("12:34:56".parse().unwrap()),
        ),
        (
            intern(&format!("{prefix}.duration")).unwrap(),
            Value::Duration("PT1H2M3S".parse().unwrap()),
        ),
    ])
    .unwrap()
}

fn graph_with_node() -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    let id = {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(intern("core.node").unwrap()),
                prop("core.name", Value::Int(7)),
            )
            .unwrap()
    };
    assert_eq!(id, NodeId::new(1));
    txn.commit().unwrap();
    shared.read().as_ref().clone()
}

fn graph_with_edge() -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        let a = mutator
            .create_node(
                LabelSet::single(intern("core.a").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        let b = mutator
            .create_node(
                LabelSet::single(intern("core.b").unwrap()),
                PropertyMap::new(),
            )
            .unwrap();
        mutator
            .create_edge(
                intern("core.edge").unwrap(),
                a,
                b,
                prop("core.weight", Value::Int(3)),
            )
            .unwrap();
    }
    txn.commit().unwrap();
    shared.read().as_ref().clone()
}

#[test]
fn new_for_live_holds_snapshot_pointer() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(Arc::clone(&snapshot));
    let inner = provider.inner.lock();
    match &*inner {
        CoreInner::Live { snapshot: observed } => assert!(Arc::ptr_eq(observed, &snapshot)),
        CoreInner::Recovery { .. } => panic!("expected live provider"),
    }
}

#[test]
fn new_for_recovery_starts_empty() {
    let provider = CoreProvider::new_for_recovery();
    let graph = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert_eq!(graph.node_count(), 0);
    assert_eq!(graph.edge_count(), 0);
}

#[test]
fn finish_recovery_on_live_mode_is_error() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(snapshot);
    assert!(matches!(
        provider.finish_recovery(GraphId::new(1)),
        Err(GraphError::Provider(ProviderError::Inconsistent { reason }))
            if reason.contains("finish_recovery called on live-mode")
    ));
}

#[test]
fn encode_decode_round_trip_meta() {
    let graph = graph_with_node();
    let bytes = encode_meta(&graph.meta, 9).unwrap();
    let payload = decode_meta(&bytes).unwrap();
    assert_eq!(payload.meta, graph.meta);
    assert_eq!(payload.sequence, 9);
}

#[test]
fn encode_decode_round_trip_nodes() {
    let empty = SeleneGraph::new(GraphId::new(1));
    assert!(
        decode_nodes(&encode_nodes(&empty).unwrap())
            .unwrap()
            .is_empty()
    );

    let one = graph_with_node();
    let rows = decode_nodes(&encode_nodes(&one).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, NodeId::new(1));
    assert!(rows[0].1.alive);

    let shared = SharedGraph::builder(GraphId::new(2)).build().unwrap();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for index in 0..100 {
            mutator
                .create_node(
                    LabelSet::single(intern("core.bulk").unwrap()),
                    prop("core.index", Value::Int(index)),
                )
                .unwrap();
        }
    }
    txn.commit().unwrap();
    let rows = decode_nodes(&encode_nodes(shared.read().as_ref()).unwrap()).unwrap();
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[99].0, NodeId::new(100));
}

#[test]
fn encode_decode_round_trip_edges() {
    let graph = graph_with_edge();
    let rows = decode_edges(&encode_edges(&graph).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, EdgeId::new(1));
    assert_eq!(rows[0].1.source, NodeId::new(1));
    assert_eq!(rows[0].1.target, NodeId::new(2));
}

#[test]
fn bytecheck_rejects_truncated_node_section() {
    let graph = graph_with_node();
    let mut bytes = encode_nodes(&graph).unwrap();
    let new_len = bytes.len().saturating_sub(16);
    bytes.truncate(new_len);
    assert!(matches!(
        decode_nodes(&bytes),
        Err(ProviderError::InvalidPayload { reason }) if reason.contains("bytecheck")
    ));
}

#[test]
fn bytecheck_rejects_corrupted_section_header() {
    let graph = graph_with_node();
    let mut bytes = encode_nodes(&graph).unwrap();
    bytes[0] ^= 0x80;
    assert!(matches!(
        decode_nodes(&bytes),
        Err(ProviderError::InvalidPayload { reason }) if reason.contains("bytecheck")
    ));
}

#[test]
fn properties_blob_round_trips_full_value_set() {
    let shared = SharedGraph::builder(GraphId::new(3)).build().unwrap();
    let expected = full_value_property_map("core.values");
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(intern("core.values.node").unwrap()),
                expected.clone(),
            )
            .unwrap();
    }
    txn.commit().unwrap();

    let rows = decode_nodes(&encode_nodes(shared.read().as_ref()).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.properties, expected);
}

#[test]
fn encoded_nodes_are_sorted_by_node_id() {
    let graph = graph_with_node();
    let rows = decode_nodes(&encode_nodes(&graph).unwrap()).unwrap();
    let keys: Vec<_> = rows.into_iter().map(|(id, _)| id).collect();
    assert_eq!(keys, vec![NodeId::new(1)]);
}

#[test]
fn core_section_oversize_returns_inconsistent_error() {
    assert!(matches!(
        ensure_section_within_cap("CORE/NODE", selene_persist::MAX_SECTION_PAYLOAD_BYTES + 1),
        Err(ProviderError::Inconsistent { reason })
            if reason.contains("core section exceeds 1 GiB cap")
    ));
}

#[test]
fn live_mode_on_change_is_noop() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(Arc::clone(&snapshot));
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(intern("core.noop").unwrap()),
            properties: PropertyMap::new(),
        },
    )
    .unwrap();
    let rows = decode_nodes(
        &IndexProvider::write_section(provider.as_ref(), SubTag(CORE_NODE_SUB)).unwrap(),
    )
    .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn live_mode_write_section_serializes_current_snapshot() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(Arc::clone(&snapshot));
    snapshot.store(Arc::new(graph_with_node()));
    let rows = decode_nodes(
        &IndexProvider::write_section(provider.as_ref(), SubTag(CORE_NODE_SUB)).unwrap(),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn live_mode_read_section_returns_inconsistent_error() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(snapshot);
    assert!(matches!(
        IndexProvider::read_section(provider.as_ref(), SubTag(CORE_NODE_SUB), &[]),
        Err(ProviderError::Inconsistent { reason })
            if reason.contains("read_section called on live-mode")
    ));
}

#[test]
fn recovery_mode_read_section_populates_state() {
    let graph = graph_with_node();
    let bytes = encode_nodes(&graph).unwrap();
    let provider = CoreProvider::new_for_recovery();
    IndexProvider::read_section(provider.as_ref(), SubTag(CORE_NODE_SUB), &bytes).unwrap();
    let recovered = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert_eq!(recovered.node_count(), 1);
    assert!(recovered.is_node_alive(NodeId::new(1)));
}

#[test]
fn recovery_mode_on_change_applies_node_created() {
    let provider = CoreProvider::new_for_recovery();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(intern("core.created").unwrap()),
            properties: prop("core.created.prop", Value::Int(1)),
        },
    )
    .unwrap();
    let graph = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert_eq!(graph.node_count(), 1);
    assert!(graph.is_node_alive(NodeId::new(1)));
}

#[test]
fn recovery_mode_on_change_applies_each_change_variant() {
    let add_label = intern("core.added").unwrap();
    let base_label = intern("core.base").unwrap();
    let prop_key = intern("core.k").unwrap();
    let provider = CoreProvider::new_for_recovery();

    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(base_label),
            properties: PropertyMap::new(),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(2),
            labels: LabelSet::single(base_label),
            properties: PropertyMap::new(),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::EdgeCreated {
            id: EdgeId::new(1),
            label: intern("core.connects").unwrap(),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeUpdated {
            id: NodeId::new(1),
            labels_diff: LabelDiff::new([add_label], []).unwrap(),
            properties_diff: PropertyDiff::new([(prop_key, Value::Int(42))], []).unwrap(),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::EdgeUpdated {
            id: EdgeId::new(1),
            properties_diff: PropertyDiff::new([(prop_key, Value::Int(7))], []).unwrap(),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::SchemaChanged {
            graph: GraphId::new(1),
            change: SchemaChange::GraphDropped {
                id: GraphId::new(9),
            },
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::IndexExtensionEvent {
            provider: intern("core.extension").unwrap(),
            payload: Arc::from([1_u8, 2]),
        },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::EdgeDeleted { id: EdgeId::new(1) },
    )
    .unwrap();
    IndexProvider::on_change(
        provider.as_ref(),
        &Change::NodeDeleted { id: NodeId::new(1) },
    )
    .unwrap();

    let graph = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert!(!graph.is_node_alive(NodeId::new(1)));
    assert!(graph.is_node_alive(NodeId::new(2)));
    assert!(!graph.is_edge_alive(EdgeId::new(1)));
    assert_eq!(graph.node_store.len(), 2);
    assert_eq!(graph.edge_store.len(), 1);
}

#[test]
fn recovery_mode_write_section_returns_inconsistent_error() {
    let provider = CoreProvider::new_for_recovery();
    assert!(matches!(
        IndexProvider::write_section(provider.as_ref(), SubTag(CORE_NODE_SUB)),
        Err(ProviderError::Inconsistent { reason })
            if reason.contains("write_section called on recovery-mode")
    ));
}

#[test]
fn recovery_provider_read_section_round_trips_via_typed_path() {
    let graph = graph_with_node();
    let bytes = encode_nodes(&graph).unwrap();
    let provider = CoreProvider::new_for_recovery();
    RecoveryProvider::read_section(provider.as_ref(), CORE_NODE_SUB, &bytes).unwrap();
    let recovered = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert!(recovered.is_node_alive(NodeId::new(1)));
}

#[test]
fn recovery_provider_on_change_calls_typed_path() {
    let provider = CoreProvider::new_for_recovery();
    RecoveryProvider::on_change(
        provider.as_ref(),
        &Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(intern("core.raw").unwrap()),
            properties: PropertyMap::new(),
        },
    )
    .unwrap();
    let recovered = provider.finish_recovery(GraphId::new(1)).unwrap();
    assert!(recovered.is_node_alive(NodeId::new(1)));
}
