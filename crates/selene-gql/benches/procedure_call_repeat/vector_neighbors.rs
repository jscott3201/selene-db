use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::SharedGraph;

const NEIGHBOR_SOURCE: &str = "CALL selene.vector_score_neighbors('embedding', $query, $anchor, 'DEPENDS_ON', 10, 'outgoing', 'squared_euclidean') YIELD node_id, distance";
const NEIGHBOR_BATCH_SOURCE: &str = "CALL selene.vector_score_neighbors_batch('embedding', $queries, $anchors, 'DEPENDS_ON', 10, 'outgoing', 'squared_euclidean') YIELD query_index, node_id, distance";
const VECTOR_SCALE: usize = 1_000;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_BATCH_QUERIES: usize = 8;
const VECTOR_NEIGHBOR_CANDIDATES: usize = 64;

pub(super) fn bench_vector_neighbor_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = vector_neighbor_graph(VECTOR_SCALE, VECTOR_DIMENSION);
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    warm_neighbor_cache(&graph, &registry, Arc::clone(&cache));
    warm_neighbor_batch_cache(&graph, &registry, Arc::clone(&batch_cache));

    let mut group = c.benchmark_group("procedure_vector_neighbors");
    group.throughput(Throughput::Elements(VECTOR_NEIGHBOR_CANDIDATES as u64));
    group.bench_function("shared_cache_score_neighbors_64_dim128_k10_1000", |b| {
        b.iter(|| {
            std::hint::black_box(execute_vector_neighbor_score(
                &graph,
                &registry,
                Some(Arc::clone(&cache)),
                0,
            ));
        });
    });
    group.bench_function(
        "shared_cache_score_neighbors_repeated_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_neighbor_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_neighbors_batch_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_neighbor_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&batch_cache)),
                ));
            });
        },
    );
    group.finish();
}

fn warm_neighbor_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_neighbor_inputs_for(&mut session, 0);
    session
        .execute_source(NEIGHBOR_SOURCE, registry)
        .expect("warmup vector neighbor scoring executes");
}

fn warm_neighbor_batch_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_neighbor_batch_inputs(&mut session);
    session
        .execute_source(NEIGHBOR_BATCH_SOURCE, registry)
        .expect("warmup batched vector neighbor scoring executes");
}

fn execute_vector_neighbor_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_neighbor_inputs_for(&mut session, query_index);
    match session
        .execute_source(NEIGHBOR_SOURCE, registry)
        .expect("vector neighbor scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_neighbor_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_neighbor_score(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
        );
    }
    rows
}

fn execute_vector_neighbor_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_neighbor_batch_inputs(&mut session);
    match session
        .execute_source(NEIGHBOR_BATCH_SOURCE, registry)
        .expect("batched vector neighbor scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn vector_neighbor_graph(scale: usize, dimension: usize) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(71_003));
    let label = istr("VectorDoc");
    let anchor_label = istr("Anchor");
    let embedding_key = istr("embedding");
    let depends = istr("DEPENDS_ON");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let props = PropertyMap::from_pairs([(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, dimension)),
                )])
                .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            for query_index in 0..VECTOR_BATCH_QUERIES {
                let anchor = mutator
                    .create_node(LabelSet::single(anchor_label.clone()), PropertyMap::new())
                    .expect("bench anchor insert succeeds");
                for candidate in neighbor_candidate_range(query_index) {
                    mutator
                        .create_edge(
                            depends.clone(),
                            anchor,
                            NodeId::new((candidate + 1) as u64),
                            PropertyMap::new(),
                        )
                        .expect("bench dependency edge insert succeeds");
                }
            }
        }
        txn.commit().expect("bench vector neighbor fixture commits");
    }
    graph
}

fn bind_neighbor_inputs_for(session: &mut Session<'_>, query_index: usize) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
    session.bind_parameter(istr("anchor"), Value::NodeRef(neighbor_anchor(query_index)));
}

fn bind_neighbor_batch_inputs(session: &mut Session<'_>) {
    session.bind_parameter(istr("queries"), vector_query_batch());
    session.bind_parameter(istr("anchors"), neighbor_anchor_batch());
}

fn vector_query_batch() -> Value {
    Value::List(
        (0..VECTOR_BATCH_QUERIES)
            .map(|query_index| Value::Vector(vector_value(query_index, VECTOR_DIMENSION)))
            .collect(),
    )
}

fn neighbor_anchor_batch() -> Value {
    Value::List(
        (0..VECTOR_BATCH_QUERIES)
            .map(|query_index| Value::NodeRef(neighbor_anchor(query_index)))
            .collect(),
    )
}

fn neighbor_anchor(query_index: usize) -> NodeId {
    NodeId::new((VECTOR_SCALE + query_index + 1) as u64)
}

fn neighbor_candidate_range(query_index: usize) -> std::ops::Range<usize> {
    let max_start = VECTOR_SCALE.saturating_sub(VECTOR_NEIGHBOR_CANDIDATES);
    let start = if max_start == 0 {
        0
    } else {
        (query_index * VECTOR_NEIGHBOR_CANDIDATES) % max_start
    };
    start..start + VECTOR_NEIGHBOR_CANDIDATES
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
