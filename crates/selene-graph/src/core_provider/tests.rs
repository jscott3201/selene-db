use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwap;
use selene_core::{
    Change, EdgeId, GraphId, HlcTimestamp, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap,
    PropertyValueType, Value, intern,
};
use selene_persist::{WalConfig, WalReader, WalWriter};

use super::sections::{
    SchemaEntry, SchemaEntryV1, SchemaKey, decode_edges, decode_graph_types, decode_meta,
    decode_nodes, decode_schemas, encode_edges, encode_graph_types, encode_meta, encode_nodes,
    ensure_section_within_cap,
};
use super::*;
use crate::graph::PropertyIndexEntry;
use crate::typed_index::TypedIndex;
use crate::{DurableProvider, GraphError, SeleneGraph, SharedGraph, TypedIndexKind};

#[path = "tests/composites.rs"]
mod composites;
#[path = "tests/cpix.rs"]
mod cpix;
#[path = "tests/gtyp.rs"]
mod gtyp;

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(intern(name).unwrap(), value)]).unwrap()
}

fn temp_wal_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-core-provider-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    dir.join(selene_persist::DEFAULT_WAL_FILE_NAME)
}

fn wal_entries(path: &Path) -> Vec<selene_persist::WalEntry> {
    let reader = WalReader::open(path).unwrap();
    reader
        .iterate(|_| true)
        .unwrap()
        .map(|entry| entry.unwrap().into_entry().unwrap())
        .collect()
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
            Value::Duration("PT1H2S".parse().unwrap()),
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

fn graph_type() -> crate::GraphTypeDef {
    crate::GraphTypeDef {
        name: intern("core.gtyp").unwrap(),
        node_types: vec![crate::NodeTypeDef {
            name: intern("core.gtyp.node").unwrap(),
            key_labels: LabelSet::single(intern("CoreTypedNode").unwrap()),
            properties: vec![crate::PropertyTypeDef {
                name: intern("core.gtyp.name").unwrap(),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,

                record_field_types: None,
            }],
            validation_mode: crate::ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    }
}

fn typed_graph() -> SeleneGraph {
    let shared = SharedGraph::builder(GraphId::new(8))
        .bound_to(graph_type())
        .unwrap()
        .build()
        .unwrap();
    shared.read().as_ref().clone()
}

#[test]
fn new_for_live_holds_snapshot_pointer() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(Arc::clone(&snapshot));
    let inner = provider.inner.lock();
    match &*inner {
        CoreInner::Live {
            snapshot: observed, ..
        } => assert!(Arc::ptr_eq(observed, &snapshot)),
        CoreInner::Recovery { .. } => panic!("expected live provider"),
    }
}

#[test]
fn new_for_recovery_starts_empty() {
    let provider = CoreProvider::new_for_recovery();
    let graph = provider.finish_recovery(GraphId::new(1), None).unwrap();
    assert_eq!(graph.node_count(), 0);
    assert_eq!(graph.edge_count(), 0);
}

#[test]
fn finish_recovery_on_live_mode_is_error() {
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live(snapshot);
    assert!(matches!(
        provider.finish_recovery(GraphId::new(1), None),
        Err(GraphError::Provider(ProviderError::Inconsistent { reason }))
            if reason.contains("finish_recovery called on live-mode")
    ));
}

#[test]
fn encode_decode_round_trip_meta() {
    let graph = graph_with_node();
    let bytes = encode_meta(&graph.meta, 9).unwrap();
    let payload = decode_meta(&bytes).unwrap();
    assert_eq!(payload.graph_id, graph.meta.graph_id);
    assert_eq!(payload.generation, graph.meta.generation);
    assert_eq!(payload.next_node_id, graph.meta.next_node_id);
    assert_eq!(payload.next_edge_id, graph.meta.next_edge_id);
    assert_eq!(payload.bound_type_index, None);
    assert_eq!(payload.sequence, 9);
}

#[test]
fn encode_decode_round_trip_empty_graph_types() {
    let graph = graph_with_node();
    let rows = decode_graph_types(&encode_graph_types(&graph).unwrap()).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn scma_decode_resorts_rows_by_receiver_handle() {
    let zebra = intern("core.scma.zebra").unwrap();
    let apple = intern("core.scma.apple").unwrap();
    let zebra_prop = intern("core.scma.zebra.prop").unwrap();
    let apple_prop = intern("core.scma.apple.prop").unwrap();
    let zebra_key = SchemaKey {
        label: zebra,
        property: zebra_prop,
    };
    let apple_key = SchemaKey {
        label: apple,
        property: apple_prop,
    };
    let rows = vec![
        (
            apple_key,
            SchemaEntryV1 {
                kind: TypedIndexKind::I64,
            },
        ),
        (
            zebra_key,
            SchemaEntryV1 {
                kind: TypedIndexKind::String,
            },
        ),
    ];
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rows)
        .unwrap()
        .into_vec();

    let decoded = decode_schemas(&bytes).unwrap();

    assert_eq!(
        decoded,
        vec![
            (
                zebra_key,
                SchemaEntry {
                    kind: TypedIndexKind::String,
                    name: None,
                }
            ),
            (
                apple_key,
                SchemaEntry {
                    kind: TypedIndexKind::I64,
                    name: None,
                }
            ),
        ]
    );
}

#[test]
fn scma_v2_round_trip_preserves_property_index_name() {
    let label = intern("core.scma.named.label").unwrap();
    let property = intern("core.scma.named.property").unwrap();
    let name = intern("core.scma.named.index").unwrap();
    let mut graph = SeleneGraph::new(GraphId::new(9991));
    graph.property_index.insert(
        (label, property),
        PropertyIndexEntry::new(TypedIndex::new(TypedIndexKind::String), Some(name)),
    );

    let decoded = decode_schemas(&encode_schemas(&graph).unwrap()).unwrap();

    assert_eq!(
        decoded,
        vec![(
            SchemaKey { label, property },
            SchemaEntry {
                kind: TypedIndexKind::String,
                name: Some(name),
            }
        )]
    );
}

#[test]
fn scma_decode_rejects_duplicate_keys_after_resort() {
    let label = intern("core.scma.dup.label").unwrap();
    let property = intern("core.scma.dup.property").unwrap();
    let key = SchemaKey { label, property };
    let rows = vec![
        (
            key,
            SchemaEntryV1 {
                kind: TypedIndexKind::I64,
            },
        ),
        (
            key,
            SchemaEntryV1 {
                kind: TypedIndexKind::String,
            },
        ),
    ];
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rows)
        .unwrap()
        .into_vec();

    let result = decode_schemas(&bytes);

    assert!(result.is_err());
}

#[test]
fn encode_decode_round_trip_bound_graph_type() {
    let graph = typed_graph();
    let rows = decode_graph_types(&encode_graph_types(&graph).unwrap()).unwrap();
    assert_eq!(rows, vec![(0, graph_type())]);

    let payload = decode_meta(&encode_meta(&graph.meta, 1).unwrap()).unwrap();
    assert_eq!(payload.bound_type_index, Some(0));
}

#[test]
fn recovery_rejects_meta_referencing_missing_gtyp() {
    let graph = typed_graph();
    let bytes = encode_meta(&graph.meta, 1).unwrap();
    let provider = CoreProvider::new_for_recovery();
    IndexProvider::read_section(provider.as_ref(), SubTag(CORE_META_SUB), &bytes).unwrap();
    assert!(matches!(
        provider.finish_recovery(GraphId::new(8), None),
        Err(GraphError::Provider(ProviderError::Inconsistent { reason }))
            if reason.contains("missing CORE/GTYP index 0")
    ));
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
fn encoded_nodes_carry_explicit_row_to_id() {
    // BRIEF-Item-4a STEP 9: the CORE/NODE section persists the explicit external
    // id from the `row_to_id` column (no longer synthesized as row+1, and no
    // longer contractually sorted-by-id). For this identity graph the single
    // committed node round-trips its real id at its row position.
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
fn core_provider_writes_one_wal_entry_per_commit() {
    let path = temp_wal_path("one-entry");
    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live_with_wal(snapshot, Some(DurableState::new(writer)));
    let changes = vec![
        Change::NodeCreated {
            id: NodeId::new(1),
            labels: LabelSet::single(intern("core.wal.a").unwrap()),
            properties: PropertyMap::new(),
        },
        Change::NodeCreated {
            id: NodeId::new(2),
            labels: LabelSet::single(intern("core.wal.b").unwrap()),
            properties: PropertyMap::new(),
        },
        Change::EdgeCreated {
            id: EdgeId::new(1),
            label: intern("core.wal.edge").unwrap(),
            source: NodeId::new(1),
            target: NodeId::new(2),
            properties: PropertyMap::new(),
        },
    ];

    let timestamp = DurableProvider::next_timestamp(provider.as_ref());
    let seq = DurableProvider::write_commit(provider.as_ref(), None, &changes, timestamp).unwrap();
    assert_eq!(seq, 1);
    DurableProvider::flush(provider.as_ref()).unwrap();
    drop(provider);

    let entries = wal_entries(&path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].changes.len(), 3);
    assert_eq!(entries[0].header.sequence, 1);
    assert_eq!(entries[0].header.hlc(), HlcTimestamp::new(1, 0));
}

#[test]
fn core_provider_threads_principal_through_wal() {
    let path = temp_wal_path("principal");
    let writer = WalWriter::open(&path, WalConfig::default()).unwrap();
    let snapshot = Arc::new(ArcSwap::from_pointee(SeleneGraph::new(GraphId::new(1))));
    let provider = CoreProvider::new_for_live_with_wal(snapshot, Some(DurableState::new(writer)));
    let changes = vec![Change::NodeCreated {
        id: NodeId::new(1),
        labels: LabelSet::single(intern("core.wal.principal").unwrap()),
        properties: PropertyMap::new(),
    }];

    let timestamp = DurableProvider::next_timestamp(provider.as_ref());
    DurableProvider::write_commit(provider.as_ref(), Some(b"alice"), &changes, timestamp).unwrap();
    DurableProvider::flush(provider.as_ref()).unwrap();
    drop(provider);

    let entries = wal_entries(&path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].header.principal.as_deref(), Some(&b"alice"[..]));
}

#[test]
fn recovery_mode_read_section_populates_state() {
    let graph = graph_with_node();
    let bytes = encode_nodes(&graph).unwrap();
    let provider = CoreProvider::new_for_recovery();
    IndexProvider::read_section(provider.as_ref(), SubTag(CORE_NODE_SUB), &bytes).unwrap();
    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
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
    let graph = provider.finish_recovery(GraphId::new(1), None).unwrap();
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

    let graph = provider.finish_recovery(GraphId::new(1), None).unwrap();
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
    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
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
    let recovered = provider.finish_recovery(GraphId::new(1), None).unwrap();
    assert!(recovered.is_node_alive(NodeId::new(1)));
}
