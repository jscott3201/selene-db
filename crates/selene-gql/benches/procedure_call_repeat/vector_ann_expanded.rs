use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value, VectorValue};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
    VectorIndexKind,
};

const ANN_EXPANDED_SOURCE: &str = "CALL selene.vector_search_expanded_candidates_ann('VectorSummary', 'embedding', $query, 2, 'SUPPORTS', 10, 'outgoing', 'squared_euclidean', 64) YIELD node_id, distance";
const ANN_EXPANDED_BATCH_SOURCE: &str = "CALL selene.vector_search_expanded_candidates_ann_batch('VectorSummary', 'embedding', $queries, 2, 'SUPPORTS', 10, 'outgoing', 'squared_euclidean', 64) YIELD query_index, node_id, distance";
const ANN_STATE_EXPANDED_SOURCE: &str = "CALL selene.vector_search_candidate_state_expanded_ann('VectorSummary', 'embedding', $query, 'active_facts', 2, 'SUPPORTS', 10, 'intersection', 'outgoing', 'squared_euclidean', 64) YIELD node_id, distance";
const VECTOR_SCALE: usize = 1_000;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_BATCH_QUERIES: usize = 8;
const VECTOR_EXPANDED_CANDIDATES: usize = 64;
const VECTOR_EXPANDED_ROOTS: usize = 2;
const VECTOR_ACTIVE_FACTS: usize = VECTOR_BATCH_QUERIES * VECTOR_EXPANDED_CANDIDATES;

pub(super) fn bench_vector_ann_expanded_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = vector_ann_expanded_graph(VECTOR_SCALE, VECTOR_DIMENSION);
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let state_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    warm_ann_expanded_cache(&graph, &registry, Arc::clone(&cache));
    warm_ann_expanded_batch_cache(&graph, &registry, Arc::clone(&batch_cache));
    warm_ann_state_expanded_cache(&graph, &registry, Arc::clone(&state_cache));

    let mut group = c.benchmark_group("procedure_vector_ann_expanded");
    group.throughput(Throughput::Elements(VECTOR_EXPANDED_CANDIDATES as u64));
    group.bench_function("shared_cache_ann_expanded_2root64_dim128_k10_1000", |b| {
        b.iter(|| {
            std::hint::black_box(execute_vector_ann_expanded_search(
                &graph,
                &registry,
                Some(Arc::clone(&cache)),
                0,
            ));
        });
    });
    group.bench_function(
        "shared_cache_ann_expanded_repeated_8x2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_ann_expanded_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_ann_expanded_batch_8x2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_ann_expanded_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&batch_cache)),
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_ann_state_expanded_intersection_2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_ann_state_expanded_search(
                    &graph,
                    &registry,
                    Some(Arc::clone(&state_cache)),
                    0,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_ann_state_expanded_intersection_repeated_8x2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_ann_state_expanded_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&state_cache)),
                ));
            });
        },
    );
    group.finish();
}

fn warm_ann_expanded_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_ann_expanded_inputs_for(&mut session, 0);
    session
        .execute_source(ANN_EXPANDED_SOURCE, registry)
        .expect("warmup ANN-expanded vector search executes");
}

fn warm_ann_expanded_batch_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_ann_expanded_batch_inputs(&mut session);
    session
        .execute_source(ANN_EXPANDED_BATCH_SOURCE, registry)
        .expect("warmup batched ANN-expanded vector search executes");
}

fn warm_ann_state_expanded_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_ann_expanded_inputs_for(&mut session, 0);
    session
        .execute_source(ANN_STATE_EXPANDED_SOURCE, registry)
        .expect("warmup ANN state-expanded vector search executes");
}

fn execute_vector_ann_expanded_search(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_ann_expanded_inputs_for(&mut session, query_index);
    match session
        .execute_source(ANN_EXPANDED_SOURCE, registry)
        .expect("ANN-expanded vector search procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_ann_state_expanded_search(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_ann_expanded_inputs_for(&mut session, query_index);
    match session
        .execute_source(ANN_STATE_EXPANDED_SOURCE, registry)
        .expect("ANN state-expanded vector search procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_ann_expanded_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_ann_expanded_search(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
        );
    }
    rows
}

fn execute_vector_ann_state_expanded_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_ann_state_expanded_search(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
        );
    }
    rows
}

fn execute_vector_ann_expanded_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_ann_expanded_batch_inputs(&mut session);
    match session
        .execute_source(ANN_EXPANDED_BATCH_SOURCE, registry)
        .expect("batched ANN-expanded vector search procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn vector_ann_expanded_graph(scale: usize, dimension: usize) -> SharedGraph {
    let active_fact = db_string("ActiveVectorFact");
    let state_name = db_string("active_facts");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([
            CandidateStateSpec::new(state_name).require_label(active_fact.clone())
        ])
        .expect("bench candidate-state spec is valid"),
    );
    let graph = SharedGraph::builder(GraphId::new(71_006))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("bench graph builds");
    let summary = db_string("VectorSummary");
    let fact = db_string("VectorFact");
    let embedding_key = db_string("embedding");
    let supports = db_string("SUPPORTS");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            let mut roots = Vec::with_capacity(scale);
            let mut facts = Vec::with_capacity(scale);
            for idx in 0..scale {
                let props = PropertyMap::from_pairs([(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, dimension)),
                )])
                .expect("bench root vector properties are valid");
                roots.push(
                    mutator
                        .create_node(LabelSet::single(summary.clone()), props)
                        .expect("bench root node insert succeeds"),
                );
            }
            for idx in 0..scale {
                let labels = if idx < VECTOR_ACTIVE_FACTS {
                    LabelSet::from_iter([fact.clone(), active_fact.clone()])
                } else {
                    LabelSet::single(fact.clone())
                };
                let props = PropertyMap::from_pairs([(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, dimension)),
                )])
                .expect("bench fact vector properties are valid");
                facts.push(
                    mutator
                        .create_node(labels, props)
                        .expect("bench fact node insert succeeds"),
                );
            }
            for query_index in 0..VECTOR_BATCH_QUERIES {
                let range = expanded_candidate_range(query_index);
                for root in range.clone().take(VECTOR_EXPANDED_ROOTS) {
                    for candidate in range.clone() {
                        mutator
                            .create_edge(
                                supports.clone(),
                                roots[root],
                                facts[candidate],
                                PropertyMap::new(),
                            )
                            .expect("bench support edge insert succeeds");
                    }
                }
            }
        }
        txn.commit().expect("bench ANN-expanded fixture commits");
    }
    graph
        .create_vector_index(
            summary,
            embedding_key,
            VectorIndexKind::HnswSquaredEuclidean,
            u32::try_from(dimension).expect("bench dimension fits u32"),
        )
        .expect("bench HNSW vector index builds");
    graph
}

fn bind_ann_expanded_inputs_for(session: &mut Session<'_>, query_index: usize) {
    session.bind_parameter(
        db_string("query"),
        Value::Vector(vector_value(query_seed(query_index), VECTOR_DIMENSION)),
    );
}

fn bind_ann_expanded_batch_inputs(session: &mut Session<'_>) {
    session.bind_parameter(db_string("queries"), vector_query_batch());
}

fn vector_query_batch() -> Value {
    Value::List(
        (0..VECTOR_BATCH_QUERIES)
            .map(|query_index| {
                Value::Vector(vector_value(query_seed(query_index), VECTOR_DIMENSION))
            })
            .collect(),
    )
}

fn query_seed(query_index: usize) -> usize {
    expanded_candidate_range(query_index).start
}

fn expanded_candidate_range(query_index: usize) -> std::ops::Range<usize> {
    let max_start = VECTOR_SCALE.saturating_sub(VECTOR_EXPANDED_CANDIDATES);
    let start = if max_start == 0 {
        0
    } else {
        (query_index * VECTOR_EXPANDED_CANDIDATES) % max_start
    };
    start..start + VECTOR_EXPANDED_CANDIDATES
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect();
    VectorValue::new(components).expect("bench vector is valid")
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}
