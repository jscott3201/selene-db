#![allow(missing_docs)]
//! Criterion benches for graph-algorithm baselines.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_algorithms::{
    ApspConfig, BetweennessConfig, GraphProjection, PageRankConfig, Parallelism,
    TriangleCountConfig, apsp, betweenness, label_propagation, louvain, pagerank, scc, scc_count,
    topological_sort, triangle_count, wcc, wcc_count,
};
use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap};
use selene_graph::SharedGraph;
use selene_testing::{BenchFixture, BenchProfile};

use common::{build_projection, criterion_config, scale_label};

/// Node scales for the scale-swept algorithm groups, selected by
/// `SELENE_BENCH_PROFILE` (quick=1k, full=10k/50k/100k, stress=+250k) so a
/// `--profile quick` spot-check no longer runs the 100k betweenness baseline.
fn profile_scales() -> &'static [usize] {
    BenchProfile::from_env().scales()
}

// All-pairs SSSP iterates every source; N=1000 is roughly 10^6 output tuples.
// Bump only with measured wall-clock evidence per BRIEF-87 section B.2.
const APSP_SCALES: &[usize] = &[200, 500, 1_000];
const BENCH_BETWEENNESS_SAMPLE_SIZE: usize = 256;
const BENCH_LABEL_PROPAGATION_MAX_ITER: usize = 50;
const BENCH_LOUVAIN_MAX_ITER: usize = 50;
const PARALLELISM_BENCH_MODES: &[(&str, Parallelism)] = &[
    ("sequential", Parallelism::Sequential),
    ("auto", Parallelism::Auto),
];

fn bench_pagerank(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/pagerank");
    for &scale in profile_scales() {
        let state = BenchState::from_bench_fixture(scale);
        for &(mode, parallelism) in PARALLELISM_BENCH_MODES {
            let config = PageRankConfig {
                damping: 0.85,
                max_iter: 100,
                tolerance: 1e-6,
                parallelism,
                personalization: None,
            };
            group.bench_function(BenchmarkId::new(mode, scale_label(scale)), |b| {
                b.iter(|| black_box(pagerank(&state.projection, config.clone())));
            });
        }
    }
    group.finish();
}

fn bench_betweenness(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/betweenness");
    for &scale in profile_scales() {
        let state = BenchState::from_bench_fixture(scale);
        // Sample large betweenness fixtures so the 10k baseline stays bounded.
        let sample_size = betweenness_sample_size(scale);
        for &(mode, parallelism) in PARALLELISM_BENCH_MODES {
            let config = BetweennessConfig {
                sample_size,
                parallelism,
            };
            group.bench_function(BenchmarkId::new(mode, scale_label(scale)), |b| {
                b.iter(|| black_box(betweenness(&state.projection, config)));
            });
        }
    }
    group.finish();
}

fn bench_triangle_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/triangle_count");
    for &scale in profile_scales() {
        // Local planted communities keep triangle_count from measuring a mostly-empty result.
        let state = BenchState::from_planted_community(scale, 82_200 + scale as u64);
        for &(mode, parallelism) in PARALLELISM_BENCH_MODES {
            let config = TriangleCountConfig { parallelism };
            group.bench_function(BenchmarkId::new(mode, scale_label(scale)), |b| {
                b.iter(|| black_box(triangle_count(&state.projection, config)));
            });
        }
    }
    group.finish();
}

fn bench_apsp(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/apsp");
    for &scale in APSP_SCALES {
        let state = BenchState::from_bench_fixture(scale);
        for &(mode, parallelism) in PARALLELISM_BENCH_MODES {
            let config = ApspConfig {
                max_nodes: scale,
                parallelism,
            };
            group.bench_function(BenchmarkId::new(mode, scale_label(scale)), |b| {
                b.iter(|| black_box(apsp(&state.projection, config).expect("apsp bench succeeds")));
            });
        }
    }
    group.finish();
}

fn bench_topological_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/topological_sort");
    for &scale in profile_scales() {
        let state = BenchState::from_dag(scale, 82_240 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| {
                black_box(topological_sort(&state.projection).expect("bench graph is a DAG"))
            });
        });
    }
    group.finish();
}

fn bench_wcc(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/wcc");
    for &scale in profile_scales() {
        let state = BenchState::from_planted_community(scale, 82_245 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| black_box(wcc(&state.projection)));
        });
    }
    group.finish();
}

fn bench_wcc_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/wcc_count");
    for &scale in profile_scales() {
        let state = BenchState::from_planted_community(scale, 82_246 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| black_box(wcc_count(&state.projection)));
        });
    }
    group.finish();
}

fn bench_scc(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/scc");
    for &scale in profile_scales() {
        let state = BenchState::from_planted_community(scale, 82_247 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| black_box(scc(&state.projection)));
        });
    }
    group.finish();
}

fn bench_scc_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/scc_count");
    for &scale in profile_scales() {
        let state = BenchState::from_planted_community(scale, 82_248 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| black_box(scc_count(&state.projection)));
        });
    }
    group.finish();
}

fn bench_label_propagation(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/label_propagation");
    for &scale in profile_scales() {
        // Local planted communities give label propagation a stable community structure.
        let state = BenchState::from_planted_community(scale, 82_250 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| {
                black_box(label_propagation(
                    &state.projection,
                    BENCH_LABEL_PROPAGATION_MAX_ITER,
                ))
            });
        });
    }
    group.finish();
}

fn bench_louvain(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/louvain");
    for &scale in profile_scales() {
        // Local planted communities give Louvain an actual modularity structure.
        let state = BenchState::from_planted_community(scale, 82_300 + scale as u64);
        group.bench_function(BenchmarkId::from_parameter(scale_label(scale)), move |b| {
            b.iter(|| black_box(louvain(&state.projection, BENCH_LOUVAIN_MAX_ITER)));
        });
    }
    group.finish();
}

struct BenchState {
    projection: GraphProjection,
}

impl BenchState {
    fn from_bench_fixture(scale: usize) -> Self {
        let fixture = BenchFixture::build(scale);
        Self {
            projection: build_projection(fixture.graph()),
        }
    }

    fn from_planted_community(scale: usize, seed: u64) -> Self {
        let graph = planted_community_graph(scale, seed);
        let snapshot = graph.read();
        Self {
            projection: build_projection(&snapshot),
        }
    }

    fn from_dag(scale: usize, graph_id: u64) -> Self {
        let graph = dag_graph(scale, graph_id);
        let snapshot = graph.read();
        Self {
            projection: build_projection(&snapshot),
        }
    }
}

fn dag_graph(scale: usize, graph_id: u64) -> SharedGraph {
    let scale = scale.max(2);
    let graph = SharedGraph::new(GraphId::new(graph_id));
    let node_label = db_string("AlgoBench");
    let rel = db_string("LINK");
    let mut txn = graph.begin_write();
    let mut nodes = Vec::with_capacity(scale);
    for _ in 0..scale {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .expect("bench node inserts"),
        );
    }

    for source in 0..scale {
        for offset in [1_usize, 2, 4] {
            let target = source + offset;
            if target < scale {
                txn.mutator()
                    .create_edge(
                        rel.clone(),
                        nodes[source],
                        nodes[target],
                        PropertyMap::new(),
                    )
                    .expect("bench dag edge inserts");
            }
        }
    }
    txn.commit().expect("bench graph commits");
    graph
}

fn planted_community_graph(scale: usize, graph_id: u64) -> SharedGraph {
    let scale = scale.max(6);
    let graph = SharedGraph::new(GraphId::new(graph_id));
    let node_label = db_string("AlgoBench");
    let rel = db_string("LINK");
    let mut txn = graph.begin_write();
    let mut nodes = Vec::with_capacity(scale);
    for _ in 0..scale {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .expect("bench node inserts"),
        );
    }

    let community_count = (scale / 64).max(2);
    let community_size = scale.div_ceil(community_count);
    for community in 0..community_count {
        let start = community * community_size;
        let end = ((community + 1) * community_size).min(scale);
        if end <= start + 1 {
            continue;
        }
        let len = end - start;
        for local in 0..len {
            let source = start + local;
            for offset in [1_usize, 2, 3] {
                if offset >= len {
                    continue;
                }
                let target = start + ((local + offset) % len);
                create_undirected_edge(&mut txn, rel.clone(), nodes[source], nodes[target]);
            }
        }
        if community + 1 < community_count {
            let next = ((community + 1) * community_size).min(scale - 1);
            create_undirected_edge(&mut txn, rel.clone(), nodes[end - 1], nodes[next]);
        }
    }
    txn.commit().expect("bench graph commits");
    graph
}

fn create_undirected_edge(
    txn: &mut selene_graph::WriteTxn<'_>,
    rel: DbString,
    source: NodeId,
    target: NodeId,
) {
    txn.mutator()
        .create_edge(rel.clone(), source, target, PropertyMap::new())
        .expect("bench edge inserts");
    txn.mutator()
        .create_edge(rel, target, source, PropertyMap::new())
        .expect("bench reverse edge inserts");
}

fn betweenness_sample_size(scale: usize) -> Option<usize> {
    (scale > BENCH_BETWEENNESS_SAMPLE_SIZE).then_some(BENCH_BETWEENNESS_SAMPLE_SIZE)
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench_pagerank,
        bench_betweenness,
        bench_triangle_count,
        bench_apsp,
        bench_topological_sort,
        bench_wcc,
        bench_wcc_count,
        bench_scc,
        bench_scc_count,
        bench_label_propagation,
        bench_louvain
}
criterion_main!(benches);

#[cfg(test)]
mod tests {
    #[test]
    fn benches_compile() {}
}
