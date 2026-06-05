use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{EdgeId, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_graph::{
    AdjacencyEdge, AdjacencyEntry, SeleneGraph, SharedGraph, VectorCandidateSet,
    VectorNeighborDirection, VectorNodeSearchHit,
};

const VECTOR_CANDIDATE_NEIGHBORS: usize = 64;
const VECTOR_CANDIDATE_ALGEBRA_SET: usize = 256;
const VECTOR_CANDIDATE_ALGEBRA_OVERLAP: usize = 128;
const VECTOR_CANDIDATE_ASYM_SMALL_SET: usize = 8;
const VECTOR_CANDIDATE_ASYM_LARGE_SET: usize = 1024;
const ADJACENCY_LABEL_MATCHING_EDGES: usize = 64;
const ADJACENCY_LABEL_NOISE_LABELS: usize = 8;

pub(super) fn bench_vector_candidate_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_candidate_set");
    for scale in super::vector_scan_scales() {
        let fixture = VectorCandidateFixture::build(scale, 128, VECTOR_CANDIDATE_NEIGHBORS);
        group.throughput(Throughput::Elements(fixture.candidate_count() as u64));
        group.bench_with_input(
            BenchmarkId::new(
                format!(
                    "neighbor_candidates_depends_on_k{}",
                    fixture.candidate_count()
                ),
                fixture.scale(),
            ),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let candidates = fixture.graph().vector_neighbor_candidates(
                        fixture.anchor(),
                        fixture.edge_label(),
                        VectorNeighborDirection::Outgoing,
                    );
                    std::hint::black_box(candidates.len());
                });
            },
        );
    }
    let adjacency =
        AdjacencyLabelFixture::build(ADJACENCY_LABEL_MATCHING_EDGES, ADJACENCY_LABEL_NOISE_LABELS);
    group.throughput(Throughput::Elements(adjacency.total_edges() as u64));
    group.bench_function(
        BenchmarkId::new(
            adjacency.bench_id("adjacency_label_range"),
            adjacency.matches(),
        ),
        |b| b.iter(|| std::hint::black_box(adjacency.range_count())),
    );
    group.bench_function(
        BenchmarkId::new(
            adjacency.bench_id("adjacency_label_scan"),
            adjacency.matches(),
        ),
        |b| b.iter(|| std::hint::black_box(adjacency.scan_count())),
    );
    bench_candidate_set_algebra(&mut group);
    group.finish();
}

fn bench_candidate_set_algebra(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
) {
    let fixture = VectorCandidateAlgebraFixture::build(
        VECTOR_CANDIDATE_ALGEBRA_SET,
        VECTOR_CANDIDATE_ALGEBRA_OVERLAP,
    );
    group.throughput(Throughput::Elements(fixture.set_width() as u64));
    group.bench_function(
        BenchmarkId::new(
            fixture.bench_id("set_intersection"),
            fixture.overlap_width(),
        ),
        |b| {
            b.iter(|| {
                let candidates = fixture.left().intersection(fixture.right());
                std::hint::black_box(candidates.len());
            });
        },
    );
    let asymmetric_fixture = VectorCandidateAlgebraFixture::build_asymmetric(
        VECTOR_CANDIDATE_ASYM_SMALL_SET,
        VECTOR_CANDIDATE_ASYM_LARGE_SET,
    );
    group.throughput(Throughput::Elements(asymmetric_fixture.left_width() as u64));
    group.bench_function(
        BenchmarkId::new(
            asymmetric_fixture.bench_id("set_intersection"),
            asymmetric_fixture.overlap_width(),
        ),
        |b| {
            b.iter(|| {
                let candidates = asymmetric_fixture
                    .left()
                    .intersection(asymmetric_fixture.right());
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.throughput(Throughput::Elements(fixture.set_width() as u64));
    group.bench_function(
        BenchmarkId::new(fixture.bench_id("set_union"), fixture.overlap_width()),
        |b| {
            b.iter(|| {
                let candidates = fixture.left().union(fixture.right());
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(
        BenchmarkId::new(fixture.bench_id("set_difference"), fixture.overlap_width()),
        |b| {
            b.iter(|| {
                let candidates = fixture.left().difference(fixture.right());
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(
        BenchmarkId::new(fixture.bench_id("from_search_hits"), fixture.set_width()),
        |b| {
            b.iter(|| {
                let candidates = VectorCandidateSet::from_search_hits(fixture.hits());
                std::hint::black_box(candidates.len());
            });
        },
    );
}

#[derive(Clone, Debug)]
struct AdjacencyLabelFixture {
    entry: AdjacencyEntry,
    label: IStr,
    matching_edges: usize,
    noise_labels: usize,
}

impl AdjacencyLabelFixture {
    fn build(matching_edges: usize, noise_labels: usize) -> Self {
        let label = intern("DEPENDS_ON").expect("bench edge label is valid");
        let mut entry = AdjacencyEntry::new();
        let mut edge_id = 1_u64;
        for label_idx in 0..noise_labels {
            let noise = intern(&format!("NOISE_{label_idx}")).expect("bench edge label is valid");
            for _ in 0..matching_edges {
                entry.add(adjacency_edge(noise.clone(), edge_id));
                edge_id += 1;
            }
        }
        for _ in 0..matching_edges {
            entry.add(adjacency_edge(label.clone(), edge_id));
            edge_id += 1;
        }
        Self {
            entry,
            label,
            matching_edges,
            noise_labels,
        }
    }

    fn range_count(&self) -> usize {
        self.entry.iter_label(&self.label).count()
    }

    fn scan_count(&self) -> usize {
        self.entry
            .iter()
            .filter(|edge| edge.label == self.label)
            .count()
    }

    fn bench_id(&self, prefix: &str) -> String {
        format!("{prefix}_l{}_k{}", self.noise_labels, self.matching_edges)
    }

    const fn matches(&self) -> usize {
        self.matching_edges
    }

    const fn total_edges(&self) -> usize {
        self.matching_edges * (self.noise_labels + 1)
    }
}

fn adjacency_edge(label: IStr, edge_id: u64) -> AdjacencyEdge {
    AdjacencyEdge {
        label,
        neighbor: NodeId::new(10_000 + edge_id),
        edge_id: EdgeId::new(edge_id),
    }
}

#[derive(Clone, Debug)]
struct VectorCandidateAlgebraFixture {
    left: VectorCandidateSet,
    right: VectorCandidateSet,
    hits: Vec<VectorNodeSearchHit>,
    left_width: usize,
    right_width: usize,
    overlap_width: usize,
}

impl VectorCandidateAlgebraFixture {
    fn build(set_width: usize, overlap_width: usize) -> Self {
        let overlap_width = overlap_width.min(set_width);
        let right_start = set_width - overlap_width + 1;
        let left_nodes = (1..=set_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        let right_nodes = (right_start..right_start + set_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        let hits = right_nodes
            .iter()
            .enumerate()
            .map(|(offset, node_id)| VectorNodeSearchHit {
                node_id: *node_id,
                distance: offset as f64,
            })
            .collect::<Vec<_>>();
        Self {
            left: VectorCandidateSet::from_nodes(left_nodes),
            right: VectorCandidateSet::from_nodes(right_nodes),
            hits,
            left_width: set_width,
            right_width: set_width,
            overlap_width,
        }
    }

    fn build_asymmetric(left_width: usize, right_width: usize) -> Self {
        let overlap_width = left_width.min(right_width);
        let left_nodes = (1..=left_width)
            .map(|id| NodeId::new(id as u64 * 64))
            .collect::<Vec<_>>();
        let right_nodes = (1..=right_width)
            .map(|id| NodeId::new(id as u64))
            .collect::<Vec<_>>();
        Self {
            left: VectorCandidateSet::from_nodes(left_nodes),
            right: VectorCandidateSet::from_nodes(right_nodes),
            hits: Vec::new(),
            left_width,
            right_width,
            overlap_width,
        }
    }

    fn bench_id(&self, prefix: &str) -> String {
        format!(
            "{prefix}_l{}_r{}_o{}",
            self.left_width, self.right_width, self.overlap_width
        )
    }

    const fn left(&self) -> &VectorCandidateSet {
        &self.left
    }

    const fn right(&self) -> &VectorCandidateSet {
        &self.right
    }

    fn hits(&self) -> &[VectorNodeSearchHit] {
        &self.hits
    }

    const fn set_width(&self) -> usize {
        self.left_width
    }

    const fn left_width(&self) -> usize {
        self.left_width
    }

    const fn overlap_width(&self) -> usize {
        self.overlap_width
    }
}

#[derive(Clone, Debug)]
struct VectorCandidateFixture {
    scale: usize,
    candidate_count: usize,
    graph: SeleneGraph,
    anchor: NodeId,
    edge_label: IStr,
}

impl VectorCandidateFixture {
    fn build(scale: usize, dimension: usize, target_candidates: usize) -> Self {
        let scale = scale.max(target_candidates.max(1));
        let anchor_label = intern("VectorAnchor").expect("bench label is valid");
        let doc_label = intern("VectorDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let edge_label = intern("DEPENDS_ON").expect("bench edge label is valid");
        let shared = SharedGraph::new(GraphId::new(19_000 + scale as u64));
        let (anchor, candidate_count) = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let anchor = mutator
                .create_node(LabelSet::single(anchor_label), PropertyMap::new())
                .expect("bench anchor insert succeeds");
            let mut first_nodes = Vec::with_capacity(target_candidates);
            for idx in 0..scale {
                let vector = Value::Vector(super::vector_value(idx, dimension));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                let node = mutator
                    .create_node(LabelSet::single(doc_label.clone()), props)
                    .expect("bench vector node insert succeeds");
                if first_nodes.len() < target_candidates {
                    first_nodes.push(node);
                }
            }
            for node in &first_nodes {
                mutator
                    .create_edge(edge_label.clone(), anchor, *node, PropertyMap::new())
                    .expect("bench candidate edge insert succeeds");
            }
            txn.commit()
                .expect("bench candidate fixture commit succeeds");
            (anchor, first_nodes.len())
        };
        Self {
            scale,
            candidate_count,
            graph: shared.read().as_ref().clone(),
            anchor,
            edge_label,
        }
    }

    const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    const fn candidate_count(&self) -> usize {
        self.candidate_count
    }

    const fn anchor(&self) -> NodeId {
        self.anchor
    }

    const fn edge_label(&self) -> &IStr {
        &self.edge_label
    }
}
