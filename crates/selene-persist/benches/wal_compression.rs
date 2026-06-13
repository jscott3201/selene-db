use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use selene_core::{HlcTimestamp, Origin};
use selene_persist::{
    COMPRESS_THRESHOLD, DEFAULT_WAL_FILE_NAME, SyncPolicy, WalCompression, WalConfig, WalReader,
    WalWriter,
};
use selene_testing::BenchProfile;

use crate::common;

#[derive(Clone, Copy)]
struct CompressionPolicy {
    name: &'static str,
    compression: WalCompression,
}

const COMPRESSION_POLICIES: [CompressionPolicy; 3] = [
    CompressionPolicy {
        name: "threshold128",
        compression: WalCompression::zstd(128),
    },
    CompressionPolicy {
        name: "default4096",
        compression: WalCompression::zstd(COMPRESS_THRESHOLD),
    },
    CompressionPolicy {
        name: "disabled",
        compression: WalCompression::disabled(),
    },
];

fn compression_policies() -> &'static [CompressionPolicy] {
    &COMPRESSION_POLICIES
}

fn compression_policy_total() -> usize {
    match BenchProfile::from_env() {
        BenchProfile::Quick => 1_000,
        _ => 10_000,
    }
}

fn compression_policy_batch_sizes() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[1, 10, 100],
        _ => &[1, 10, 100, 1_000],
    }
}

fn compression_policy_flush_batch_sizes() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[1, 100],
        _ => &[1, 10, 100, 1_000],
    }
}

pub(crate) fn bench_wal_payload_compression_policy_no_fsync(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_compression_policy_no_fsync");
    let total = compression_policy_total();
    for &shape in common::payload_shapes() {
        for &batch_size in compression_policy_batch_sizes() {
            let entries = total.div_ceil(batch_size);
            group.throughput(Throughput::Elements(total as u64));
            for &policy in compression_policies() {
                group.bench_function(
                    BenchmarkId::new(shape.name(), format!("b{batch_size}_{}", policy.name)),
                    |b| {
                        b.iter_batched(
                            || {
                                let dir = common::TempDir::new("wal-compression-policy");
                                let writer = WalWriter::open_with_compression(
                                    &dir.path().join(DEFAULT_WAL_FILE_NAME),
                                    WalConfig {
                                        sync_policy: SyncPolicy::OnFlushOnly,
                                        snapshot_seq: 0,
                                    },
                                    policy.compression,
                                )
                                .expect("wal opens");
                                let payload = common::changes_with_payload(batch_size, shape);
                                (dir, writer, payload)
                            },
                            |(_dir, mut writer, payload)| {
                                append_entries(&mut writer, entries, &payload);
                                std::hint::black_box(writer.last_sequence());
                            },
                            BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

pub(crate) fn bench_wal_payload_compression_policy_flush(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_compression_policy_flush");
    let total = compression_policy_total();
    for &shape in common::payload_shapes() {
        for &batch_size in compression_policy_flush_batch_sizes() {
            let entries = total.div_ceil(batch_size);
            for &policy in compression_policies() {
                let file_len = common::wal_file_len_with_payload_compression(
                    entries,
                    batch_size,
                    shape,
                    policy.compression,
                );
                group.throughput(Throughput::Bytes(file_len));
                group.bench_function(
                    BenchmarkId::new(
                        shape.name(),
                        format!("b{batch_size}_{}_{}b", policy.name, file_len),
                    ),
                    |b| {
                        b.iter_batched(
                            || {
                                let dir = common::TempDir::new("wal-compression-policy-flush");
                                let writer = WalWriter::open_with_compression(
                                    &dir.path().join(DEFAULT_WAL_FILE_NAME),
                                    WalConfig {
                                        sync_policy: SyncPolicy::OnFlushOnly,
                                        snapshot_seq: 0,
                                    },
                                    policy.compression,
                                )
                                .expect("wal opens");
                                let payload = common::changes_with_payload(batch_size, shape);
                                (dir, writer, payload)
                            },
                            |(_dir, mut writer, payload)| {
                                append_entries(&mut writer, entries, &payload);
                                writer.flush().expect("flush succeeds");
                                std::hint::black_box(writer.last_sequence());
                            },
                            BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

pub(crate) fn bench_wal_payload_compression_policy_replay(c: &mut Criterion) {
    let mut group = c.benchmark_group("persist_wal_payload_compression_policy_replay");
    let total = compression_policy_total();
    for &shape in common::payload_shapes() {
        for &batch_size in compression_policy_batch_sizes() {
            let entries = total.div_ceil(batch_size);
            group.throughput(Throughput::Elements(total as u64));
            for &policy in compression_policies() {
                group.bench_function(
                    BenchmarkId::new(shape.name(), format!("b{batch_size}_{}", policy.name)),
                    |b| {
                        b.iter_batched(
                            || {
                                let dir = common::TempDir::new("wal-compression-policy-replay");
                                common::write_wal_with_payload_compression(
                                    dir.path(),
                                    entries,
                                    batch_size,
                                    shape,
                                    0,
                                    SyncPolicy::OnFlushOnly,
                                    policy.compression,
                                );
                                let reader =
                                    WalReader::open(&dir.path().join(DEFAULT_WAL_FILE_NAME))
                                        .expect("reader opens");
                                (dir, reader)
                            },
                            |(_dir, reader)| {
                                let mut decoded = 0_usize;
                                for entry in reader.iterate(|_| true).expect("iterate") {
                                    let entry = entry.expect("entry reads");
                                    decoded += entry.body().expect("body decodes").len();
                                }
                                std::hint::black_box(decoded);
                            },
                            BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

fn append_entries(writer: &mut WalWriter, entries: usize, payload: &[selene_core::Change]) {
    for idx in 0..entries {
        writer
            .append(
                HlcTimestamp::new(idx as u64 + 1, 0),
                Origin::Local,
                None,
                payload,
            )
            .expect("append succeeds");
    }
}
