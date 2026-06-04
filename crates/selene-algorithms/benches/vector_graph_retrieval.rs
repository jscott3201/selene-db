#![allow(missing_docs)]
//! Criterion benches for graph-augmented vector retrieval research.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::collections::{HashMap, HashSet};
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_algorithms::{GraphProjection, PageRankConfig, Parallelism, ProjectionConfig, pagerank};
use selene_core::{
    CancellationChecker, GraphId, HnswIndexConfig, IStr, LabelSet, NodeId, PropertyMap, Value,
    VectorMetric, VectorValue, intern,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexConfig, VectorIndexKind,
    VectorNodeSearchHit,
};
use selene_testing::BenchProfile;

use common::{criterion_config, scale_label};

const DIMENSION: usize = 128;
const FACTS_PER_TOPIC: usize = 8;
const RESULT_K: usize = FACTS_PER_TOPIC;
const SEED_K: usize = 4;
const WIDE_SEED_K: usize = 16;
const ORACLE_SEED_K: usize = 64;
const SEARCH_WIDTH: usize = 32;
const PAGERANK_WEIGHT: f64 = 0.05;

const STRATEGIES: &[RetrievalStrategy] = &[
    RetrievalStrategy::VectorOnly,
    RetrievalStrategy::PagerankPrior,
    RetrievalStrategy::GraphExpand,
    RetrievalStrategy::GraphExpandValid,
    RetrievalStrategy::GraphExpandSuperseded,
    RetrievalStrategy::GraphExpandPagerank,
    RetrievalStrategy::ExactGraphOracle,
];

#[derive(Clone, Copy, Debug)]
enum RetrievalStrategy {
    VectorOnly,
    PagerankPrior,
    GraphExpand,
    GraphExpandValid,
    GraphExpandSuperseded,
    GraphExpandPagerank,
    ExactGraphOracle,
}

impl RetrievalStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::VectorOnly => "vector_only",
            Self::PagerankPrior => "pagerank_prior",
            Self::GraphExpand => "graph_expand",
            Self::GraphExpandValid => "graph_expand_valid",
            Self::GraphExpandSuperseded => "graph_expand_superseded",
            Self::GraphExpandPagerank => "graph_expand_pagerank",
            Self::ExactGraphOracle => "exact_graph_oracle",
        }
    }
}

fn bench_graph_augmented_vector_retrieval(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_retrieval");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build(scale);
        group.throughput(Throughput::Elements(
            (fixture.query_count() * RESULT_K) as u64,
        ));
        for &strategy in STRATEGIES {
            let quality = fixture.quality(strategy);
            group.bench_function(
                BenchmarkId::new(
                    strategy.name(),
                    format!(
                        "{}_q{}_covbp{}_curbp{}_precbp{}",
                        scale_label(fixture.scale()),
                        fixture.query_count(),
                        basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
                        basis_points(
                            quality.current_coverage,
                            fixture.query_count() * FACTS_PER_TOPIC
                        ),
                        basis_points(quality.precision, fixture.query_count() * RESULT_K),
                    ),
                ),
                |b| {
                    b.iter(|| {
                        black_box(fixture.total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

struct MemoryRetrievalFixture {
    graph: SeleneGraph,
    scale: usize,
    label: IStr,
    embedding_key: IStr,
    support_edge: IStr,
    valid_edge: IStr,
    superseded_by_edge: IStr,
    queries: Vec<Query>,
    metadata: HashMap<NodeId, NodeMeta>,
    pagerank: HashMap<NodeId, f64>,
}

impl MemoryRetrievalFixture {
    fn build(requested_scale: usize) -> Self {
        let label = istr("Memory");
        let embedding_key = istr("embedding");
        let support_edge = istr("SUPPORTS");
        let valid_edge = istr("VALID_AT");
        let superseded_by_edge = istr("SUPERSEDED_BY");
        let topic_count = topic_count(requested_scale);
        let duplicates = duplicates_per_fact(requested_scale, topic_count);
        let shared = SharedGraph::new(GraphId::new(91_000 + requested_scale as u64));
        let mut topic_nodes = vec![vec![Vec::new(); FACTS_PER_TOPIC]; topic_count];
        let mut metadata = HashMap::new();

        {
            let mut txn = shared.begin_write();
            {
                let mut mutator = txn.mutator();
                for (topic, facts) in topic_nodes.iter_mut().enumerate() {
                    for (fact, nodes) in facts.iter_mut().enumerate() {
                        for duplicate in 0..duplicates {
                            let vector = memory_vector(topic, fact, duplicate, 0.0);
                            let props = PropertyMap::from_pairs([(
                                embedding_key.clone(),
                                Value::Vector(vector),
                            )])
                            .expect("bench properties fit");
                            let node = mutator
                                .create_node(LabelSet::single(label.clone()), props)
                                .expect("bench node insert succeeds");
                            nodes.push(node);
                            metadata.insert(
                                node,
                                NodeMeta {
                                    topic,
                                    fact,
                                    current: fact == 0 || duplicate % 2 == 0,
                                },
                            );
                        }
                    }
                }
                for facts in &topic_nodes {
                    for (duplicate, summary) in facts[0].iter().enumerate() {
                        for evidence_nodes in facts.iter().skip(1) {
                            let evidence = evidence_nodes[duplicate % evidence_nodes.len()];
                            mutator
                                .create_edge(
                                    support_edge.clone(),
                                    *summary,
                                    evidence,
                                    PropertyMap::new(),
                                )
                                .expect("bench support edge inserts");
                            if metadata.get(&evidence).is_some_and(|meta| meta.current) {
                                mutator
                                    .create_edge(
                                        valid_edge.clone(),
                                        *summary,
                                        evidence,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench valid edge inserts");
                            } else {
                                let replacement = current_replacement(evidence_nodes, duplicate);
                                mutator
                                    .create_edge(
                                        superseded_by_edge.clone(),
                                        evidence,
                                        replacement,
                                        PropertyMap::new(),
                                    )
                                    .expect("bench supersession edge inserts");
                            }
                        }
                    }
                }
                mutator
                    .create_vector_index_named_with_configs(
                        label.clone(),
                        embedding_key.clone(),
                        VectorIndexKind::HnswCosine,
                        DIMENSION as u32,
                        None,
                        VectorIndexConfig::new(Some(HnswIndexConfig::new(24, 64)), None),
                    )
                    .expect("bench vector index build succeeds");
            }
            txn.commit().expect("bench graph commits");
        }

        let graph = shared.read().as_ref().clone();
        let pagerank = pagerank_scores(&graph, &label, &support_edge);
        let queries = (0..topic_count)
            .map(|topic| Query {
                topic,
                vector: memory_vector(topic, 0, duplicates / 2, 0.0003),
            })
            .collect();
        Self {
            graph,
            scale: topic_count * FACTS_PER_TOPIC * duplicates,
            label,
            embedding_key,
            support_edge,
            valid_edge,
            superseded_by_edge,
            queries,
            metadata,
            pagerank,
        }
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn quality(&self, strategy: RetrievalStrategy) -> RetrievalQuality {
        self.queries
            .iter()
            .map(|query| self.query_quality(query, strategy))
            .fold(RetrievalQuality::default(), |mut total, next| {
                total.coverage += next.coverage;
                total.current_coverage += next.current_coverage;
                total.precision += next.precision;
                total
            })
    }

    fn total_coverage(&self, strategy: RetrievalStrategy) -> usize {
        self.quality(strategy).coverage
    }

    fn query_quality(&self, query: &Query, strategy: RetrievalStrategy) -> RetrievalQuality {
        let selected = self.select(query, strategy);
        let mut facts = [false; FACTS_PER_TOPIC];
        let mut current_facts = [false; FACTS_PER_TOPIC];
        let mut precision = 0usize;
        for node in selected {
            if let Some(meta) = self.metadata.get(&node)
                && meta.topic == query.topic
            {
                precision += 1;
                facts[meta.fact] = true;
                if meta.current {
                    current_facts[meta.fact] = true;
                }
            }
        }
        RetrievalQuality {
            coverage: facts.into_iter().filter(|covered| *covered).count(),
            current_coverage: current_facts.into_iter().filter(|covered| *covered).count(),
            precision,
        }
    }

    fn select(&self, query: &Query, strategy: RetrievalStrategy) -> Vec<NodeId> {
        match strategy {
            RetrievalStrategy::VectorOnly => self
                .ann_hits(query, RESULT_K)
                .into_iter()
                .map(|hit| hit.node_id)
                .collect(),
            RetrievalStrategy::PagerankPrior => {
                let hits = self.ann_hits(query, WIDE_SEED_K);
                self.select_from_candidates(query, hits, false, true, false)
            }
            RetrievalStrategy::GraphExpand => {
                let hits = self.ann_hits(query, SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand(query, hits, false),
                    true,
                    false,
                    false,
                )
            }
            RetrievalStrategy::GraphExpandValid => {
                let hits = self.ann_hits(query, SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand(query, hits, true),
                    true,
                    false,
                    true,
                )
            }
            RetrievalStrategy::GraphExpandSuperseded => {
                let hits = self.ann_hits(query, SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand_with_supersession(query, hits),
                    true,
                    false,
                    true,
                )
            }
            RetrievalStrategy::GraphExpandPagerank => {
                let hits = self.ann_hits(query, SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand(query, hits, false),
                    true,
                    true,
                    false,
                )
            }
            RetrievalStrategy::ExactGraphOracle => {
                let hits = self.exact_hits(query, ORACLE_SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand(query, hits, true),
                    true,
                    false,
                    true,
                )
            }
        }
    }

    fn ann_hits(&self, query: &Query, k: usize) -> Vec<VectorNodeSearchHit> {
        self.graph
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &query.vector,
                ApproximateVectorSearchOptions::new(VectorMetric::Cosine, k, SEARCH_WIDTH),
                CancellationChecker::disabled(),
            )
            .expect("bench ANN search succeeds")
    }

    fn exact_hits(&self, query: &Query, k: usize) -> Vec<VectorNodeSearchHit> {
        self.graph
            .exact_vector_search_nodes(
                &self.label,
                &self.embedding_key,
                &query.vector,
                VectorMetric::Cosine,
                k,
            )
            .expect("bench exact search succeeds")
    }

    fn expand(
        &self,
        query: &Query,
        hits: Vec<VectorNodeSearchHit>,
        valid_only: bool,
    ) -> Vec<VectorNodeSearchHit> {
        let mut candidates = Vec::new();
        for hit in hits {
            let node_id = hit.node_id;
            candidates.push(node_id);
            if let Some(edges) = self.graph.outgoing_edges(node_id) {
                for edge in edges.iter().filter(|edge| edge.label == self.support_edge) {
                    if valid_only && !self.has_valid_edge(node_id, edge.neighbor) {
                        continue;
                    }
                    candidates.push(edge.neighbor);
                }
            }
        }
        self.score_candidate_ids(query, candidates)
    }

    fn expand_with_supersession(
        &self,
        query: &Query,
        hits: Vec<VectorNodeSearchHit>,
    ) -> Vec<VectorNodeSearchHit> {
        let mut candidates = Vec::new();
        for hit in hits {
            let node_id = hit.node_id;
            candidates.push(node_id);
            if let Some(edges) = self.graph.outgoing_edges(node_id) {
                for edge in edges.iter().filter(|edge| edge.label == self.support_edge) {
                    let neighbor = self
                        .superseded_replacement(edge.neighbor)
                        .unwrap_or(edge.neighbor);
                    candidates.push(neighbor);
                }
            }
        }
        self.score_candidate_ids(query, candidates)
    }

    fn has_valid_edge(&self, source: NodeId, target: NodeId) -> bool {
        self.graph.outgoing_edges(source).is_some_and(|edges| {
            edges
                .iter()
                .any(|edge| edge.label == self.valid_edge && edge.neighbor == target)
        })
    }

    fn superseded_replacement(&self, node_id: NodeId) -> Option<NodeId> {
        self.graph.outgoing_edges(node_id).and_then(|edges| {
            edges
                .iter()
                .find(|edge| edge.label == self.superseded_by_edge)
                .map(|edge| edge.neighbor)
        })
    }

    fn score_candidate_ids<I>(&self, query: &Query, candidates: I) -> Vec<VectorNodeSearchHit>
    where
        I: IntoIterator<Item = NodeId>,
    {
        let candidates = candidates.into_iter().collect::<Vec<_>>();
        self.graph
            .score_vector_nodes_checked(
                &self.embedding_key,
                &query.vector,
                &candidates,
                VectorMetric::Cosine,
                candidates.len(),
                CancellationChecker::disabled(),
            )
            .expect("bench candidate scoring succeeds")
    }

    fn select_from_candidates(
        &self,
        query: &Query,
        mut candidates: Vec<VectorNodeSearchHit>,
        diversify: bool,
        use_prior: bool,
        require_current: bool,
    ) -> Vec<NodeId> {
        candidates.sort_by(|left, right| {
            self.rank_score(left, use_prior)
                .total_cmp(&self.rank_score(right, use_prior))
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        if !diversify {
            return candidates
                .into_iter()
                .take(RESULT_K)
                .map(|hit| hit.node_id)
                .collect();
        }

        let mut selected = Vec::with_capacity(RESULT_K);
        let mut seen_facts = HashSet::new();
        let mut deferred = Vec::new();
        for hit in candidates {
            if require_current && !self.is_current(hit.node_id) {
                continue;
            }
            let fact_key = self
                .metadata
                .get(&hit.node_id)
                .filter(|meta| meta.topic == query.topic)
                .map(|meta| meta.fact);
            if let Some(fact) = fact_key
                && seen_facts.insert(fact)
            {
                selected.push(hit.node_id);
            } else {
                deferred.push(hit.node_id);
            }
            if selected.len() == RESULT_K {
                return selected;
            }
        }
        selected.extend(deferred.into_iter().take(RESULT_K - selected.len()));
        selected
    }

    fn is_current(&self, node_id: NodeId) -> bool {
        self.metadata
            .get(&node_id)
            .is_some_and(|metadata| metadata.current)
    }

    fn rank_score(&self, hit: &VectorNodeSearchHit, use_prior: bool) -> f64 {
        if use_prior {
            hit.distance - self.pagerank.get(&hit.node_id).copied().unwrap_or(0.0) * PAGERANK_WEIGHT
        } else {
            hit.distance
        }
    }
}

#[derive(Clone, Debug)]
struct Query {
    topic: usize,
    vector: VectorValue,
}

#[derive(Clone, Copy, Debug)]
struct NodeMeta {
    topic: usize,
    fact: usize,
    current: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct RetrievalQuality {
    coverage: usize,
    current_coverage: usize,
    precision: usize,
}

fn pagerank_scores(graph: &SeleneGraph, label: &IStr, support_edge: &IStr) -> HashMap<NodeId, f64> {
    let projection = GraphProjection::build(
        graph,
        &ProjectionConfig {
            name: "memory_retrieval".to_owned(),
            node_labels: vec![label.clone()],
            edge_labels: vec![support_edge.clone()],
            weight_property: None,
        },
        None,
    )
    .expect("bench projection builds");
    let scores = pagerank(
        &projection,
        PageRankConfig {
            damping: 0.85,
            max_iter: 32,
            tolerance: 1e-6,
            parallelism: Parallelism::Sequential,
        },
    );
    let max = scores.iter().map(|(_, score)| *score).fold(0.0, f64::max);
    scores
        .into_iter()
        .map(|(node, score)| (node, if max > 0.0 { score / max } else { 0.0 }))
        .collect()
}

fn topic_count(scale: usize) -> usize {
    (scale / (FACTS_PER_TOPIC * 4)).clamp(4, 64)
}

fn duplicates_per_fact(scale: usize, topics: usize) -> usize {
    (scale / (topics * FACTS_PER_TOPIC)).max(2)
}

fn current_replacement(nodes: &[NodeId], duplicate: usize) -> NodeId {
    let replacement = if duplicate.is_multiple_of(2) {
        duplicate
    } else {
        duplicate.saturating_sub(1)
    };
    nodes[replacement]
}

fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(parse_scales)
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn parse_scales(raw: String) -> Option<Vec<usize>> {
    let mut scales: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|scale| *scale > 0)
        .collect();
    scales.sort_unstable();
    scales.dedup();
    (!scales.is_empty()).then_some(scales)
}

fn memory_vector(topic: usize, fact: usize, duplicate: usize, shift: f32) -> VectorValue {
    let primary = topic % DIMENSION;
    let secondary = topic.wrapping_mul(5).wrapping_add(3) % DIMENSION;
    let fact_dim = topic.wrapping_mul(11).wrapping_add(fact * 17 + 7) % DIMENSION;
    let components: Vec<f32> = (0..DIMENSION)
        .map(|dim| {
            let base = (((topic + 3) * (dim + 11)) % 17) as f32 / 200.0;
            let topic_signal = if dim == primary { 1.0 } else { 0.0 };
            let secondary_signal = if dim == secondary { 0.25 } else { 0.0 };
            let fact_signal = if dim == fact_dim {
                fact as f32 * 0.055
            } else {
                0.0
            };
            let duplicate_noise =
                ((duplicate * (dim + 13) + fact * 31 + topic * 7) % 29) as f32 / 100_000.0;
            base + topic_signal + secondary_signal + fact_signal + duplicate_noise + shift
        })
        .collect();
    VectorValue::new(components).expect("bench vector is valid")
}

fn basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}

criterion_group! {
    name = vector_graph_retrieval;
    config = criterion_config();
    targets = bench_graph_augmented_vector_retrieval
}
criterion_main!(vector_graph_retrieval);
