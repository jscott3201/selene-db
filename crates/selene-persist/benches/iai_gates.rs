#![allow(missing_docs)]
//! iai-callgrind gates for deterministic persistence hot paths.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::sync::{Arc, Mutex};

use iai_callgrind::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};
use selene_core::{Change, HlcTimestamp, Origin};
use selene_persist::{
    DEFAULT_WAL_FILE_NAME, ProviderRegistry, RecoveryProvider, RecoveryResult, SnapshotReader,
    SyncPolicy, WalConfig, WalReader, WalWriter, recover,
};

#[library_benchmark]
fn wal_append_single_1k() -> u64 {
    let dir = common::TempDir::new("iai-wal-single");
    let mut writer = WalWriter::open(
        &dir.path().join(DEFAULT_WAL_FILE_NAME),
        WalConfig {
            sync_policy: SyncPolicy::EveryN(1_000),
            snapshot_seq: 0,
        },
    )
    .expect("wal opens");
    let changes = common::changes(1);
    for idx in 0..1_000 {
        writer
            .append(HlcTimestamp::new(idx + 1, 0), Origin::Local, None, &changes)
            .expect("append succeeds");
    }
    writer.flush().expect("flush succeeds");
    std::hint::black_box(writer.last_sequence())
}

#[library_benchmark]
fn wal_replay_1k() -> usize {
    let dir = common::TempDir::new("iai-wal-replay");
    common::write_wal(dir.path(), 1_000, 1, 0);
    let reader = WalReader::open(&dir.path().join(DEFAULT_WAL_FILE_NAME)).expect("wal opens");
    std::hint::black_box(
        reader
            .iterate(|_| true)
            .expect("wal iterates")
            .map(|view| {
                view.expect("entry reads")
                    .body()
                    .expect("body decodes")
                    .len()
            })
            .sum(),
    )
}

#[library_benchmark]
fn snapshot_read_1k() -> usize {
    let dir = common::TempDir::new("iai-snapshot-read");
    let path = common::write_snapshot(dir.path(), 1, 64 * 1_000);
    let mut reader = SnapshotReader::open(&path).expect("snapshot opens");
    reader.verify_body_hash().expect("hash verifies");
    std::hint::black_box(
        reader
            .read_section(*b"CORE", *b"DATA")
            .expect("section reads")
            .len(),
    )
}

#[library_benchmark]
fn full_recovery_1k() -> u64 {
    let dir = common::TempDir::new("iai-full-recovery");
    common::write_snapshot(dir.path(), 1, 64 * 500);
    common::write_wal(dir.path(), 500, 1, 1);
    let mut registry = ProviderRegistry::new();
    registry
        .register(Arc::new(RecordingProvider::default()))
        .expect("provider registers");
    std::hint::black_box(
        recover(dir.path(), &registry)
            .expect("recovery succeeds")
            .wal_changes_applied,
    )
}

#[derive(Default)]
struct RecordingProvider {
    events: Mutex<usize>,
}

impl RecoveryProvider for RecordingProvider {
    fn provider_tag(&self) -> [u8; 4] {
        *b"CORE"
    }

    fn read_section(&self, _sub: [u8; 4], bytes: &[u8]) -> RecoveryResult<()> {
        *self.events.lock().expect("provider mutex is not poisoned") += bytes.len();
        Ok(())
    }

    fn on_change(&self, _change: &Change) -> RecoveryResult<()> {
        *self.events.lock().expect("provider mutex is not poisoned") += 1;
        Ok(())
    }
}

library_benchmark_group!(
    name = persist_gates;
    benchmarks = wal_append_single_1k, wal_replay_1k, snapshot_read_1k, full_recovery_1k
);

fn gate_config() -> LibraryBenchmarkConfig {
    let mut callgrind = Callgrind::default();
    callgrind.soft_limits([(EventKind::Ir, 5.0)]);
    callgrind.fail_fast(true);

    let mut config = LibraryBenchmarkConfig::default();
    config.tool(callgrind);
    config
}

main!(
    config = gate_config();
    library_benchmark_groups = persist_gates
);
