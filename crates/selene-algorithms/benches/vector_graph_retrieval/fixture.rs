//! Support fixture for graph-augmented vector retrieval benchmarks.

use std::collections::{HashMap, HashSet};
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{CancellationChecker, IStr, NodeId, VectorMetric, VectorValue};
use selene_graph::{ApproximateVectorSearchOptions, SeleneGraph, VectorNodeSearchHit};

use crate::common::scale_label;

mod builder;
mod community_pressure;
mod component_pressure;
mod query_filter_pressure;
mod session_pressure;
mod support;
mod topology_pressure;

use support::{
    FACTS_PER_TOPIC, ORACLE_SEED_K, PAGERANK_WEIGHT, RESULT_K, SEARCH_WIDTH, SEED_K, WIDE_SEED_K,
    basis_points, vector_scales,
};

const STRATEGIES: &[RetrievalStrategy] = &[
    RetrievalStrategy::VectorOnly,
    RetrievalStrategy::PagerankPrior,
    RetrievalStrategy::GraphExpand,
    RetrievalStrategy::GraphExpandValid,
    RetrievalStrategy::GraphExpandSuperseded,
    RetrievalStrategy::GraphExpandValidWide,
    RetrievalStrategy::GraphExpandSupersededWide,
    RetrievalStrategy::GraphComponentFilter,
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
    GraphExpandValidWide,
    GraphExpandSupersededWide,
    GraphComponentFilter,
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
            Self::GraphExpandValidWide => "graph_expand_valid_wide",
            Self::GraphExpandSupersededWide => "graph_expand_superseded_wide",
            Self::GraphComponentFilter => "graph_component_filter",
            Self::GraphExpandPagerank => "graph_expand_pagerank",
            Self::ExactGraphOracle => "exact_graph_oracle",
        }
    }
}

pub(crate) fn bench_graph_augmented_vector_retrieval(c: &mut Criterion) {
    bench_retrieval_strategies(c);
    component_pressure::bench(c);
    topology_pressure::bench(c);
    community_pressure::bench(c);
    query_filter_pressure::bench(c);
    session_pressure::bench(c);
}

fn bench_retrieval_strategies(c: &mut Criterion) {
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
    scope_edge: IStr,
    session_edge: IStr,
    valid_edge: IStr,
    superseded_by_edge: IStr,
    contradicts_edge: IStr,
    queries: Vec<Query>,
    metadata: HashMap<NodeId, NodeMeta>,
    graph_current_nodes: HashSet<NodeId>,
    pagerank: HashMap<NodeId, f64>,
    component_candidates: HashMap<u64, Vec<NodeId>>,
    component_order: Vec<u64>,
    component_offsets: HashMap<u64, usize>,
    louvain_by_node: HashMap<NodeId, u64>,
    louvain_candidates: HashMap<u64, Vec<NodeId>>,
    label_by_node: HashMap<NodeId, u64>,
    label_candidates: HashMap<u64, Vec<NodeId>>,
    topic_candidates: Vec<Vec<NodeId>>,
}

impl MemoryRetrievalFixture {
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
        self.selected_quality(query, selected)
    }

    fn selected_quality(&self, query: &Query, selected: Vec<NodeId>) -> RetrievalQuality {
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
            RetrievalStrategy::GraphExpandValidWide => {
                let hits = self.ann_hits(query, WIDE_SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand(query, hits, true),
                    true,
                    false,
                    true,
                )
            }
            RetrievalStrategy::GraphExpandSupersededWide => {
                let hits = self.ann_hits(query, WIDE_SEED_K);
                self.select_from_candidates(
                    query,
                    self.expand_with_supersession(query, hits),
                    true,
                    false,
                    true,
                )
            }
            RetrievalStrategy::GraphComponentFilter => self
                .component_candidates
                .get(&query.component)
                .map(|candidates| {
                    let hits = self.score_candidate_ids(query, candidates.iter().copied());
                    self.select_from_candidates(query, hits, true, false, true)
                })
                .unwrap_or_default(),
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

struct Query {
    topic: usize,
    anchor: NodeId,
    component: u64,
    vector: VectorValue,
}

struct NodeMeta {
    topic: usize,
    fact: usize,
    current: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TopologyNoise {
    Clean,
    CrossTopicSupportRing,
    SparseSupport,
    NoisySparseSupport,
    MultiHopSupport,
    NoisyMultiHopSupport,
    NoisySparseMultiHopSupport,
    ContradictedCurrentDuplicates,
}

#[derive(Clone, Copy, Debug, Default)]
struct RetrievalQuality {
    coverage: usize,
    current_coverage: usize,
    precision: usize,
}
