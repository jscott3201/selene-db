//! Public writer/reader round-trip coverage for the WAL v2 entry format.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use selene_core::{Change, HlcTimestamp, NodeId, Origin, intern};
use selene_persist::{SyncPolicy, WalConfig, WalWriter};

fn temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "selene-persist-{name}-{}-{nanos}.wal",
        std::process::id()
    ))
}

fn principal(index: usize) -> Option<Arc<[u8]>> {
    match index % 4 {
        0 => None,
        1 => Some(Arc::from([])),
        2 => Some(Arc::from(b"alice".as_slice())),
        _ => Some(Arc::from(vec![index as u8; 4096])),
    }
}

fn origin(index: usize) -> Origin {
    if index.is_multiple_of(2) {
        Origin::Local
    } else {
        Origin::Replicated {
            source_node_id: NodeId::new(index as u64 + 1),
            source_seq: index as u64 * 10,
        }
    }
}

fn changes(index: usize) -> Vec<Change> {
    vec![Change::IndexExtensionEvent {
        provider: intern("wal.v2.roundtrip").unwrap(),
        payload: Arc::from(format!("entry-{index}").into_bytes()),
    }]
}

#[test]
fn wal_v2_round_trips_mixed_origins_and_principals() {
    let path = temp_path("v2-round-trip");
    let expected: Vec<_> = (0..100)
        .map(|index| (origin(index), principal(index), changes(index)))
        .collect();
    {
        let mut writer = WalWriter::open(
            &path,
            WalConfig {
                sync_policy: SyncPolicy::OnFlushOnly,
                snapshot_seq: 0,
            },
        )
        .unwrap();
        for (index, (origin, principal, changes)) in expected.iter().enumerate() {
            let sequence = writer
                .append(
                    HlcTimestamp::new(index as u64, index as u32),
                    *origin,
                    principal.clone(),
                    changes,
                )
                .unwrap();
            assert_eq!(sequence, index as u64 + 1);
        }
    }

    let reader = selene_persist::WalReader::open(&path).unwrap();
    let observed: Vec<_> = reader
        .iterate(|_| true)
        .unwrap()
        .map(|result| result.unwrap())
        .collect();
    assert_eq!(observed.len(), expected.len());
    for (index, (view, (origin, principal, changes))) in
        observed.into_iter().zip(expected).enumerate()
    {
        assert_eq!(view.header.sequence, index as u64 + 1);
        assert_eq!(view.header.origin, origin);
        assert_eq!(view.header.principal, principal);
        assert_eq!(view.body().unwrap(), changes);
    }

    let _ = fs::remove_file(path);
}
