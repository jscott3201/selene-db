use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{
    CancellationChecker, DbString, EdgeId, GraphId, LabelSet, NodeId, PropertyMap, Value,
    VectorMetric, VectorValue, db_string,
};
use selene_graph::{
    AdjacencyEdge, AdjacencyEntry, CandidateStateSpec, IndexProvider,
    MaintainedCandidateStateProvider, SeleneGraph, SharedGraph, VectorCandidateSet,
    VectorNeighborDirection, VectorNeighborSearchOptions, VectorNodeSearchHit,
};

const VECTOR_CANDIDATE_NEIGHBORS: usize = 64;
const VECTOR_CANDIDATE_SCORE_DIMENSION: usize = 1024;
const VECTOR_CANDIDATE_SCORE_WIDTHS: &[usize] = &[64, 256, 1024, 4096];
const VECTOR_CANDIDATE_SCORE_BATCH_QUERIES: &[usize] = &[8, 64];
const VECTOR_CANDIDATE_ALGEBRA_SET: usize = 256;
const VECTOR_CANDIDATE_ALGEBRA_OVERLAP: usize = 128;
const VECTOR_CANDIDATE_ASYM_SMALL_SET: usize = 8;
const VECTOR_CANDIDATE_ASYM_LARGE_SET: usize = 1024;
const ADJACENCY_LABEL_MATCHING_EDGES: usize = 64;
const ADJACENCY_LABEL_NOISE_LABELS: usize = 8;
const CANDIDATE_STATE_ACTIVE_NODES: usize = 512;
const CANDIDATE_STATE_STALE_NODES: usize = 512;

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
    bench_candidate_set_scoring(&mut group);
    bench_candidate_set_algebra(&mut group);
    group.finish();
    bench_candidate_state(c);
}

fn bench_candidate_set_scoring(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
) {
    for &width in VECTOR_CANDIDATE_SCORE_WIDTHS {
        let fixture = VectorCandidateFixture::build(width, VECTOR_CANDIDATE_SCORE_DIMENSION, width);
        group.throughput(Throughput::Elements(fixture.candidate_count() as u64));
        group.bench_function(
            BenchmarkId::new(
                fixture.bench_id("score_candidate_set_cosine"),
                fixture.candidate_count(),
            ),
            |b| {
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .score_vector_candidate_set(
                            fixture.embedding_key(),
                            fixture.query(),
                            fixture.candidate_set(),
                            VectorMetric::Cosine,
                            10,
                        )
                        .expect("bench candidate scoring succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                fixture.bench_id("score_candidate_set_cosine_checked_with_deadline"),
                fixture.candidate_count(),
            ),
            |b| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph()
                        .score_vector_candidate_set_checked(
                            fixture.embedding_key(),
                            fixture.query(),
                            fixture.candidate_set(),
                            VectorMetric::Cosine,
                            10,
                            checker,
                        )
                        .expect("bench candidate scoring succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        for &query_count in VECTOR_CANDIDATE_SCORE_BATCH_QUERIES {
            let queries = (0..query_count)
                .map(|idx| super::vector_value(idx, VECTOR_CANDIDATE_SCORE_DIMENSION))
                .collect::<Vec<_>>();
            let node_sets = vec![fixture.candidate_set().as_nodes().to_vec(); query_count];
            let candidate_sets = vec![fixture.candidate_set().clone(); query_count];
            group.throughput(Throughput::Elements(
                (query_count * fixture.candidate_count()) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    fixture.bench_id(&format!("score_nodes_batch_cosine_q{query_count}")),
                    fixture.candidate_count(),
                ),
                |b| {
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .score_vector_nodes_batch(
                                fixture.embedding_key(),
                                &queries,
                                &node_sets,
                                VectorMetric::Cosine,
                                10,
                            )
                            .expect("bench explicit node batch scoring succeeds");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
            group.bench_function(
                BenchmarkId::new(
                    fixture.bench_id(&format!("score_candidate_sets_batch_cosine_q{query_count}")),
                    fixture.candidate_count(),
                ),
                |b| {
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .score_vector_candidate_sets_batch(
                                fixture.embedding_key(),
                                &queries,
                                &candidate_sets,
                                VectorMetric::Cosine,
                                10,
                            )
                            .expect("bench candidate batch scoring succeeds");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
            let root_sets = vec![VectorCandidateSet::from_nodes([fixture.anchor()]); query_count];
            group.bench_function(
                BenchmarkId::new(
                    fixture.bench_id(&format!("score_expanded_batch_cosine_q{query_count}")),
                    fixture.candidate_count(),
                ),
                |b| {
                    b.iter(|| {
                        let hits = fixture
                            .graph()
                            .score_vector_expanded_candidate_sets_batch(
                                fixture.embedding_key(),
                                &queries,
                                &root_sets,
                                VectorNeighborSearchOptions::new(
                                    fixture.edge_label(),
                                    VectorNeighborDirection::Outgoing,
                                    VectorMetric::Cosine,
                                    10,
                                ),
                            )
                            .expect("bench expanded batch scoring succeeds");
                        std::hint::black_box(hits.iter().map(Vec::len).sum::<usize>());
                    });
                },
            );
        }
    }
}

fn bench_candidate_state(c: &mut Criterion) {
    let fixture = MaintainedCandidateStateFixture::build(
        CANDIDATE_STATE_ACTIVE_NODES,
        CANDIDATE_STATE_STALE_NODES,
    );
    let mut group = c.benchmark_group("graph_vector_candidate_state");
    group.throughput(Throughput::Elements(fixture.total_nodes() as u64));
    group.bench_function(
        BenchmarkId::new(
            fixture.bench_id("maintained_active"),
            fixture.active_nodes(),
        ),
        |b| {
            b.iter(|| {
                let candidates = fixture.maintained_candidate_set();
                std::hint::black_box(candidates.len());
            });
        },
    );
    group.bench_function(
        BenchmarkId::new(
            fixture.bench_id("dynamic_active_scan"),
            fixture.active_nodes(),
        ),
        |b| {
            b.iter(|| {
                let candidates = fixture.dynamic_candidate_set();
                std::hint::black_box(candidates.len());
            });
        },
    );
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
    label: DbString,
    matching_edges: usize,
    noise_labels: usize,
}

impl AdjacencyLabelFixture {
    fn build(matching_edges: usize, noise_labels: usize) -> Self {
        let label = db_string("DEPENDS_ON").expect("bench edge label is valid");
        let mut entry = AdjacencyEntry::new();
        let mut edge_id = 1_u64;
        for label_idx in 0..noise_labels {
            let noise =
                db_string(&format!("NOISE_{label_idx}")).expect("bench edge label is valid");
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

fn adjacency_edge(label: DbString, edge_id: u64) -> AdjacencyEdge {
    AdjacencyEdge {
        label,
        neighbor: NodeId::new(10_000 + edge_id),
        edge_id: EdgeId::new(edge_id),
    }
}

fn deadline_checker() -> CancellationChecker<'static> {
    CancellationChecker::new(None, Some(Instant::now() + Duration::from_secs(3600)))
}

struct MaintainedCandidateStateFixture {
    graph: SeleneGraph,
    provider: Arc<MaintainedCandidateStateProvider>,
    set_name: DbString,
    superseded: DbString,
    docs: Vec<NodeId>,
    active_count: usize,
    stale_count: usize,
}

impl MaintainedCandidateStateFixture {
    fn build(active_count: usize, stale_count: usize) -> Self {
        let set_name = db_string("current").expect("bench set name is valid");
        let doc_label = db_string("MemoryFact").expect("bench label is valid");
        let superseded = db_string("SUPERSEDED_BY").expect("bench edge label is valid");
        let spec = CandidateStateSpec::new(set_name.clone())
            .require_label(doc_label.clone())
            .exclude_outgoing(superseded.clone());
        let provider = Arc::new(
            MaintainedCandidateStateProvider::new([spec]).expect("bench provider is valid"),
        );
        let shared = SharedGraph::builder(GraphId::new(21_000 + active_count as u64))
            .with_provider(provider.clone() as Arc<dyn IndexProvider>)
            .build()
            .expect("bench graph builds");
        let docs = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let mut active = Vec::with_capacity(active_count);
            let mut stale = Vec::with_capacity(stale_count);
            for _ in 0..active_count {
                active.push(
                    mutator
                        .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
                        .expect("bench active node insert succeeds"),
                );
            }
            for idx in 0..stale_count {
                let node = mutator
                    .create_node(LabelSet::single(doc_label.clone()), PropertyMap::new())
                    .expect("bench stale node insert succeeds");
                let target = active[idx % active.len()];
                mutator
                    .create_edge(superseded.clone(), node, target, PropertyMap::new())
                    .expect("bench stale edge insert succeeds");
                stale.push(node);
            }
            txn.commit()
                .expect("bench candidate-state fixture commit succeeds");
            active.into_iter().chain(stale).collect::<Vec<_>>()
        };
        Self {
            graph: shared.read().as_ref().clone(),
            provider,
            set_name,
            superseded,
            docs,
            active_count,
            stale_count,
        }
    }

    fn maintained_candidate_set(&self) -> VectorCandidateSet {
        self.provider
            .candidate_set(&self.set_name)
            .expect("bench set is configured")
    }

    fn dynamic_candidate_set(&self) -> VectorCandidateSet {
        VectorCandidateSet::from_nodes(self.docs.iter().copied().filter(|node| {
            self.graph
                .outgoing_edges(*node)
                .is_none_or(|entry| entry.iter_label(&self.superseded).next().is_none())
        }))
    }

    fn bench_id(&self, prefix: &str) -> String {
        format!(
            "{prefix}_c{}_total{}",
            self.active_count,
            self.total_nodes()
        )
    }

    const fn active_nodes(&self) -> usize {
        self.active_count
    }

    const fn total_nodes(&self) -> usize {
        self.active_count + self.stale_count
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
    edge_label: DbString,
    embedding_key: DbString,
    query: VectorValue,
    candidate_set: VectorCandidateSet,
}

impl VectorCandidateFixture {
    fn build(scale: usize, dimension: usize, target_candidates: usize) -> Self {
        let scale = scale.max(target_candidates.max(1));
        let anchor_label = db_string("VectorAnchor").expect("bench label is valid");
        let doc_label = db_string("VectorDoc").expect("bench label is valid");
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let edge_label = db_string("DEPENDS_ON").expect("bench edge label is valid");
        let shared = SharedGraph::new(GraphId::new(19_000 + scale as u64));
        let (anchor, candidate_count, candidate_set) = {
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
            let candidate_count = first_nodes.len();
            (
                anchor,
                candidate_count,
                VectorCandidateSet::from_nodes(first_nodes),
            )
        };
        Self {
            scale,
            candidate_count,
            graph: shared.read().as_ref().clone(),
            anchor,
            edge_label,
            embedding_key,
            query: super::vector_value(0, dimension),
            candidate_set,
        }
    }

    fn bench_id(&self, prefix: &str) -> String {
        format!(
            "{prefix}_c{}_d{}",
            self.candidate_count, VECTOR_CANDIDATE_SCORE_DIMENSION
        )
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

    const fn edge_label(&self) -> &DbString {
        &self.edge_label
    }

    const fn embedding_key(&self) -> &DbString {
        &self.embedding_key
    }

    const fn query(&self) -> &VectorValue {
        &self.query
    }

    const fn candidate_set(&self) -> &VectorCandidateSet {
        &self.candidate_set
    }
}
