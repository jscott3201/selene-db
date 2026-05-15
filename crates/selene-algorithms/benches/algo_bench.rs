#![allow(missing_docs)]
//! Criterion benches for sequential graph-algorithm baselines.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_algorithms::{
    ApspConfig, BetweennessConfig, GraphProjection, PageRankConfig, Parallelism, ProjectionConfig,
    TriangleCountConfig, apsp, betweenness, louvain, pagerank, triangle_count,
};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::{SeleneGraph, SharedGraph};
use selene_testing::{BenchFixture, BenchProfile};

const BENCH_SCALES: &[usize] = &[200, 1_000, 10_000];
const APSP_SCALES: &[usize] = &[200, 500, 1_000];
const BENCH_BETWEENNESS_SAMPLE_SIZE: usize = 256;
const BENCH_LOUVAIN_MAX_ITER: usize = 50;
const PARALLELISM_BENCH_MODES: &[(&str, Parallelism)] = &[
    ("sequential", Parallelism::Sequential),
    ("auto", Parallelism::Auto),
];

const BENCH_PAGERANK_CONFIG: PageRankConfig = PageRankConfig {
    damping: 0.85,
    max_iter: 100,
    tolerance: 1e-6,
    parallelism: Parallelism::Sequential,
};

fn bench_pagerank(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/pagerank");
    for &scale in BENCH_SCALES {
        let state = BenchState::from_bench_fixture(scale);
        for &(mode, parallelism) in PARALLELISM_BENCH_MODES {
            let config = PageRankConfig {
                parallelism,
                ..BENCH_PAGERANK_CONFIG
            };
            group.bench_function(BenchmarkId::new(mode, scale_label(scale)), |b| {
                b.iter(|| black_box(pagerank(&state.projection, config)));
            });
        }
    }
    group.finish();
}

fn bench_betweenness(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/betweenness");
    for &scale in BENCH_SCALES {
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
    for &scale in BENCH_SCALES {
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

fn bench_louvain(c: &mut Criterion) {
    let mut group = c.benchmark_group("algo/louvain");
    for &scale in BENCH_SCALES {
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
}

fn build_projection(snapshot: &SeleneGraph) -> GraphProjection {
    let config = ProjectionConfig {
        name: "bench".to_string(),
        node_labels: Vec::new(),
        edge_labels: Vec::new(),
        weight_property: None,
    };
    GraphProjection::build(snapshot, &config, None).expect("bench projection builds")
}

fn planted_community_graph(scale: usize, graph_id: u64) -> SharedGraph {
    let scale = scale.max(6);
    let graph = SharedGraph::new(GraphId::new(graph_id));
    let node_label = istr("AlgoBench");
    let rel = istr("LINK");
    let mut txn = graph.begin_write();
    let mut nodes = Vec::with_capacity(scale);
    for _ in 0..scale {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(node_label), PropertyMap::new())
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
                create_undirected_edge(&mut txn, rel, nodes[source], nodes[target]);
            }
        }
        if community + 1 < community_count {
            let next = ((community + 1) * community_size).min(scale - 1);
            create_undirected_edge(&mut txn, rel, nodes[end - 1], nodes[next]);
        }
    }
    txn.commit().expect("bench graph commits");
    graph
}

fn create_undirected_edge(
    txn: &mut selene_graph::WriteTxn<'_>,
    rel: IStr,
    source: NodeId,
    target: NodeId,
) {
    txn.mutator()
        .create_edge(rel, source, target, PropertyMap::new())
        .expect("bench edge inserts");
    txn.mutator()
        .create_edge(rel, target, source, PropertyMap::new())
        .expect("bench reverse edge inserts");
}

fn betweenness_sample_size(scale: usize) -> Option<usize> {
    (scale > BENCH_BETWEENNESS_SAMPLE_SIZE).then_some(BENCH_BETWEENNESS_SAMPLE_SIZE)
}

fn scale_label(scale: usize) -> String {
    if scale >= 1_000 {
        format!("{}k", scale / 1_000)
    } else {
        scale.to_string()
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}

fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench_pagerank,
        bench_betweenness,
        bench_triangle_count,
        bench_apsp,
        bench_louvain
}
criterion_main!(benches);

#[cfg(test)]
mod tests {
    #[test]
    fn benches_compile() {}
}
