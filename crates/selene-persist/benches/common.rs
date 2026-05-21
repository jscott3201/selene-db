#![allow(missing_docs)]
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::Criterion;
use selene_core::{Change, HlcTimestamp, LabelSet, NodeId, Origin, PropertyMap, Value, intern};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, SectionCompression, SnapshotBuilder, SnapshotConfig, SyncPolicy,
    WalConfig, WalWriter, snapshot_path,
};
use selene_testing::BenchProfile;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "selene-bench-{name}-{}-{nanos}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).expect("temp dir is created");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

pub(crate) fn scales() -> &'static [usize] {
    BenchProfile::from_env().scales()
}

pub(crate) fn changes(count: usize) -> Vec<Change> {
    let label = intern("bench.persist.node").expect("fixture string fits");
    let key = intern("bench.persist.value").expect("fixture string fits");
    (0..count)
        .map(|idx| Change::NodeCreated {
            id: NodeId::new(idx as u64 + 1),
            labels: LabelSet::single(label),
            properties: PropertyMap::from_pairs([(key, Value::Int(idx as i64))])
                .expect("fixture properties fit"),
        })
        .collect()
}

pub(crate) fn write_wal(dir: &Path, entries: usize, batch_size: usize, snapshot_seq: u64) {
    let path = dir.join(DEFAULT_WAL_FILE_NAME);
    let mut writer = WalWriter::open(
        &path,
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1_000),
            snapshot_seq,
        },
    )
    .expect("wal opens");
    let payload = changes(batch_size);
    for idx in 0..entries {
        writer
            .append(
                HlcTimestamp::new(idx as u64 + 1, 0),
                Origin::Local,
                None,
                &payload,
            )
            .expect("wal append succeeds");
    }
    writer.flush().expect("wal flush succeeds");
}

pub(crate) fn write_snapshot(dir: &Path, sequence: u64, bytes: usize) -> PathBuf {
    let mut builder = SnapshotBuilder::new(SnapshotConfig {
        dir: dir.to_path_buf(),
        sequence,
        compression: SectionCompression::None,
        fsync: false,
    });
    builder
        .add_section(*b"CORE", *b"DATA", vec![0x5a; bytes])
        .expect("section add succeeds");
    builder.finalize().expect("snapshot write succeeds");
    snapshot_path(dir, sequence)
}
