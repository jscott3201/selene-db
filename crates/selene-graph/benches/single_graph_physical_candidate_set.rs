use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{GraphId, LabelSet, PropertyMap};
use selene_graph::{CandidateSet, Node, SeleneGraph, SharedGraph};

const PHYSICAL_CANDIDATE_WIDTH: usize = 1_024;

pub(super) fn bench_physical_candidate_set(c: &mut Criterion) {
    let fixture = PhysicalCandidateFixture::build(PHYSICAL_CANDIDATE_WIDTH);
    let mut group = c.benchmark_group("graph_physical_candidate_set");
    group.throughput(Throughput::Elements(fixture.width as u64));
    group.bench_function(BenchmarkId::new("build_live_nodes", fixture.width), |b| {
        b.iter(|| {
            let candidates = fixture
                .graph
                .live_node_candidates()
                .expect("bench graph has trusted typed inverse rows");
            std::hint::black_box(candidates.len());
        });
    });
    group.bench_function(BenchmarkId::new("iterate_stable_ids", fixture.width), |b| {
        b.iter(|| {
            let checksum = fixture
                .candidates
                .iter()
                .fold(0_u64, |sum, id| sum.wrapping_add(id.get()));
            std::hint::black_box(checksum);
        });
    });
    group.bench_function(BenchmarkId::new("union_full_overlap", fixture.width), |b| {
        b.iter(|| {
            let candidates = fixture
                .graph
                .union_candidates(&fixture.candidates, &fixture.candidates)
                .expect("same snapshot identity accepts algebra");
            std::hint::black_box(candidates.len());
        });
    });
    group.bench_function(
        BenchmarkId::new("intersection_full_overlap", fixture.width),
        |b| {
            b.iter(|| {
                let candidates = fixture
                    .graph
                    .intersect_candidates(&fixture.candidates, &fixture.candidates)
                    .expect("same snapshot identity accepts algebra");
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(
        BenchmarkId::new("difference_full_overlap", fixture.width),
        |b| {
            b.iter(|| {
                let candidates = fixture
                    .graph
                    .difference_candidates(&fixture.candidates, &fixture.candidates)
                    .expect("same snapshot identity accepts algebra");
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(BenchmarkId::new("clone_snapshot", fixture.width), |b| {
        b.iter(|| std::hint::black_box(fixture.graph.as_ref().clone()));
    });
    group.finish();
}

struct PhysicalCandidateFixture {
    graph: Arc<SeleneGraph>,
    candidates: CandidateSet<Node>,
    width: usize,
}

impl PhysicalCandidateFixture {
    fn build(width: usize) -> Self {
        let shared = SharedGraph::new(GraphId::new(61_000));
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            for _ in 0..width {
                mutator
                    .create_node(LabelSet::new(), PropertyMap::new())
                    .expect("bench node insert succeeds");
            }
        }
        txn.commit().expect("bench fixture commit succeeds");
        let graph = shared.read();
        let candidates = graph
            .live_node_candidates()
            .expect("bench graph has trusted typed inverse rows");
        Self {
            graph,
            candidates,
            width,
        }
    }
}
