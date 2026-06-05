#![allow(missing_docs)]
//! Criterion benches for exact and indexed BM25 text search.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorMetric,
    VectorValue, intern,
};
use selene_graph::{ApproximateVectorSearchOptions, VectorIndexKind};
use selene_graph::{SeleneGraph, SharedGraph, TextIndex, VectorNodeSearchHit};
use selene_testing::BenchProfile;

const TOPICS: [&str; 8] = [
    "gql",
    "vector",
    "memory",
    "planner",
    "storage",
    "agent",
    "schema",
    "retrieval",
];
const STATES: [&str; 4] = ["current", "stale", "draft", "verified"];
const HYBRID_FACTS_PER_TOPIC: usize = 8;
const HYBRID_RESULT_K: usize = 8;
const HYBRID_DIMENSION: usize = 64;
const HYBRID_ANN_SEARCH_WIDTH: usize = 64;

const HYBRID_STRATEGIES: [HybridStrategy; 12] = [
    HybridStrategy::VectorOnly,
    HybridStrategy::VectorBm25Current,
    HybridStrategy::VectorBm25CurrentVector,
    HybridStrategy::AnnOnly,
    HybridStrategy::AnnBm25Current,
    HybridStrategy::AnnBm25CurrentVector,
    HybridStrategy::Bm25TopicCurrent,
    HybridStrategy::Bm25TopicCurrentVector,
    HybridStrategy::GraphTopicBm25Current,
    HybridStrategy::GraphTopicBm25CurrentVector,
    HybridStrategy::GraphTopicBm25CurrentScoped,
    HybridStrategy::GraphTopicBm25CurrentScopedVector,
];

fn bench_exact_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_text_bm25_exact");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = TextFixture::build(scale);
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_with_input(
            BenchmarkId::new("topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph
                        .exact_text_search_nodes(
                            &fixture.label,
                            &fixture.property,
                            &fixture.query,
                            10,
                        )
                        .expect("BM25 search succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

fn bench_indexed_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_text_bm25_indexed");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = TextFixture::build(scale);
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_with_input(
            BenchmarkId::new("prebuilt_topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture.index.search(&fixture.query, 10);
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("registered_topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture.registered_index.search(&fixture.query, 10);
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("transient_build_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph
                        .indexed_text_search_nodes(
                            &fixture.label,
                            &fixture.property,
                            &fixture.query,
                            10,
                        )
                        .expect("indexed BM25 search succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

fn bench_hybrid_bm25_vector(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_text_bm25_vector_hybrid");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = HybridFixture::build(scale);
        group.throughput(Throughput::Elements(
            (fixture.query_count() * HYBRID_RESULT_K) as u64,
        ));
        for strategy in HYBRID_STRATEGIES {
            let quality = fixture.quality(strategy);
            group.bench_function(
                BenchmarkId::new(
                    strategy.name(),
                    format!(
                        "n{}_q{}_c{}_precbp{}_curbp{}",
                        fixture.scale,
                        fixture.query_count(),
                        quality.selected,
                        basis_points(quality.precision, fixture.query_count() * HYBRID_RESULT_K),
                        basis_points(
                            quality.current_precision,
                            fixture.query_count() * HYBRID_RESULT_K
                        ),
                    ),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(fixture.total_precision(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

struct TextFixture {
    graph: Arc<SeleneGraph>,
    label: IStr,
    property: IStr,
    query: String,
    index: TextIndex,
    registered_index: Arc<TextIndex>,
}

impl TextFixture {
    fn build(scale: usize) -> Self {
        let shared = SharedGraph::new(GraphId::new(431_201));
        let label = intern("BenchTextDoc").expect("bench label interns");
        let property = intern("body").expect("bench property interns");
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for row in 0..scale {
                let topic = TOPICS[row % TOPICS.len()];
                let neighbor = TOPICS[(row + 3) % TOPICS.len()];
                let state = STATES[row % STATES.len()];
                let text = format!(
                    "{topic} {state} agent memory fact supports {neighbor} retrieval evidence"
                );
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(
                            &property,
                            Value::String(intern(&text).expect("bench text interns")),
                        ),
                    )
                    .expect("bench node inserts");
            }
            txn.commit().expect("bench fixture commits");
        }
        shared
            .create_text_index(label.clone(), property.clone())
            .expect("bench text index registers");
        let graph = shared.read();
        let index = graph
            .build_text_index(&label, &property)
            .expect("bench text index builds");
        let registered_index = graph
            .text_index_for(&label, &property)
            .expect("registered bench text index exists");
        Self {
            graph,
            label,
            property,
            query: "gql current retrieval evidence".to_owned(),
            index,
            registered_index,
        }
    }
}

#[derive(Clone, Copy)]
enum HybridStrategy {
    VectorOnly,
    VectorBm25Current,
    VectorBm25CurrentVector,
    AnnOnly,
    AnnBm25Current,
    AnnBm25CurrentVector,
    Bm25TopicCurrent,
    Bm25TopicCurrentVector,
    GraphTopicBm25Current,
    GraphTopicBm25CurrentVector,
    GraphTopicBm25CurrentScoped,
    GraphTopicBm25CurrentScopedVector,
}

impl HybridStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::VectorOnly => "vector_only",
            Self::VectorBm25Current => "vector_bm25_current_filter",
            Self::VectorBm25CurrentVector => "vector_bm25_current_vector_rerank",
            Self::AnnOnly => "ann_only",
            Self::AnnBm25Current => "ann_bm25_current_filter",
            Self::AnnBm25CurrentVector => "ann_bm25_current_vector_rerank",
            Self::Bm25TopicCurrent => "bm25_topic_current",
            Self::Bm25TopicCurrentVector => "bm25_topic_current_vector_rerank",
            Self::GraphTopicBm25Current => "graph_topic_bm25_current",
            Self::GraphTopicBm25CurrentVector => "graph_topic_bm25_current_vector_rerank",
            Self::GraphTopicBm25CurrentScoped => "graph_topic_bm25_current_scoped",
            Self::GraphTopicBm25CurrentScopedVector => {
                "graph_topic_bm25_current_scoped_vector_rerank"
            }
        }
    }
}

struct HybridFixture {
    graph: Arc<SeleneGraph>,
    scale: usize,
    label: IStr,
    embedding_key: IStr,
    topic_edge: IStr,
    text_index: Arc<TextIndex>,
    queries: Vec<HybridQuery>,
    metadata: HashMap<NodeId, HybridMeta>,
    bm25_width: usize,
    vector_candidate_width: usize,
    topic_current_width: usize,
}

impl HybridFixture {
    fn build(requested_scale: usize) -> Self {
        let label = intern("HybridMemoryDoc").expect("hybrid label interns");
        let topic_label = intern("HybridMemoryTopic").expect("hybrid topic label interns");
        let body_key = intern("body").expect("hybrid body key interns");
        let embedding_key = intern("embedding").expect("hybrid embedding key interns");
        let topic_edge = intern("IN_HYBRID_TOPIC").expect("hybrid topic edge interns");
        let duplicates = (requested_scale / (TOPICS.len() * HYBRID_FACTS_PER_TOPIC)).clamp(2, 256);
        let scale = TOPICS.len() * HYBRID_FACTS_PER_TOPIC * duplicates;
        let shared = SharedGraph::new(GraphId::new(434_000 + scale as u64));
        let mut topic_nodes = Vec::with_capacity(TOPICS.len());
        let mut metadata = HashMap::with_capacity(scale);
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for topic in TOPICS {
                let topic_node = mutator
                    .create_node(
                        LabelSet::single(topic_label.clone()),
                        props(
                            &body_key,
                            Value::String(intern(topic).expect("topic interns")),
                        ),
                    )
                    .expect("hybrid topic node inserts");
                topic_nodes.push(topic_node);
            }
            for (topic_index, topic) in TOPICS.iter().enumerate() {
                for fact in 0..HYBRID_FACTS_PER_TOPIC {
                    for duplicate in 0..duplicates {
                        let current = duplicate.is_multiple_of(2);
                        let state = if current { "current" } else { "stale" };
                        let body =
                            format!("{topic} {state} evidence memory retrieval fact{fact} hybrid");
                        let vector = hybrid_vector(topic_index, fact, duplicate);
                        let node = mutator
                            .create_node(
                                LabelSet::single(label.clone()),
                                hybrid_props(
                                    &body_key,
                                    Value::String(intern(&body).expect("hybrid body interns")),
                                    &embedding_key,
                                    Value::Vector(vector),
                                ),
                            )
                            .expect("hybrid memory node inserts");
                        mutator
                            .create_edge(
                                topic_edge.clone(),
                                node,
                                topic_nodes[topic_index],
                                PropertyMap::new(),
                            )
                            .expect("hybrid topic edge inserts");
                        metadata.insert(
                            node,
                            HybridMeta {
                                topic: topic_index,
                                fact,
                                current,
                            },
                        );
                    }
                }
            }
            txn.commit().expect("hybrid fixture commits");
        }
        shared
            .create_text_index(label.clone(), body_key.clone())
            .expect("hybrid text index registers");
        shared
            .create_vector_index(
                label.clone(),
                embedding_key.clone(),
                VectorIndexKind::HnswCosine,
                u32::try_from(HYBRID_DIMENSION).expect("hybrid dimension fits u32"),
            )
            .expect("hybrid ANN index registers");
        let graph = shared.read();
        let text_index = graph
            .text_index_for(&label, &body_key)
            .expect("hybrid text index exists");
        let queries = TOPICS
            .iter()
            .enumerate()
            .map(|(topic, name)| {
                let fact = 1 + (topic * 3 % (HYBRID_FACTS_PER_TOPIC - 1));
                HybridQuery {
                    topic,
                    fact,
                    topic_node: topic_nodes[topic],
                    topic_current_text: format!("{name} current evidence"),
                    current_text: "current evidence".to_owned(),
                    current_filter_text: "current".to_owned(),
                    vector: hybrid_vector(topic, fact, 0),
                }
            })
            .collect();
        Self {
            graph,
            scale,
            label,
            embedding_key,
            topic_edge,
            text_index,
            queries,
            metadata,
            bm25_width: scale,
            vector_candidate_width: HYBRID_FACTS_PER_TOPIC * duplicates.div_ceil(2),
            topic_current_width: HYBRID_FACTS_PER_TOPIC * duplicates.div_ceil(2),
        }
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn total_precision(&self, strategy: HybridStrategy) -> usize {
        self.quality(strategy).precision
    }

    fn quality(&self, strategy: HybridStrategy) -> HybridQuality {
        self.queries
            .iter()
            .map(|query| self.query_quality(query, strategy))
            .fold(HybridQuality::default(), |mut total, next| {
                total.selected += next.selected;
                total.precision += next.precision;
                total.current_precision += next.current_precision;
                total
            })
    }

    fn query_quality(&self, query: &HybridQuery, strategy: HybridStrategy) -> HybridQuality {
        let selected = self.select(query, strategy);
        let mut quality = HybridQuality {
            selected: selected.len(),
            ..HybridQuality::default()
        };
        for node in selected {
            if let Some(meta) = self.metadata.get(&node)
                && meta.topic == query.topic
                && meta.fact == query.fact
            {
                quality.precision += 1;
                if meta.current {
                    quality.current_precision += 1;
                }
            }
        }
        quality
    }

    fn select(&self, query: &HybridQuery, strategy: HybridStrategy) -> Vec<NodeId> {
        match strategy {
            HybridStrategy::VectorOnly => self.vector_hits(query, HYBRID_RESULT_K),
            HybridStrategy::VectorBm25Current => self
                .vector_bm25_current_candidates(query)
                .into_iter()
                .take(HYBRID_RESULT_K)
                .collect(),
            HybridStrategy::VectorBm25CurrentVector => {
                let candidates = self.vector_bm25_current_candidates(query);
                self.vector_rerank(query, &candidates, HYBRID_RESULT_K)
            }
            HybridStrategy::AnnOnly => self.ann_hits(query, HYBRID_RESULT_K),
            HybridStrategy::AnnBm25Current => self
                .ann_bm25_current_candidates(query)
                .into_iter()
                .take(HYBRID_RESULT_K)
                .collect(),
            HybridStrategy::AnnBm25CurrentVector => {
                let candidates = self.ann_bm25_current_candidates(query);
                self.vector_rerank(query, &candidates, HYBRID_RESULT_K)
            }
            HybridStrategy::Bm25TopicCurrent => {
                self.bm25_hits(&query.topic_current_text, HYBRID_RESULT_K)
            }
            HybridStrategy::Bm25TopicCurrentVector => {
                let candidates =
                    self.bm25_hits(&query.topic_current_text, self.topic_current_width);
                self.vector_rerank(query, &candidates, HYBRID_RESULT_K)
            }
            HybridStrategy::GraphTopicBm25Current => self
                .graph_topic_bm25_candidates(query)
                .into_iter()
                .take(HYBRID_RESULT_K)
                .collect(),
            HybridStrategy::GraphTopicBm25CurrentVector => {
                let candidates = self.graph_topic_bm25_candidates(query);
                self.vector_rerank(query, &candidates, HYBRID_RESULT_K)
            }
            HybridStrategy::GraphTopicBm25CurrentScoped => self
                .graph_topic_bm25_scoped_candidates(query)
                .into_iter()
                .take(HYBRID_RESULT_K)
                .collect(),
            HybridStrategy::GraphTopicBm25CurrentScopedVector => {
                let candidates = self.graph_topic_bm25_scoped_candidates(query);
                self.vector_rerank(query, &candidates, HYBRID_RESULT_K)
            }
        }
    }

    fn vector_hits(&self, query: &HybridQuery, k: usize) -> Vec<NodeId> {
        self.graph
            .exact_vector_search_nodes(
                &self.label,
                &self.embedding_key,
                &query.vector,
                VectorMetric::Cosine,
                k,
            )
            .expect("hybrid vector search succeeds")
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    fn ann_hits(&self, query: &HybridQuery, k: usize) -> Vec<NodeId> {
        self.graph
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &query.vector,
                ApproximateVectorSearchOptions::new(
                    VectorMetric::Cosine,
                    k,
                    HYBRID_ANN_SEARCH_WIDTH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("hybrid ANN vector search succeeds")
            .into_iter()
            .map(vector_hit_node)
            .collect()
    }

    fn bm25_hits(&self, query: &str, k: usize) -> Vec<NodeId> {
        self.text_index
            .search(query, k)
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    fn ann_bm25_current_candidates(&self, query: &HybridQuery) -> Vec<NodeId> {
        let ann_nodes = self.ann_hits(query, self.vector_candidate_width);
        self.text_index
            .search_candidates(
                &query.current_filter_text,
                &ann_nodes,
                self.vector_candidate_width,
            )
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    fn vector_bm25_current_candidates(&self, query: &HybridQuery) -> Vec<NodeId> {
        let vector_nodes = self.vector_hits(query, self.vector_candidate_width);
        self.text_index
            .search_candidates(
                &query.current_filter_text,
                &vector_nodes,
                self.vector_candidate_width,
            )
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    fn graph_topic_bm25_candidates(&self, query: &HybridQuery) -> Vec<NodeId> {
        let topic_nodes = HashSet::<_>::from_iter(self.topic_candidates(query));
        self.bm25_hits(&query.current_text, self.bm25_width)
            .into_iter()
            .filter(|node| topic_nodes.contains(node))
            .take(self.topic_current_width)
            .collect()
    }

    fn graph_topic_bm25_scoped_candidates(&self, query: &HybridQuery) -> Vec<NodeId> {
        let topic_nodes = self.topic_candidates(query);
        self.text_index
            .search_candidates(&query.current_text, &topic_nodes, self.topic_current_width)
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    fn topic_candidates(&self, query: &HybridQuery) -> Vec<NodeId> {
        let mut candidates: Vec<NodeId> = self
            .graph
            .incoming_edges(query.topic_node)
            .map(|entry| {
                entry
                    .iter_label(&self.topic_edge)
                    .map(|edge| edge.neighbor)
                    .collect()
            })
            .unwrap_or_default();
        candidates.sort_unstable();
        candidates
    }

    fn vector_rerank(&self, query: &HybridQuery, candidates: &[NodeId], k: usize) -> Vec<NodeId> {
        self.graph
            .score_vector_nodes_checked(
                &self.embedding_key,
                &query.vector,
                candidates,
                VectorMetric::Cosine,
                k,
                CancellationChecker::disabled(),
            )
            .expect("hybrid vector rerank succeeds")
            .into_iter()
            .map(vector_hit_node)
            .collect()
    }
}

struct HybridQuery {
    topic: usize,
    fact: usize,
    topic_node: NodeId,
    topic_current_text: String,
    current_text: String,
    current_filter_text: String,
    vector: VectorValue,
}

struct HybridMeta {
    topic: usize,
    fact: usize,
    current: bool,
}

#[derive(Default)]
struct HybridQuality {
    selected: usize,
    precision: usize,
    current_precision: usize,
}

fn hybrid_vector(topic: usize, fact: usize, duplicate: usize) -> VectorValue {
    let primary = topic % HYBRID_DIMENSION;
    let secondary = topic.wrapping_mul(7).wrapping_add(5) % HYBRID_DIMENSION;
    let fact_dim = topic.wrapping_mul(13).wrapping_add(fact * 11 + 3) % HYBRID_DIMENSION;
    let components: Vec<f32> = (0..HYBRID_DIMENSION)
        .map(|dim| {
            let topic_signal = if dim == primary { 1.0 } else { 0.0 };
            let secondary_signal = if dim == secondary { 0.25 } else { 0.0 };
            let fact_signal = if dim == fact_dim { 0.5 } else { 0.0 };
            let duplicate_noise =
                ((duplicate * (dim + 17) + topic * 19 + fact * 23) % 31) as f32 / 100_000.0;
            topic_signal + secondary_signal + fact_signal + duplicate_noise + 0.001
        })
        .collect();
    VectorValue::new(components).expect("hybrid vector is valid")
}

fn vector_hit_node(hit: VectorNodeSearchHit) -> NodeId {
    hit.node_id
}

fn hybrid_props(
    text_key: &IStr,
    text_value: Value,
    vector_key: &IStr,
    vector_value: Value,
) -> PropertyMap {
    PropertyMap::from_pairs([
        (text_key.clone(), text_value),
        (vector_key.clone(), vector_value),
    ])
    .expect("hybrid property map is valid")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("bench property map is valid")
}

fn basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

criterion_group! {
    name = text_search;
    config = common::criterion_config();
    targets = bench_exact_bm25, bench_indexed_bm25, bench_hybrid_bm25_vector
}
criterion_main!(text_search);
