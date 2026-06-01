#![allow(missing_docs)]
//! Criterion bench for high-degree hub deletion (GRAPH-05 gating bench).
//!
//! Deleting a node cascades over every incident edge; `adjacency` removal is
//! linear (`.position()` + `Vec::remove`) per edge, so deleting a degree-`D`
//! hub is O(D²). This bench builds a star graph and times deleting the center,
//! sweeping the **degree** axis (not node scale) so the curve visibly bends
//! from quadratic to linear once in-place adjacency delete lands. The uniform
//! ~degree-6 `BenchFixture` and the edgeless-node `write_txn_lifecycle`
//! `delete_only` bench cannot surface this.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::NodeId;
use selene_graph::SharedGraph;
use selene_testing::BenchProfile;
use selene_testing::bench_fixtures::star_graph;

/// Hub degrees swept. This is an independent axis from `BenchProfile` node
/// scales (a hub's cost is its degree); `quick` trims the slow high-degree arm
/// so spot-checks stay fast, while `full`/`stress` keep the curve that exposes
/// the quadratic.
fn degrees() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[100, 1_000],
        BenchProfile::Full => &[100, 1_000, 10_000],
        _ => &[100, 1_000, 10_000, 25_000],
    }
}

fn bench_hub_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_hub_delete");
    for &degree in degrees() {
        // Built once per degree, outside the timed routine; each iteration
        // clones the immutable snapshot (CoW) so teardown stays untimed.
        let star = star_graph(degree);
        group.throughput(Throughput::Elements(degree as u64));
        group.bench_function(BenchmarkId::from_parameter(degree), |b| {
            b.iter_batched(
                || SharedGraph::from_graph(star.clone()),
                |shared| {
                    let mut txn = shared.begin_write();
                    {
                        let mut mutator = txn.mutator();
                        mutator
                            .delete_node(NodeId::new(1))
                            .expect("hub center delete succeeds");
                    }
                    let changes = txn
                        .commit()
                        .expect("hub delete commit succeeds")
                        .changes
                        .len();
                    std::hint::black_box((shared, changes))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = graph_hub_delete;
    config = common::criterion_config();
    targets = bench_hub_delete
}
criterion_main!(graph_hub_delete);
