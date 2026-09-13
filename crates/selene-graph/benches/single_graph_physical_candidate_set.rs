use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{GraphId, LabelSet, NodeId, PropertyMap};
use selene_graph::{CandidateSet, Node, SeleneGraph, SharedGraph, VectorCandidateSet};

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
    group.bench_function(BenchmarkId::new("bind_canonical_ids", fixture.width), |b| {
        b.iter(|| {
            let candidates = fixture
                .graph
                .bind_node_candidates(std::hint::black_box(&fixture.canonical_ids).iter().copied())
                .expect("bench ids bind to the pinned snapshot");
            std::hint::black_box(candidates.len());
        });
    });
    group.bench_function(
        BenchmarkId::new("bind_noncanonical_duplicate_ids", fixture.width),
        |b| {
            b.iter(|| {
                let candidates = fixture
                    .graph
                    .bind_node_candidates(
                        std::hint::black_box(&fixture.noncanonical_ids)
                            .iter()
                            .copied(),
                    )
                    .expect("bench ids bind to the pinned snapshot");
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(
        BenchmarkId::new("bind_vector_candidate_set", fixture.width),
        |b| {
            b.iter(|| {
                let candidates = fixture
                    .graph
                    .bind_vector_candidate_set(std::hint::black_box(&fixture.vector_candidates))
                    .expect("bench vector candidates bind to the pinned snapshot");
                std::hint::black_box(candidates.len());
            });
        },
    );
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
    canonical_ids: Vec<NodeId>,
    noncanonical_ids: Vec<NodeId>,
    vector_candidates: VectorCandidateSet,
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
        let canonical_ids = candidates.iter().collect::<Vec<_>>();
        let mut noncanonical_ids = canonical_ids.iter().rev().copied().collect::<Vec<_>>();
        noncanonical_ids.extend(canonical_ids.iter().step_by(4).copied());
        let vector_candidates = VectorCandidateSet::from_nodes(canonical_ids.iter().copied());
        Self {
            graph,
            candidates,
            canonical_ids,
            noncanonical_ids,
            vector_candidates,
            width,
        }
    }
}
