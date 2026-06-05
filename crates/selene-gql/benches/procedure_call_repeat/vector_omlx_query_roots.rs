use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    hint::black_box,
    num::NonZeroUsize,
    sync::Arc,
};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BindingTable, BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::SharedGraph;
use selene_testing::local_omlx::{CorpusInput, CorpusProfile, OmlxClient, Topic, topic_label};

const QUERY_ROOT_SOURCE: &str = "MATCH (anchor:OmlxQueryAnchor)-[:OmlxDependsOn]->(root:OmlxEmbeddingDoc) WHERE anchor.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_expanded_candidates('embedding', $query, roots, 'OmlxSupports', 4, 'outgoing', 'squared_euclidean') YIELD node_id, distance RETURN node_id, distance";
const ENABLE_ENV: &str = "SELENE_OMLX_EMBEDDING_BENCH";
const API_KEY_ENVS: &[&str] = &["SELENE_OMLX_API_KEY", "OMLX_KEY"];
const BASE_URL_ENV: &str = "SELENE_OMLX_BASE_URL";
const MODELS_ENV: &str = "SELENE_OMLX_EMBEDDING_MODELS";
const CORPUS_ENV: &str = "SELENE_OMLX_CORPUS";
const BATCH_SIZE_ENV: &str = "SELENE_OMLX_EMBEDDING_BATCH_SIZE";
const GRAPH_HINT_DOCS_PER_TOPIC_ENV: &str = "SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC";
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:7700/v1";
const DEFAULT_MODELS: &[&str] = &[
    "Qwen3-Embedding-0.6B-4bit-DWQ",
    "Qwen3-Embedding-4B-4bit-DWQ",
];
const DEFAULT_EMBEDDING_BATCH_SIZE: usize = 64;
const TOP_K: usize = 4;

pub(super) fn bench_vector_omlx_query_roots_procedure(c: &mut Criterion) {
    let Some(config) = OmlxBenchConfig::from_env() else {
        return;
    };
    let client = OmlxClient::new(config.base_url, config.api_key, config.batch_size);
    let inputs = config.corpus.inputs();
    let registry = BuiltinProcedureRegistry::new();
    let mut group = c.benchmark_group("procedure_vector_omlx_query_roots");
    for model in config.models {
        let model_id = model_id(&model);
        let vectors = client
            .embed(&model, &inputs)
            .expect("local oMLX embedding request succeeds");
        let fixture = OmlxGqlQueryRootFixture::build(
            &model,
            &inputs,
            vectors,
            config.graph_hint_docs_per_topic,
        );
        let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        fixture.warm_query_root_cache(&registry, Arc::clone(&cache));
        let precision = fixture.gql_precision_basis_points(&registry, Some(Arc::clone(&cache)));
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_expansion",
                format!(
                    "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    TOP_K,
                    fixture.first_query_root_count(),
                    fixture.first_query_expanded_count(),
                    fixture.dimension,
                    precision,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_queries(&registry, Some(Arc::clone(&cache))));
                });
            },
        );
    }
    group.finish();
}

struct OmlxGqlQueryRootFixture {
    graph: SharedGraph,
    dimension: usize,
    documents: Vec<DocumentMeta>,
    topics_by_node: HashMap<NodeId, Topic>,
    queries: Vec<QueryVector>,
}

impl OmlxGqlQueryRootFixture {
    fn build(
        model: &str,
        inputs: &[CorpusInput],
        vectors: Vec<VectorValue>,
        graph_hint_docs_per_topic: Option<usize>,
    ) -> Self {
        assert_eq!(
            vectors.len(),
            inputs.len(),
            "oMLX returned one vector per corpus input"
        );
        let dimension = vectors
            .first()
            .map(VectorValue::dimension)
            .expect("corpus has at least one vector");
        assert!(
            vectors.iter().all(|vector| vector.dimension() == dimension),
            "oMLX returned consistent vector dimensions"
        );
        let graph = SharedGraph::new(graph_id_for_model(model));
        let doc_label = istr("OmlxEmbeddingDoc");
        let query_label = istr("OmlxQueryAnchor");
        let dependency_edge = istr("OmlxDependsOn");
        let support_edge = istr("OmlxSupports");
        let embedding_key = istr("embedding");
        let query_index_key = istr("query_index");
        let mut documents = Vec::new();
        let mut query_anchors = Vec::new();
        let mut graph_hint_counts = HashMap::<Topic, usize>::new();
        {
            let mut txn = graph.begin_write();
            {
                let mut mutator = txn.mutator();
                for (input, vector) in inputs.iter().zip(vectors.iter()) {
                    if !input.is_document {
                        continue;
                    }
                    let props = PropertyMap::from_pairs([(
                        embedding_key.clone(),
                        Value::Vector(vector.clone()),
                    )])
                    .expect("oMLX GQL bench document properties fit");
                    let graph_hint = admits_graph_hint(
                        &mut graph_hint_counts,
                        input.topic,
                        graph_hint_docs_per_topic,
                    );
                    let mut labels = LabelSet::single(doc_label.clone());
                    if graph_hint {
                        labels.insert(topic_label(input.topic));
                    }
                    let node = mutator
                        .create_node(labels, props)
                        .expect("oMLX GQL bench document node inserts");
                    documents.push(DocumentMeta {
                        node,
                        topic: input.topic,
                        graph_hint,
                    });
                }
                for source in documents.iter().filter(|document| document.graph_hint) {
                    for target in documents
                        .iter()
                        .filter(|document| document.topic == source.topic)
                    {
                        if target.node == source.node {
                            continue;
                        }
                        mutator
                            .create_edge(
                                support_edge.clone(),
                                source.node,
                                target.node,
                                PropertyMap::new(),
                            )
                            .expect("oMLX GQL bench support edge inserts");
                    }
                }
                for (query_index, input) in
                    inputs.iter().filter(|input| !input.is_document).enumerate()
                {
                    let props = PropertyMap::from_pairs([(
                        query_index_key.clone(),
                        Value::Int(query_index as i64),
                    )])
                    .expect("oMLX GQL bench query properties fit");
                    let anchor = mutator
                        .create_node(LabelSet::from_iter([query_label.clone()]), props)
                        .expect("oMLX GQL bench query anchor inserts");
                    for document in documents
                        .iter()
                        .filter(|document| document.topic == input.topic && document.graph_hint)
                    {
                        mutator
                            .create_edge(
                                dependency_edge.clone(),
                                anchor,
                                document.node,
                                PropertyMap::new(),
                            )
                            .expect("oMLX GQL bench query dependency edge inserts");
                    }
                    query_anchors.push(input.topic);
                }
            }
            txn.commit().expect("oMLX GQL bench graph commits");
        }
        let mut query_anchors = query_anchors.into_iter();
        let queries = inputs
            .iter()
            .zip(vectors)
            .filter_map(|(input, vector)| {
                (!input.is_document).then(|| {
                    let anchor_topic = query_anchors
                        .next()
                        .expect("oMLX GQL bench query anchor count matches queries");
                    debug_assert_eq!(anchor_topic, input.topic);
                    QueryVector {
                        topic: input.topic,
                        vector,
                    }
                })
            })
            .collect::<Vec<_>>();
        debug_assert!(
            query_anchors.next().is_none(),
            "oMLX GQL bench consumed every query anchor"
        );
        let topics_by_node = documents
            .iter()
            .map(|document| (document.node, document.topic))
            .collect();
        Self {
            graph,
            dimension,
            documents,
            topics_by_node,
            queries,
        }
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn first_query_root_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            self.documents
                .iter()
                .filter(|document| document.topic == query.topic && document.graph_hint)
                .count()
        })
    }

    fn first_query_expanded_count(&self) -> usize {
        self.queries.first().map_or(0, |query| {
            if self.first_query_root_count() == 0 {
                0
            } else {
                self.documents
                    .iter()
                    .filter(|document| document.topic == query.topic)
                    .count()
            }
        })
    }

    fn warm_query_root_cache(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Arc<CallPlanCache>,
    ) {
        self.execute_query(0, registry, Some(cache));
    }

    fn execute_all_queries(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        (0..self.queries.len())
            .map(|query_index| {
                self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone))
                    .row_count()
            })
            .sum()
    }

    fn gql_precision_basis_points(
        &self,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> usize {
        let total_precision = self
            .queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                let table =
                    self.execute_query(query_index, registry, cache.as_ref().map(Arc::clone));
                self.precision(query.topic, &table)
            })
            .sum();
        precision_basis_points(total_precision, self.query_count() * TOP_K)
    }

    fn execute_query(
        &self,
        query_index: usize,
        registry: &BuiltinProcedureRegistry,
        cache: Option<Arc<CallPlanCache>>,
    ) -> BindingTable {
        let mut session = Session::new(&self.graph);
        if let Some(cache) = cache {
            session = session.with_call_plan_cache(cache);
        }
        let query = self
            .queries
            .get(query_index)
            .expect("oMLX GQL bench query index is valid");
        session.bind_parameter(istr("query"), Value::Vector(query.vector.clone()));
        session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
        match session
            .execute_source(QUERY_ROOT_SOURCE, registry)
            .expect("oMLX GQL query-root vector procedure executes")
        {
            StatementOutput::Rows(table) => table,
            other => panic!("unexpected output: {other:?}"),
        }
    }

    fn precision(&self, topic: Topic, table: &BindingTable) -> usize {
        let node_column = table
            .column_index(istr("node_id"))
            .expect("node_id column exists");
        table
            .iter()
            .filter_map(|row| match row.get(node_column) {
                Some(Value::NodeRef(node)) => Some(*node),
                _ => None,
            })
            .filter(|node| {
                self.topics_by_node
                    .get(node)
                    .is_some_and(|hit_topic| *hit_topic == topic)
            })
            .count()
    }
}

#[derive(Clone, Copy)]
struct DocumentMeta {
    node: NodeId,
    topic: Topic,
    graph_hint: bool,
}

struct QueryVector {
    topic: Topic,
    vector: VectorValue,
}

struct OmlxBenchConfig {
    base_url: String,
    api_key: String,
    models: Vec<String>,
    corpus: CorpusProfile,
    batch_size: usize,
    graph_hint_docs_per_topic: Option<usize>,
}

impl OmlxBenchConfig {
    fn from_env() -> Option<Self> {
        if std::env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
            return None;
        }
        let api_key = API_KEY_ENVS
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .expect("SELENE_OMLX_API_KEY or OMLX_KEY must be set for local oMLX benches");
        let base_url = std::env::var(BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        let models = std::env::var(MODELS_ENV)
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|models| !models.is_empty())
            .unwrap_or_else(|| {
                DEFAULT_MODELS
                    .iter()
                    .map(|model| (*model).to_owned())
                    .collect()
            });
        Some(Self {
            base_url,
            api_key,
            models,
            corpus: CorpusProfile::from_env(CORPUS_ENV),
            batch_size: embedding_batch_size(),
            graph_hint_docs_per_topic: graph_hint_docs_per_topic(),
        })
    }
}

fn embedding_batch_size() -> usize {
    std::env::var(BATCH_SIZE_ENV)
        .ok()
        .map(|raw| {
            let batch_size = raw
                .parse::<usize>()
                .expect("SELENE_OMLX_EMBEDDING_BATCH_SIZE must be a positive integer");
            assert!(
                batch_size > 0,
                "SELENE_OMLX_EMBEDDING_BATCH_SIZE must be greater than zero"
            );
            batch_size
        })
        .unwrap_or(DEFAULT_EMBEDDING_BATCH_SIZE)
}

fn graph_hint_docs_per_topic() -> Option<usize> {
    std::env::var(GRAPH_HINT_DOCS_PER_TOPIC_ENV)
        .ok()
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            raw.parse::<usize>()
                .expect("SELENE_OMLX_GRAPH_HINT_DOCS_PER_TOPIC must be a non-negative integer")
        })
}

fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(98_000 + hasher.finish() % 1_000)
}

fn admits_graph_hint(
    graph_hint_counts: &mut HashMap<Topic, usize>,
    topic: Topic,
    graph_hint_docs_per_topic: Option<usize>,
) -> bool {
    let Some(limit) = graph_hint_docs_per_topic else {
        return true;
    };
    let count = graph_hint_counts.entry(topic).or_insert(0);
    if *count >= limit {
        return false;
    }
    *count += 1;
    true
}

fn model_id(model: &str) -> String {
    model
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn corpus_label(corpus: CorpusProfile) -> &'static str {
    match corpus {
        CorpusProfile::Tiny => "tiny",
        CorpusProfile::AgentMemory => "agent_memory",
        CorpusProfile::AmbiguousMemory => "ambiguous_memory",
        CorpusProfile::ScaledAmbiguousMemory => "scaled_ambiguous_memory",
    }
}

fn precision_basis_points(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(10_000)
        .checked_div(denominator)
        .unwrap_or(10_000)
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}
