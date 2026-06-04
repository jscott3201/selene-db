//! Support fixture for graph-augmented vector retrieval benchmarks.

use std::collections::{HashMap, HashSet};
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{
    CancellationChecker, HnswIndexConfig, IStr, LabelSet, NodeId, PropertyMap, Value, VectorMetric,
    VectorValue,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexConfig, VectorIndexKind,
    VectorNodeSearchHit,
};

use crate::common::scale_label;

mod component_pressure;
mod support;
mod topology_pressure;

use support::{
    DIMENSION, FACTS_PER_TOPIC, ORACLE_SEED_K, PAGERANK_WEIGHT, RESULT_K, SEARCH_WIDTH, SEED_K,
    WIDE_SEED_K, basis_points, component_candidates, current_replacement, duplicates_per_fact,
    graph_id_for_scale, istr, memory_vector, pagerank_scores, topic_count, vector_scales,
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
    valid_edge: IStr,
    superseded_by_edge: IStr,
    queries: Vec<Query>,
    metadata: HashMap<NodeId, NodeMeta>,
    pagerank: HashMap<NodeId, f64>,
    component_candidates: HashMap<u64, Vec<NodeId>>,
    component_order: Vec<u64>,
    component_offsets: HashMap<u64, usize>,
    topic_candidates: Vec<Vec<NodeId>>,
}

impl MemoryRetrievalFixture {
    fn build(requested_scale: usize) -> Self {
        Self::build_with_topology(requested_scale, TopologyNoise::Clean)
    }

    fn build_with_topology(requested_scale: usize, topology: TopologyNoise) -> Self {
        let label = istr("Memory");
        let embedding_key = istr("embedding");
        let support_edge = istr("SUPPORTS");
        let valid_edge = istr("VALID_AT");
        let superseded_by_edge = istr("SUPERSEDED_BY");
        let topic_count = topic_count(requested_scale);
        let duplicates = duplicates_per_fact(requested_scale, topic_count);
        let shared = SharedGraph::new(graph_id_for_scale(requested_scale));
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
                if topology == TopologyNoise::CrossTopicSupportRing {
                    for topic in 0..topic_count {
                        let next_topic = (topic + 1) % topic_count;
                        for duplicate in 0..duplicates {
                            let summary = topic_nodes[topic][0][duplicate];
                            let target_fact = 1 + duplicate % (FACTS_PER_TOPIC - 1);
                            let target_duplicate = (duplicate + 1) % duplicates;
                            let target = topic_nodes[next_topic][target_fact][target_duplicate];
                            mutator
                                .create_edge(
                                    support_edge.clone(),
                                    summary,
                                    target,
                                    PropertyMap::new(),
                                )
                                .expect("bench noisy support edge inserts");
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
        let (component_by_node, component_candidates) =
            component_candidates(&graph, &label, &support_edge, &superseded_by_edge);
        let mut component_order: Vec<_> = component_candidates.keys().copied().collect();
        component_order.sort_unstable();
        let component_offsets = component_order
            .iter()
            .copied()
            .enumerate()
            .map(|(offset, component)| (component, offset))
            .collect();
        let mut topic_candidates = vec![Vec::new(); topic_count];
        for (&node, meta) in &metadata {
            topic_candidates[meta.topic].push(node);
        }
        for candidates in &mut topic_candidates {
            candidates.sort_unstable();
        }
        let queries = (0..topic_count)
            .map(|topic| Query {
                topic,
                component: component_by_node[&topic_nodes[topic][0][duplicates / 2]],
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
            component_candidates,
            component_order,
            component_offsets,
            topic_candidates,
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
}

#[derive(Clone, Copy, Debug, Default)]
struct RetrievalQuality {
    coverage: usize,
    current_coverage: usize,
    precision: usize,
}
