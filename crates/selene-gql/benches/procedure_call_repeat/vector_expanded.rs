use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::SharedGraph;

const EXPANDED_SOURCE: &str = "CALL selene.vector_score_expanded_candidates('embedding', $query, $roots, 'SUPPORTS', 10, 'outgoing', 'squared_euclidean') YIELD node_id, distance";
const EXPANDED_BATCH_SOURCE: &str = "CALL selene.vector_score_expanded_candidates_batch('embedding', $queries, $roots, 'SUPPORTS', 10, 'outgoing', 'squared_euclidean') YIELD query_index, node_id, distance";
const EXPANDED_QUERY_ROOTS_SOURCE: &str = "MATCH (root:VectorRoot) WHERE root.query_index = $query_index WITH collect_list(root) AS roots CALL selene.vector_score_expanded_candidates('embedding', $query, roots, 'SUPPORTS', 10, 'outgoing', 'squared_euclidean') YIELD node_id, distance RETURN node_id, distance";
const VECTOR_SCALE: usize = 1_000;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_BATCH_QUERIES: usize = 8;
const VECTOR_EXPANDED_CANDIDATES: usize = 64;
const VECTOR_EXPANDED_ROOTS: usize = 2;

pub(super) fn bench_vector_expanded_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = vector_expanded_graph(VECTOR_SCALE, VECTOR_DIMENSION);
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let query_roots_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    warm_expanded_cache(&graph, &registry, Arc::clone(&cache));
    warm_expanded_batch_cache(&graph, &registry, Arc::clone(&batch_cache));
    warm_expanded_query_roots_cache(&graph, &registry, Arc::clone(&query_roots_cache));

    let mut group = c.benchmark_group("procedure_vector_expanded");
    group.throughput(Throughput::Elements(VECTOR_EXPANDED_CANDIDATES as u64));
    group.bench_function("shared_cache_score_expanded_2root64_dim128_k10_1000", |b| {
        b.iter(|| {
            std::hint::black_box(execute_vector_expanded_score(
                &graph,
                &registry,
                Some(Arc::clone(&cache)),
                0,
            ));
        });
    });
    group.bench_function(
        "shared_cache_score_expanded_repeated_8x2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_expanded_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_expanded_batch_8x2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_expanded_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&batch_cache)),
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_expanded_query_roots_2root64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_expanded_query_roots_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&query_roots_cache)),
                    0,
                ));
            });
        },
    );
    group.finish();
}

fn warm_expanded_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_expanded_inputs_for(&mut session, 0);
    session
        .execute_source(EXPANDED_SOURCE, registry)
        .expect("warmup vector expanded scoring executes");
}

fn warm_expanded_batch_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_expanded_batch_inputs(&mut session);
    session
        .execute_source(EXPANDED_BATCH_SOURCE, registry)
        .expect("warmup batched vector expanded scoring executes");
}

fn warm_expanded_query_roots_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_expanded_query_root_inputs_for(&mut session, 0);
    session
        .execute_source(EXPANDED_QUERY_ROOTS_SOURCE, registry)
        .expect("warmup query-root vector expanded scoring executes");
}

fn execute_vector_expanded_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_expanded_inputs_for(&mut session, query_index);
    match session
        .execute_source(EXPANDED_SOURCE, registry)
        .expect("vector expanded scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_expanded_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_expanded_score(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
        );
    }
    rows
}

fn execute_vector_expanded_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_expanded_batch_inputs(&mut session);
    match session
        .execute_source(EXPANDED_BATCH_SOURCE, registry)
        .expect("batched vector expanded scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_expanded_query_roots_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_expanded_query_root_inputs_for(&mut session, query_index);
    match session
        .execute_source(EXPANDED_QUERY_ROOTS_SOURCE, registry)
        .expect("query-root vector expanded scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn vector_expanded_graph(scale: usize, dimension: usize) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(71_004));
    let label = istr("VectorDoc");
    let root_label = istr("VectorRoot");
    let embedding_key = istr("embedding");
    let query_index_key = istr("query_index");
    let supports = istr("SUPPORTS");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let root_query_index = root_query_index(idx);
                let labels = match root_query_index {
                    Some(_) => LabelSet::from_iter([label.clone(), root_label.clone()]),
                    None => LabelSet::single(label.clone()),
                };
                let mut props = vec![(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, dimension)),
                )];
                if let Some(query_index) = root_query_index {
                    props.push((query_index_key.clone(), Value::Int(query_index as i64)));
                }
                let props =
                    PropertyMap::from_pairs(props).expect("bench vector properties are valid");
                mutator
                    .create_node(labels, props)
                    .expect("bench vector node insert succeeds");
            }
            for query_index in 0..VECTOR_BATCH_QUERIES {
                let range = expanded_candidate_range(query_index);
                let roots: Vec<_> = range.clone().take(VECTOR_EXPANDED_ROOTS).collect();
                for root in roots {
                    for candidate in range.clone().skip(VECTOR_EXPANDED_ROOTS) {
                        mutator
                            .create_edge(
                                supports.clone(),
                                node_id(root),
                                node_id(candidate),
                                PropertyMap::new(),
                            )
                            .expect("bench support edge insert succeeds");
                    }
                }
            }
        }
        txn.commit().expect("bench vector expanded fixture commits");
    }
    graph
}

fn bind_expanded_inputs_for(session: &mut Session<'_>, query_index: usize) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
    session.bind_parameter(istr("roots"), expanded_roots(query_index));
}

fn bind_expanded_query_root_inputs_for(session: &mut Session<'_>, query_index: usize) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
    session.bind_parameter(istr("query_index"), Value::Int(query_index as i64));
}

fn bind_expanded_batch_inputs(session: &mut Session<'_>) {
    session.bind_parameter(istr("queries"), vector_query_batch());
    session.bind_parameter(istr("roots"), expanded_root_batch());
}

fn vector_query_batch() -> Value {
    Value::List(
        (0..VECTOR_BATCH_QUERIES)
            .map(|query_index| Value::Vector(vector_value(query_index, VECTOR_DIMENSION)))
            .collect(),
    )
}

fn expanded_root_batch() -> Value {
    Value::List((0..VECTOR_BATCH_QUERIES).map(expanded_roots).collect())
}

fn expanded_roots(query_index: usize) -> Value {
    Value::List(
        expanded_candidate_range(query_index)
            .take(VECTOR_EXPANDED_ROOTS)
            .map(|idx| Value::NodeRef(node_id(idx)))
            .collect(),
    )
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

fn root_query_index(index: usize) -> Option<usize> {
    (0..VECTOR_BATCH_QUERIES).find(|query_index| {
        expanded_candidate_range(*query_index)
            .take(VECTOR_EXPANDED_ROOTS)
            .any(|root| root == index)
    })
}

fn node_id(index: usize) -> NodeId {
    NodeId::new((index + 1) as u64)
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

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}
