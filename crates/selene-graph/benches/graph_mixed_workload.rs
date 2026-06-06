#![allow(missing_docs)]
//! Scalar graph read/write mixed workload benchmark.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{LabelDiff, NodeId, PropertyDiff, Value};
use selene_graph::SharedGraph;
use selene_persist::{DEFAULT_WAL_FILE_NAME, WalConfig};
use selene_testing::BenchFixture;

const READS_PER_CYCLE: usize = 60;
const WRITES_PER_CYCLE: usize = 40;
const OPS_PER_CYCLE: usize = READS_PER_CYCLE + WRITES_PER_CYCLE;

static WAL_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn bench_scalar_mixed_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_mixed_workload");
    for fixture in common::fixtures() {
        group.throughput(Throughput::Elements(OPS_PER_CYCLE as u64));
        group.bench_with_input(
            BenchmarkId::new("point_read_update_r60w40", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter_batched(
                    || ScalarMixedFixture::build(fixture),
                    |fixture| std::hint::black_box(fixture.run_cycle()),
                    BatchSize::LargeInput,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("point_read_update_r60w40_wal", fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter_batched(
                    || WalMixedFixture::build(fixture),
                    |fixture| std::hint::black_box(fixture.run_cycle()),
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

struct ScalarMixedFixture {
    shared: SharedGraph,
    score_key: selene_core::IStr,
    read_ids: Vec<NodeId>,
    update_ids: Vec<NodeId>,
}

impl ScalarMixedFixture {
    fn build(fixture: &BenchFixture) -> Self {
        Self::from_shared(SharedGraph::from_graph(fixture.graph().clone()), fixture)
    }

    fn from_shared(shared: SharedGraph, fixture: &BenchFixture) -> Self {
        let node_count = fixture.graph().node_count().max(1);
        Self {
            shared,
            score_key: fixture.score_key(),
            read_ids: (0..READS_PER_CYCLE)
                .map(|idx| NodeId::new((idx % node_count) as u64 + 1))
                .collect(),
            update_ids: (0..WRITES_PER_CYCLE)
                .map(|idx| NodeId::new((idx % node_count) as u64 + 1))
                .collect(),
        }
    }

    fn run_cycle(self) -> usize {
        let mut read_idx = 0;
        let mut write_idx = 0;
        let mut observed = 0;
        for slot in 0..OPS_PER_CYCLE {
            if slot % 5 < 3 {
                observed += self.read_once(read_idx);
                read_idx += 1;
            } else {
                self.write_once(write_idx);
                write_idx += 1;
            }
        }
        observed
    }

    fn read_once(&self, idx: usize) -> usize {
        self.shared
            .read()
            .node_properties(self.read_ids[idx])
            .is_some() as usize
    }

    fn write_once(&self, idx: usize) {
        let mut txn = self.shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let diff = PropertyDiff::new(
                [(
                    self.score_key.clone(),
                    Value::Int((idx + READS_PER_CYCLE) as i64),
                )],
                [],
            )
            .expect("bench property diff is valid");
            mutator
                .update_node(
                    self.update_ids[idx],
                    LabelDiff::new([], []).expect("bench label diff is valid"),
                    diff,
                )
                .expect("bench scalar update succeeds");
        }
        txn.commit().expect("bench scalar commit succeeds");
    }
}

struct WalMixedFixture {
    inner: Option<ScalarMixedFixture>,
    dir: PathBuf,
}

impl WalMixedFixture {
    fn build(fixture: &BenchFixture) -> Self {
        let dir = fresh_wal_dir();
        let shared = SharedGraph::from_graph_with_wal(
            fixture.graph().clone(),
            dir.join(DEFAULT_WAL_FILE_NAME),
            WalConfig::default(),
        )
        .expect("open WAL-backed mixed-workload graph");
        Self {
            inner: Some(ScalarMixedFixture::from_shared(shared, fixture)),
            dir,
        }
    }

    fn run_cycle(mut self) -> usize {
        self.inner
            .take()
            .expect("WAL mixed fixture owns inner graph")
            .run_cycle()
    }
}

impl Drop for WalMixedFixture {
    fn drop(&mut self) {
        let _ = self.inner.take();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fresh_wal_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after UNIX epoch")
        .as_nanos();
    let seq = WAL_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "selene-bench-mixed-wal-{}-{nanos}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create mixed-workload WAL dir");
    dir
}

criterion_group! {
    name = graph_mixed_workload;
    config = common::criterion_config();
    targets = bench_scalar_mixed_workload
}
criterion_main!(graph_mixed_workload);
