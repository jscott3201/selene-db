use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};

const CANDIDATE_STATE_SOURCE: &str = "CALL selene.vector_score_candidate_state('embedding', $query, 'active_docs', 10, 'squared_euclidean') YIELD node_id, distance";
const VECTOR_SCALE: usize = 1_000;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_BATCH_QUERIES: usize = 8;
const VECTOR_CANDIDATE_STATE_CANDIDATES: usize = 64;

pub(super) fn bench_vector_candidate_state_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = vector_candidate_state_graph(
        VECTOR_SCALE,
        VECTOR_DIMENSION,
        VECTOR_CANDIDATE_STATE_CANDIDATES,
    );
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    warm_candidate_state_cache(&graph, &registry, Arc::clone(&cache));

    let mut group = c.benchmark_group("procedure_vector_candidate_state");
    group.throughput(Throughput::Elements(
        VECTOR_CANDIDATE_STATE_CANDIDATES as u64,
    ));
    group.bench_function(
        "shared_cache_score_candidate_state_64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                    0,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_repeated_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                ));
            });
        },
    );
    group.finish();
}

fn warm_candidate_state_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_candidate_state_inputs_for(&mut session, 0);
    session
        .execute_source(CANDIDATE_STATE_SOURCE, registry)
        .expect("warmup vector candidate-state scoring executes");
}

fn execute_vector_candidate_state_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_candidate_state_inputs_for(&mut session, query_index);
    match session
        .execute_source(CANDIDATE_STATE_SOURCE, registry)
        .expect("vector candidate-state scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_candidate_state_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_candidate_state_score(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
        );
    }
    rows
}

fn vector_candidate_state_graph(
    scale: usize,
    dimension: usize,
    active_count: usize,
) -> SharedGraph {
    let active_doc = istr("ActiveVectorDoc");
    let state_name = istr("active_docs");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([
            CandidateStateSpec::new(state_name).require_label(active_doc.clone())
        ])
        .expect("bench candidate-state spec is valid"),
    );
    let graph = SharedGraph::builder(GraphId::new(71_006))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("bench graph builds");
    let vector_doc = istr("VectorDoc");
    let embedding_key = istr("embedding");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let labels = if idx < active_count {
                    LabelSet::from_iter([vector_doc.clone(), active_doc.clone()])
                } else {
                    LabelSet::single(vector_doc.clone())
                };
                let props = PropertyMap::from_pairs([(
                    embedding_key.clone(),
                    Value::Vector(vector_value(idx, dimension)),
                )])
                .expect("bench vector properties are valid");
                mutator
                    .create_node(labels, props)
                    .expect("bench vector node insert succeeds");
            }
        }
        txn.commit()
            .expect("bench vector candidate-state fixture commits");
    }
    graph
}

fn bind_candidate_state_inputs_for(session: &mut Session<'_>, query_index: usize) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
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
