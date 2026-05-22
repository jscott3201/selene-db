#![allow(missing_docs)]
//! Criterion benches for snapshot read/write and recovery.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::sync::{Arc, Mutex};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::Change;
use selene_persist::{ProviderRegistry, RecoveryProvider, RecoveryResult, SnapshotReader, recover};

fn bench_snapshot_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_snapshot_write");
    for &scale in common::scales() {
        let bytes = section_bytes(scale);
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || common::TempDir::new("snapshot-write"),
                |dir| {
                    let path = common::write_snapshot(dir.path(), 1, bytes);
                    std::hint::black_box(path);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_snapshot_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_snapshot_read");
    for &scale in common::scales() {
        let bytes = section_bytes(scale);
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("snapshot-read");
                    let path = common::write_snapshot(dir.path(), 1, bytes);
                    (dir, path)
                },
                |(_dir, path)| {
                    let mut reader = SnapshotReader::open(&path).expect("snapshot opens");
                    reader.verify_body_hash().expect("hash verifies");
                    let sections = reader.sections().to_vec();
                    let bytes = sections
                        .iter()
                        .map(|entry| {
                            reader
                                .read_section(entry.provider, entry.sub)
                                .expect("section reads")
                                .len()
                        })
                        .sum::<usize>();
                    std::hint::black_box(bytes);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_full_recovery(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_full_recovery");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("full-recovery");
                    common::write_snapshot(dir.path(), 1, section_bytes(scale / 2));
                    common::write_wal(dir.path(), scale.saturating_sub(scale / 2), 1, 1);
                    dir
                },
                |dir| {
                    let provider = RecordingProvider::default();
                    let mut registry = ProviderRegistry::new();
                    registry
                        .register(Arc::new(provider.clone()))
                        .expect("provider registers");
                    let outcome = recover(dir.path(), &registry).expect("recovery succeeds");
                    std::hint::black_box((
                        outcome.applied_snapshot_seq,
                        outcome.wal_changes_applied,
                        provider.event_count(),
                    ));
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn section_bytes(scale: usize) -> usize {
    scale.saturating_mul(64).max(64)
}

#[derive(Clone, Default)]
struct RecordingProvider {
    events: Arc<Mutex<usize>>,
}

impl RecordingProvider {
    fn event_count(&self) -> usize {
        *self.events.lock().expect("provider mutex is not poisoned")
    }
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

criterion_group! {
    name = snapshot_group;
    config = common::criterion_config();
    targets = bench_snapshot_write, bench_snapshot_read, bench_full_recovery
}
criterion_main!(snapshot_group);
