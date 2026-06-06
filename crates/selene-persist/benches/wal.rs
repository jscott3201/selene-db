#![allow(missing_docs)]
//! Criterion benches for WAL append and replay.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod wal_compression;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{HlcTimestamp, Origin};
use selene_persist::{
    COMPRESS_THRESHOLD, DEFAULT_WAL_FILE_NAME, FLAG_PAYLOAD_COMPRESSED, SyncPolicy, WalConfig,
    WalReader, WalWriter,
};
use selene_testing::BenchProfile;

fn bench_wal_append_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_append_single");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-single");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::EveryN(1_000),
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes(1))
                },
                |(_dir, mut writer, changes)| {
                    for idx in 0..scale {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    writer.flush().expect("flush succeeds");
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_wal_append_batch_1000(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_append_batch_1000");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-batch");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::EveryN(1_000),
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes(1_000))
                },
                |(_dir, mut writer, changes)| {
                    let entries = scale.div_ceil(1_000);
                    for idx in 0..entries {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    writer.flush().expect("flush succeeds");
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_wal_append_single_no_fsync(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_append_single_no_fsync");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-single-no-fsync");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::OnFlushOnly,
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes(1))
                },
                |(_dir, mut writer, changes)| {
                    for idx in 0..scale {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_wal_append_batch_1000_no_fsync(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_append_batch_1000_no_fsync");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-batch-no-fsync");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::OnFlushOnly,
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes(1_000))
                },
                |(_dir, mut writer, changes)| {
                    let entries = scale.div_ceil(1_000);
                    for idx in 0..entries {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Total changes written per sample for the body-size sweep — held constant so
/// the only thing varying across the x-axis is how they are *packed* into WAL
/// entries.
fn body_size_total() -> usize {
    match BenchProfile::from_env() {
        BenchProfile::Quick => 10_000,
        _ => 100_000,
    }
}

/// Changes-per-entry packings swept at a fixed total: many tiny entries vs few
/// large ones. The large packings exercise the big-body serialize+write path
/// (PERSIST-04 vectored write) that the entry-count sweeps never reach.
fn body_size_packings() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[100, 1_000],
        _ => &[100, 1_000, 10_000, 50_000],
    }
}

/// Fixed total work, swept entry body size, no fsync — isolates the per-byte
/// body serialize+write cost from the per-entry overhead the count sweeps
/// measure. OnFlushOnly so disk sync never enters the sample.
fn bench_wal_body_size_no_fsync(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_body_size_no_fsync");
    let total = body_size_total();
    for &per_entry in body_size_packings() {
        let entries = total.div_ceil(per_entry);
        group.throughput(Throughput::Elements((entries * per_entry) as u64));
        group.bench_function(BenchmarkId::from_parameter(per_entry), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-body-size");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::OnFlushOnly,
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes(per_entry))
                },
                |(_dir, mut writer, changes)| {
                    for idx in 0..entries {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn payload_shape_total() -> usize {
    match BenchProfile::from_env() {
        BenchProfile::Quick => 1_000,
        _ => 10_000,
    }
}

fn payload_shape_batch_size() -> usize {
    100
}

fn bench_wal_payload_shape_no_fsync(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_shape_no_fsync");
    let total = payload_shape_total();
    let batch_size = payload_shape_batch_size();
    let entries = total.div_ceil(batch_size);
    for &shape in common::payload_shapes() {
        group.throughput(Throughput::Elements(total as u64));
        group.bench_function(BenchmarkId::new(shape.name(), total), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-payload-shape");
                    let writer = WalWriter::open(
                        &dir.path().join(DEFAULT_WAL_FILE_NAME),
                        WalConfig {
                            sync_policy: SyncPolicy::OnFlushOnly,
                            snapshot_seq: 0,
                        },
                    )
                    .expect("wal opens");
                    (dir, writer, common::changes_with_payload(batch_size, shape))
                },
                |(_dir, mut writer, changes)| {
                    for idx in 0..entries {
                        writer
                            .append(
                                HlcTimestamp::new(idx as u64 + 1, 0),
                                Origin::Local,
                                None,
                                &changes,
                            )
                            .expect("append succeeds");
                    }
                    std::hint::black_box(writer.last_sequence());
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

#[derive(Clone, Copy)]
struct CompressionThreshold {
    name: &'static str,
    bytes: Option<usize>,
}

fn compression_thresholds() -> &'static [CompressionThreshold] {
    &[
        CompressionThreshold {
            name: "always",
            bytes: Some(0),
        },
        CompressionThreshold {
            name: "threshold128",
            bytes: Some(128),
        },
        CompressionThreshold {
            name: "default",
            bytes: Some(COMPRESS_THRESHOLD),
        },
        CompressionThreshold {
            name: "512",
            bytes: Some(512),
        },
        CompressionThreshold {
            name: "4096",
            bytes: Some(4_096),
        },
        CompressionThreshold {
            name: "never",
            bytes: None,
        },
    ]
}

fn compression_batch_sizes() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[1, 10, 100],
        _ => &[1, 10, 100, 1_000],
    }
}

struct CompressionFixture {
    raw: Vec<u8>,
    threshold: CompressionThreshold,
}

struct CompressionOutcome {
    len: usize,
    checksum_lo: u32,
    flags: u8,
}

fn encode_with_threshold(raw: &[u8], threshold: CompressionThreshold) -> CompressionOutcome {
    if threshold.bytes.is_some_and(|bytes| raw.len() >= bytes) {
        let bytes = zstd::stream::encode_all(raw, 1).expect("bench zstd compression succeeds");
        CompressionOutcome {
            len: bytes.len(),
            checksum_lo: checksum_lo(&bytes),
            flags: FLAG_PAYLOAD_COMPRESSED,
        }
    } else {
        CompressionOutcome {
            len: raw.len(),
            checksum_lo: checksum_lo(raw),
            flags: 0,
        }
    }
}

fn checksum_lo(bytes: &[u8]) -> u32 {
    (xxhash_rust::xxh3::xxh3_64(bytes) & 0xFFFF_FFFF) as u32
}

fn bench_wal_payload_compression_sweep(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_compression_sweep");
    for &shape in common::payload_shapes() {
        for &batch_size in compression_batch_sizes() {
            let changes = common::changes_with_payload(batch_size, shape);
            let raw = postcard::to_stdvec(&changes).expect("bench WAL payload serializes");
            group.throughput(Throughput::Bytes(raw.len() as u64));
            for &threshold in compression_thresholds() {
                let fixture = CompressionFixture {
                    raw: raw.clone(),
                    threshold,
                };
                group.bench_with_input(
                    BenchmarkId::new(shape.name(), format!("b{batch_size}_{}", threshold.name)),
                    &fixture,
                    |b, fixture| {
                        b.iter(|| {
                            let outcome = encode_with_threshold(&fixture.raw, fixture.threshold);
                            std::hint::black_box((outcome.len, outcome.checksum_lo, outcome.flags));
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

fn bench_wal_sync_policy_sweep(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_sync_sweep");
    for (name, sync_policy) in [
        ("every1", SyncPolicy::EveryN(1)),
        ("every10", SyncPolicy::EveryN(10)),
        ("every100", SyncPolicy::EveryN(100)),
        ("every1000", SyncPolicy::EveryN(1_000)),
        ("on_flush_only", SyncPolicy::OnFlushOnly),
    ] {
        for scale in sync_sweep_scales(sync_policy) {
            group.throughput(Throughput::Elements(scale as u64));
            group.bench_function(BenchmarkId::new(name, scale), |b| {
                b.iter_batched(
                    || {
                        let dir = common::TempDir::new("wal-sync-sweep");
                        let writer = WalWriter::open(
                            &dir.path().join(DEFAULT_WAL_FILE_NAME),
                            WalConfig {
                                sync_policy,
                                snapshot_seq: 0,
                            },
                        )
                        .expect("wal opens");
                        (dir, writer, common::changes(1))
                    },
                    |(_dir, mut writer, changes)| {
                        for idx in 0..scale {
                            writer
                                .append(
                                    HlcTimestamp::new(idx as u64 + 1, 0),
                                    Origin::Local,
                                    None,
                                    &changes,
                                )
                                .expect("append succeeds");
                        }
                        writer.flush().expect("flush succeeds");
                        std::hint::black_box(writer.last_sequence());
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}

fn sync_sweep_scales(sync_policy: SyncPolicy) -> Vec<usize> {
    let mut scales = match BenchProfile::from_env() {
        BenchProfile::Quick => vec![1_000],
        BenchProfile::Full | BenchProfile::Stress => vec![1_000, 10_000, 100_000],
        _ => vec![1_000],
    };
    // The fsync-frequent policies are bound by `fsync` syscall latency, not
    // selene-db code: at 100k entries `every1`/`every10`/`every100` cost tens of
    // seconds per iteration, which criterion balloons to ~20 min for the
    // `every10/100k` arm alone (`sample_size(30)`). Cap them at ≤10k so a full
    // sweep is not dominated by one durability cell; `every1000`/`on_flush_only`
    // (few/no fsyncs) keep the 100k point.
    if matches!(sync_policy, SyncPolicy::EveryN(n) if n <= 100) {
        scales.retain(|scale| *scale <= 10_000);
    }
    scales
}

fn bench_wal_payload_shape_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_shape_replay");
    let total = payload_shape_total();
    let batch_size = payload_shape_batch_size();
    let entries = total.div_ceil(batch_size);
    for &shape in common::payload_shapes() {
        group.throughput(Throughput::Elements(total as u64));
        group.bench_function(BenchmarkId::new(shape.name(), total), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-payload-shape-replay");
                    common::write_wal_with_payload(
                        dir.path(),
                        entries,
                        batch_size,
                        shape,
                        0,
                        SyncPolicy::OnFlushOnly,
                    );
                    dir
                },
                |dir| {
                    let reader = WalReader::open(&dir.path().join(DEFAULT_WAL_FILE_NAME))
                        .expect("wal reads");
                    let total = reader
                        .iterate(|_| true)
                        .expect("wal iterates")
                        .map(|view| {
                            view.expect("entry reads")
                                .body()
                                .expect("body decodes")
                                .len()
                        })
                        .sum::<usize>();
                    std::hint::black_box(total);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_wal_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_replay");
    for &scale in common::scales() {
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::from_parameter(scale), |b| {
            b.iter_batched(
                || {
                    let dir = common::TempDir::new("wal-replay");
                    common::write_wal(dir.path(), scale, 1, 0);
                    dir
                },
                |dir| {
                    let reader = WalReader::open(&dir.path().join(DEFAULT_WAL_FILE_NAME))
                        .expect("wal reads");
                    let total = reader
                        .iterate(|_| true)
                        .expect("wal iterates")
                        .map(|view| {
                            view.expect("entry reads")
                                .body()
                                .expect("body decodes")
                                .len()
                        })
                        .sum::<usize>();
                    std::hint::black_box(total);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = wal_group;
    config = common::criterion_config();
    targets = bench_wal_append_single, bench_wal_append_batch_1000,
        bench_wal_append_single_no_fsync, bench_wal_append_batch_1000_no_fsync,
        bench_wal_body_size_no_fsync, bench_wal_payload_shape_no_fsync,
        bench_wal_payload_compression_sweep, bench_wal_sync_policy_sweep,
        wal_compression::bench_wal_payload_compression_policy_no_fsync,
        wal_compression::bench_wal_payload_compression_policy_flush,
        wal_compression::bench_wal_payload_compression_policy_replay,
        bench_wal_payload_shape_replay, bench_wal_replay
}
criterion_main!(wal_group);
