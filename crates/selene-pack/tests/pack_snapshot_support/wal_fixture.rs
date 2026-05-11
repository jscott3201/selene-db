//! WAL fixture writer for pack history snapshots.

use std::path::Path;

use jiff::Timestamp;
use selene_core::{
    Change, GraphId, HlcTimestamp, IStr, LabelSet, NodeId, Origin, PackLifecycleEvent, PropertyMap,
    SchemaChange, intern,
};
use selene_persist::{WalConfig, WalWriter};
use selene_testing::{PackHistoryWalEntry, PackLifecycleEventPayload};
use tempfile::NamedTempFile;

pub const HISTORY_GRAPH_ID: GraphId = GraphId::new(49);

pub struct TempWal {
    file: NamedTempFile,
}

impl TempWal {
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

pub fn write_history_wal(entries: &[PackHistoryWalEntry]) -> TempWal {
    let file = NamedTempFile::new().expect("test WAL tempfile opens");
    let mut writer = WalWriter::open(file.path(), WalConfig::default()).expect("test WAL opens");
    for (index, entry) in entries.iter().enumerate() {
        let change = entry_to_change(entry);
        writer
            .append(
                HlcTimestamp::new(index as u64 + 1, 0),
                Origin::Local,
                None,
                &[change],
            )
            .expect("test WAL append succeeds");
    }
    writer.flush().expect("test WAL flush succeeds");
    drop(writer);
    TempWal { file }
}

fn entry_to_change(entry: &PackHistoryWalEntry) -> Change {
    match entry {
        PackHistoryWalEntry::Lifecycle(event) => Change::SchemaChanged {
            graph: HISTORY_GRAPH_ID,
            change: SchemaChange::ProcedurePackLifecycle {
                event: lifecycle_event(event),
            },
        },
        PackHistoryWalEntry::LegacyProcedurePackActivated { pack_name, version } => {
            Change::SchemaChanged {
                graph: HISTORY_GRAPH_ID,
                change: SchemaChange::ProcedurePackActivated {
                    pack_name: istr(pack_name),
                    version: istr(version),
                },
            }
        }
        PackHistoryWalEntry::LegacyProcedurePackDeprecated {
            pack_name,
            version,
            reason,
        } => Change::SchemaChanged {
            graph: HISTORY_GRAPH_ID,
            change: SchemaChange::ProcedurePackDeprecated {
                pack_name: istr(pack_name),
                version: istr(version),
                reason: istr(reason),
            },
        },
        PackHistoryWalEntry::LegacyProcedurePackDisabled {
            pack_name,
            version,
            reason,
        } => Change::SchemaChanged {
            graph: HISTORY_GRAPH_ID,
            change: SchemaChange::ProcedurePackDisabled {
                pack_name: istr(pack_name),
                version: istr(version),
                reason: istr(reason),
            },
        },
        PackHistoryWalEntry::NodeCreated { id, label } => Change::NodeCreated {
            id: NodeId::new(*id),
            labels: LabelSet::single(istr(label)),
            properties: PropertyMap::new(),
        },
        _ => unreachable!("unknown pack history WAL entry"),
    }
}

fn lifecycle_event(event: &PackLifecycleEventPayload) -> PackLifecycleEvent {
    match event {
        PackLifecycleEventPayload::ValidationFailed {
            pack_name,
            principal,
            error,
            at_seconds,
        } => PackLifecycleEvent::ValidationFailed {
            pack_name: pack_name.map(istr),
            principal: istr(principal),
            error: (*error).to_owned(),
            at: timestamp(*at_seconds),
        },
        PackLifecycleEventPayload::Staged {
            pack_name,
            content_hash,
            principal,
            at_seconds,
        } => PackLifecycleEvent::Staged {
            pack_name: istr(pack_name),
            content_hash: *content_hash,
            principal: istr(principal),
            at: timestamp(*at_seconds),
        },
        PackLifecycleEventPayload::Activated {
            pack_name,
            content_hash,
            principal,
            at_seconds,
        } => PackLifecycleEvent::Activated {
            pack_name: istr(pack_name),
            content_hash: *content_hash,
            principal: istr(principal),
            at: timestamp(*at_seconds),
        },
        PackLifecycleEventPayload::Deprecated {
            pack_name,
            reason,
            principal,
            at_seconds,
        } => PackLifecycleEvent::Deprecated {
            pack_name: istr(pack_name),
            reason: istr(reason),
            principal: istr(principal),
            at: timestamp(*at_seconds),
        },
        PackLifecycleEventPayload::Disabled {
            pack_name,
            principal,
            at_seconds,
        } => PackLifecycleEvent::Disabled {
            pack_name: istr(pack_name),
            principal: istr(principal),
            at: timestamp(*at_seconds),
        },
        _ => unreachable!("unknown pack lifecycle payload"),
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("test timestamp in range")
}
