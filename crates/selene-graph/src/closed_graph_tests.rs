use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{
    Change, EdgeId, GraphId, HlcTimestamp, LabelDiff, LabelSet, NodeId, Origin, PropertyDiff,
    PropertyMap, PropertyValueType, Value,
};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotConfig, SyncPolicy, WalConfig, WalWriter,
};

use crate::{
    EdgeEndpointDef, EntityId, GraphError, GraphTypeDef, NodeTypeDef, PropertyDefaultValue,
    PropertyElementType, PropertyTypeDef, SharedGraph, TypeViolation, ValidationMode,
};

#[path = "closed_graph_tests/immutable.rs"]
mod immutable;

#[path = "closed_graph_tests/one_of.rs"]
mod one_of;

#[path = "closed_graph_tests/truncate.rs"]
mod truncate;

#[path = "closed_graph_tests/unique.rs"]
mod unique;

#[path = "closed_graph_tests/basic.rs"]
mod basic;

#[path = "closed_graph_tests/endpoints.rs"]
mod endpoints;

#[path = "closed_graph_tests/recovery.rs"]
mod recovery;

fn db_string(name: &str) -> selene_core::DbString {
    selene_core::db_string(name).unwrap()
}

fn prop(name: &str, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(db_string(name), value)]).unwrap()
}

fn person_graph_type() -> GraphTypeDef {
    GraphTypeDef {
        name: db_string("closed.person.graph"),
        node_types: vec![NodeTypeDef {
            name: db_string("closed.person"),
            key_labels: LabelSet::single(db_string("Person")),
            properties: vec![PropertyTypeDef {
                name: db_string("name"),
                value_type: PropertyValueType::String,
                list_element_type: None,
                required: true,
                default: None,
                immutable: false,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: vec![crate::EdgeTypeDef {
            name: db_string("closed.knows"),
            label: db_string("KNOWS"),
            source_node_type: EdgeEndpointDef::NodeType(0),
            target_node_type: EdgeEndpointDef::NodeType(0),
            properties: vec![PropertyTypeDef {
                name: db_string("since"),
                value_type: PropertyValueType::Int,
                list_element_type: None,
                required: false,
                default: None,
                immutable: false,
                unique: false,
                decimal_type: None,
                character_string_type: None,
                byte_string_type: None,
                record_field_types: None,
            }],
            validation_mode: ValidationMode::Strict,
        }],
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "selene-closed-graph-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    dir
}

fn write_snapshot(dir: &Path, shared: &SharedGraph, sequence: u64) {
    shared
        .write_snapshot(SnapshotConfig {
            dir: dir.to_path_buf(),
            sequence,
            compression: SectionCompression::None,
            fsync: false,
        })
        .unwrap();
}

fn append_wal(dir: &Path, snapshot_seq: u64, changes: &[Change]) {
    let mut writer = WalWriter::open(
        &dir.join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1),
            snapshot_seq,
        },
    )
    .unwrap();
    writer
        .append(HlcTimestamp::zero(), Origin::Local, None, changes)
        .unwrap();
    writer.flush().unwrap();
}
