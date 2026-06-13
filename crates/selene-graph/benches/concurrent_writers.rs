#![allow(missing_docs)]
//! Criterion benches for serialized writer queueing under contention.
//!
//! Two axes (v1.2 BRIEF 2):
//! - **in-memory** (no WAL) — the original rows; pure committer queueing.
//! - **WAL-backed** (real on-disk WAL, tempdir per iteration) — the only setup
//!   where group commit can show a win, because the win comes from coalescing
//!   real `fsync` syscalls. Each is run for both [`CommitBatching::Off`] (one
//!   fsync per commit, the BRIEF-1 baseline) and [`CommitBatching::DEFAULT_ON`]
//!   (coalesce a contiguous run into one fsync). At 16/32-thread fan-in the
//!   batchON arm should show higher `Throughput::Elements` and lower p99/p999
//!   than batchOFF.
//!
//! On the `full` / `stress` profiles each WAL-backed arm also runs an **untimed**
//! percentile pass (outside Criterion's measured closure) that collects
//! per-commit latency `Instant` deltas and prints p50/p99/p999 to stderr — the
//! tail-latency read the mean-oriented Criterion sample cannot show. WAL `fsync`
//! teardown is kept out of the measured region via `BatchSize::LargeInput`
//! (Criterion runs setup + drop untimed).

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelDiff, NodeId, PropertyDiff, Value};
use selene_graph::{CommitBatching, SharedGraph};
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};
use selene_testing::{BenchFixture, BenchProfile};

const TOTAL_COMMITS: usize = 1_000;
const UPDATES_PER_COMMIT: usize = 10;
const THREADS: &[usize] = &[1, 2, 4, 8, 16, 32];

/// Monotonic suffix so each WAL tempdir is unique within a process run.
static WAL_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// A WAL-backed graph plus its tempdir. Dropping it joins the committer (which
/// flushes acked work) and removes the dir; both happen in Criterion's untimed
/// teardown under `BatchSize::LargeInput`.
struct WalGraph {
    shared: SharedGraph,
    dir: PathBuf,
}

impl Drop for WalGraph {
    fn drop(&mut self) {
        // The SharedGraph drop (joining the committer + releasing the WAL file
        // lock) runs first via field drop order, then we reclaim the dir.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fresh_wal_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = WAL_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "selene-bench-wal-{}-{nanos}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create bench WAL dir");
    dir
}

fn build_wal_graph(fixture: &BenchFixture, batching: CommitBatching) -> WalGraph {
    let dir = fresh_wal_dir();
    let shared = SharedGraph::builder(fixture.graph().meta.graph_id)
        .with_wal(dir.join(DEFAULT_WAL_FILE_NAME), WalConfig::default())
        .expect("open bench WAL")
        .with_commit_batching(batching)
        .build()
        .expect("build WAL-backed graph");
    // Seed the live graph with the fixture's nodes via one bulk commit so the
    // hot update loop has rows to touch (the in-memory arm clones a pre-seeded
    // fixture; the WAL arm must replay it through the funnel).
    seed_from_fixture(&shared, fixture);
    WalGraph { shared, dir }
}

/// Seed `shared` with one node per fixture node id (ids 1..=N) so the update
/// loop's `node_for` targets resolve. One commit, off the measured path.
fn seed_from_fixture(shared: &SharedGraph, fixture: &BenchFixture) {
    let count = fixture.graph().node_count();
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for _ in 0..count {
            mutator
                .create_node(
                    selene_core::LabelSet::new(),
                    selene_core::PropertyMap::new(),
                )
                .expect("seed node");
        }
    }
    txn.commit().expect("seed commit");
}

fn bench_concurrent_writers(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_writers");
    let fixture = BenchFixture::build(10_000);
    let profile = BenchProfile::from_env();
    let collect_percentiles = matches!(profile, BenchProfile::Full | BenchProfile::Stress);

    for &threads in THREADS {
        group.throughput(Throughput::Elements(TOTAL_COMMITS as u64));

        // ── In-memory (no WAL): original rows, pure committer queueing. ──
        group.bench_function(
            BenchmarkId::from_parameter(format!("threads{threads}")),
            |b| {
                b.iter_batched(
                    || SharedGraph::from_graph(fixture.graph().clone()),
                    |shared| {
                        run_writers(&shared, threads);
                        std::hint::black_box(shared.read().node_count())
                    },
                    BatchSize::LargeInput,
                );
            },
        );

        group.bench_function(
            BenchmarkId::from_parameter(format!("threads{threads}_with_readers8")),
            |b| {
                b.iter_batched(
                    || SharedGraph::from_graph(fixture.graph().clone()),
                    |shared| {
                        run_writers_with_readers(&shared, threads);
                        std::hint::black_box(shared.read().node_count())
                    },
                    BatchSize::LargeInput,
                );
            },
        );

        // ── WAL-backed: the only arm where group commit can win. ──
        for (label, batching) in [
            ("batchOFF", CommitBatching::Off),
            ("batchON", CommitBatching::DEFAULT_ON),
        ] {
            group.bench_function(
                BenchmarkId::from_parameter(format!("wal_threads{threads}_{label}")),
                |b| {
                    b.iter_batched(
                        || build_wal_graph(&fixture, batching),
                        |wal| {
                            run_writers(&wal.shared, threads);
                            std::hint::black_box(wal.shared.read().node_count())
                        },
                        BatchSize::LargeInput,
                    );
                },
            );

            if collect_percentiles {
                dump_percentiles(&fixture, threads, label, batching);
            }
        }
    }
    group.finish();
}

fn run_writers(shared: &SharedGraph, threads: usize) {
    std::thread::scope(|scope| {
        let handles = (0..threads)
            .map(|thread_idx| {
                scope.spawn(move || {
                    writer_loop(shared, thread_idx, TOTAL_COMMITS / threads);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("writer thread joins");
        }
    });
}

fn run_writers_with_readers(shared: &SharedGraph, threads: usize) {
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let reader_handles = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let mut reads = 0_usize;
                    while !done.load(Ordering::Relaxed) {
                        let snapshot = shared.read();
                        std::hint::black_box(snapshot.node_properties(NodeId::new(1)));
                        reads = reads.wrapping_add(1);
                    }
                    std::hint::black_box(reads);
                })
            })
            .collect::<Vec<_>>();

        let writer_handles = (0..threads)
            .map(|thread_idx| {
                scope.spawn(move || {
                    writer_loop(shared, thread_idx, TOTAL_COMMITS / threads);
                })
            })
            .collect::<Vec<_>>();

        for handle in writer_handles {
            handle.join().expect("writer thread joins");
        }
        done.store(true, Ordering::Relaxed);
        for handle in reader_handles {
            handle.join().expect("reader thread joins");
        }
    });
}

fn writer_loop(shared: &SharedGraph, thread_idx: usize, commits: usize) {
    let score = selene_core::db_string("score").expect("score key fits DB string cap");
    for commit_idx in 0..commits {
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            for update_idx in 0..UPDATES_PER_COMMIT {
                let node = node_for(thread_idx, update_idx);
                let value = ((commit_idx * UPDATES_PER_COMMIT) + update_idx) as i64;
                let diff = PropertyDiff::new([(score.clone(), Value::Int(value))], [])
                    .expect("property diff");
                mutator
                    .update_node(
                        node,
                        LabelDiff::new([], []).expect("label diff is valid"),
                        diff,
                    )
                    .expect("node update succeeds");
            }
        }
        txn.commit().expect("writer commit succeeds");
    }
}

fn node_for(thread_idx: usize, update_idx: usize) -> NodeId {
    NodeId::new(((thread_idx * UPDATES_PER_COMMIT + update_idx) % 10_000) as u64 + 1)
}

/// Untimed percentile pass over a real WAL-backed graph: every writer records
/// each commit's wall-clock latency; merged + sorted, p50/p99/p999 print to
/// stderr. Run only on `full`/`stress`; outside Criterion's measured closure so
/// the collection overhead never pollutes the throughput sample.
// Why: this is a bench-only diagnostic dump. The brief mandates a p50/p99/p999
// stderr line for the WAL fan-in arms (the tail latency Criterion's mean sample
// cannot surface); stderr is the intended channel, so the workspace
// `print_stderr` deny is locally relaxed for this one bench helper.
#[allow(clippy::print_stderr)]
fn dump_percentiles(fixture: &BenchFixture, threads: usize, label: &str, batching: CommitBatching) {
    let wal = build_wal_graph(fixture, batching);
    let per_thread = TOTAL_COMMITS / threads;
    let mut all: Vec<u64> = Vec::with_capacity(per_thread * threads);
    let collected = std::thread::scope(|scope| {
        let handles = (0..threads)
            .map(|thread_idx| {
                let shared = &wal.shared;
                scope.spawn(move || {
                    let score = selene_core::db_string("score").expect("score fits DB string cap");
                    let mut samples = Vec::with_capacity(per_thread);
                    for commit_idx in 0..per_thread {
                        let start = Instant::now();
                        let mut txn = shared.begin_write();
                        {
                            let mut mutator = txn.mutator();
                            for update_idx in 0..UPDATES_PER_COMMIT {
                                let node = node_for(thread_idx, update_idx);
                                let value = ((commit_idx * UPDATES_PER_COMMIT) + update_idx) as i64;
                                let diff =
                                    PropertyDiff::new([(score.clone(), Value::Int(value))], [])
                                        .expect("property diff");
                                mutator
                                    .update_node(
                                        node,
                                        LabelDiff::new([], []).expect("label diff valid"),
                                        diff,
                                    )
                                    .expect("update ok");
                            }
                        }
                        txn.commit().expect("commit ok");
                        samples.push(start.elapsed().as_nanos() as u64);
                    }
                    samples
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().expect("percentile writer joins"))
            .collect::<Vec<_>>()
    });
    for samples in collected {
        all.extend(samples);
    }
    all.sort_unstable();
    let pct = |p: f64| -> u64 {
        if all.is_empty() {
            return 0;
        }
        let idx = ((p * (all.len() as f64 - 1.0)).round() as usize).min(all.len() - 1);
        all[idx]
    };
    eprintln!(
        "[concurrent_writers percentiles] wal_threads{threads}_{label}: \
         n={} p50={}us p99={}us p999={}us",
        all.len(),
        pct(0.50) / 1_000,
        pct(0.99) / 1_000,
        pct(0.999) / 1_000,
    );
}

criterion_group! {
    name = concurrent_writers;
    config = common::criterion_config();
    targets = bench_concurrent_writers
}
criterion_main!(concurrent_writers);
