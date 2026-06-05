use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorValue, intern};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};

const CANDIDATE_STATE_SOURCE: &str = "CALL selene.vector_score_candidate_state('embedding', $query, 'active_docs', 10, 'squared_euclidean') YIELD node_id, distance";
const CANDIDATE_STATE_NODES_SOURCE: &str = "CALL selene.vector_score_candidate_state_nodes('embedding', $query, 'active_docs', $nodes, 10, $operation, 'squared_euclidean') YIELD node_id, distance";
const CANDIDATE_STATE_EXPANDED_SOURCE: &str = "CALL selene.vector_score_candidate_state_expanded('embedding', $query, 'active_docs', $roots, $edge_label, 10, $operation, 'outgoing', 'squared_euclidean') YIELD node_id, distance";
const CANDIDATE_STATE_EXPANDED_BATCH_SOURCE: &str = "CALL selene.vector_score_candidate_state_expanded_batch('embedding', $queries, 'active_docs', $roots, $edge_label, 10, $operation, 'outgoing', 'squared_euclidean') YIELD query_index, node_id, distance";
const VECTOR_SCALE: usize = 1_000;
const VECTOR_DIMENSION: usize = 128;
const VECTOR_BATCH_QUERIES: usize = 8;
const VECTOR_CANDIDATE_STATE_CANDIDATES: usize = 64;
const VECTOR_CANDIDATE_STATE_HALF_CANDIDATES: usize = 32;
const VECTOR_EXPANDED_ROOTS: usize = 2;

#[derive(Clone, Copy)]
enum CandidateStateNodeFixture {
    Intersection64,
    Union128,
    StateDifference32,
}

#[derive(Clone, Copy)]
enum CandidateStateExpandedFixture {
    Intersection64,
    Union128,
    StateDifference32,
}

impl CandidateStateExpandedFixture {
    fn operation(self) -> &'static str {
        match self {
            Self::Intersection64 => "intersection",
            Self::Union128 => "union",
            Self::StateDifference32 => "state_difference",
        }
    }

    fn roots(self) -> Value {
        match self {
            Self::Intersection64 | Self::StateDifference32 => node_list(0, VECTOR_EXPANDED_ROOTS),
            Self::Union128 => node_list(VECTOR_CANDIDATE_STATE_CANDIDATES, VECTOR_EXPANDED_ROOTS),
        }
    }

    fn edge_label(self) -> &'static str {
        match self {
            Self::Intersection64 | Self::Union128 => "SUPPORTS",
            Self::StateDifference32 => "SUPPORTS_HALF",
        }
    }
}

impl CandidateStateNodeFixture {
    fn operation(self) -> &'static str {
        match self {
            Self::Intersection64 => "intersection",
            Self::Union128 => "union",
            Self::StateDifference32 => "state_difference",
        }
    }

    fn nodes(self) -> Value {
        match self {
            Self::Intersection64 => node_list(0, VECTOR_CANDIDATE_STATE_CANDIDATES),
            Self::Union128 => node_list(
                VECTOR_CANDIDATE_STATE_CANDIDATES,
                VECTOR_CANDIDATE_STATE_CANDIDATES,
            ),
            Self::StateDifference32 => node_list(0, VECTOR_CANDIDATE_STATE_HALF_CANDIDATES),
        }
    }
}

pub(super) fn bench_vector_candidate_state_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = vector_candidate_state_graph(
        VECTOR_SCALE,
        VECTOR_DIMENSION,
        VECTOR_CANDIDATE_STATE_CANDIDATES,
    );
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let nodes_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let expanded_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let expanded_batch_cache =
        Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    warm_candidate_state_cache(&graph, &registry, Arc::clone(&cache));
    warm_candidate_state_nodes_cache(&graph, &registry, Arc::clone(&nodes_cache));
    warm_candidate_state_expanded_cache(&graph, &registry, Arc::clone(&expanded_cache));
    warm_candidate_state_expanded_batch_cache(&graph, &registry, Arc::clone(&expanded_batch_cache));

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
    group.bench_function(
        "shared_cache_score_candidate_state_nodes_intersection_64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_nodes_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&nodes_cache)),
                    0,
                    CandidateStateNodeFixture::Intersection64,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_nodes_intersection_repeated_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_nodes_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&nodes_cache)),
                    CandidateStateNodeFixture::Intersection64,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_nodes_union_128_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_nodes_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&nodes_cache)),
                    0,
                    CandidateStateNodeFixture::Union128,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_nodes_state_difference_32_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_nodes_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&nodes_cache)),
                    0,
                    CandidateStateNodeFixture::StateDifference32,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_expanded_intersection_64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_expanded_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&expanded_cache)),
                    0,
                    CandidateStateExpandedFixture::Intersection64,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_expanded_intersection_repeated_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_expanded_repeated_batch(
                    &graph,
                    &registry,
                    Some(Arc::clone(&expanded_cache)),
                    CandidateStateExpandedFixture::Intersection64,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_expanded_intersection_batch_8x64_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_expanded_batch_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&expanded_batch_cache)),
                    CandidateStateExpandedFixture::Intersection64,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_expanded_union_128_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_expanded_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&expanded_cache)),
                    0,
                    CandidateStateExpandedFixture::Union128,
                ));
            });
        },
    );
    group.bench_function(
        "shared_cache_score_candidate_state_expanded_state_difference_32_dim128_k10_1000",
        |b| {
            b.iter(|| {
                std::hint::black_box(execute_vector_candidate_state_expanded_score(
                    &graph,
                    &registry,
                    Some(Arc::clone(&expanded_cache)),
                    0,
                    CandidateStateExpandedFixture::StateDifference32,
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

fn warm_candidate_state_nodes_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_candidate_state_node_inputs_for(
        &mut session,
        0,
        CandidateStateNodeFixture::Intersection64,
    );
    session
        .execute_source(CANDIDATE_STATE_NODES_SOURCE, registry)
        .expect("warmup vector candidate-state node-composition scoring executes");
}

fn warm_candidate_state_expanded_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_candidate_state_expanded_inputs_for(
        &mut session,
        0,
        CandidateStateExpandedFixture::Intersection64,
    );
    session
        .execute_source(CANDIDATE_STATE_EXPANDED_SOURCE, registry)
        .expect("warmup vector candidate-state expanded scoring executes");
}

fn warm_candidate_state_expanded_batch_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_candidate_state_expanded_batch_inputs_for(
        &mut session,
        CandidateStateExpandedFixture::Intersection64,
    );
    session
        .execute_source(CANDIDATE_STATE_EXPANDED_BATCH_SOURCE, registry)
        .expect("warmup batched vector candidate-state expanded scoring executes");
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

fn execute_vector_candidate_state_nodes_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
    fixture: CandidateStateNodeFixture,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_candidate_state_node_inputs_for(&mut session, query_index, fixture);
    match session
        .execute_source(CANDIDATE_STATE_NODES_SOURCE, registry)
        .expect("vector candidate-state node-composition scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_candidate_state_expanded_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    query_index: usize,
    fixture: CandidateStateExpandedFixture,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_candidate_state_expanded_inputs_for(&mut session, query_index, fixture);
    match session
        .execute_source(CANDIDATE_STATE_EXPANDED_SOURCE, registry)
        .expect("vector candidate-state expanded scoring procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn execute_vector_candidate_state_expanded_batch_score(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    fixture: CandidateStateExpandedFixture,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_candidate_state_expanded_batch_inputs_for(&mut session, fixture);
    match session
        .execute_source(CANDIDATE_STATE_EXPANDED_BATCH_SOURCE, registry)
        .expect("batched vector candidate-state expanded scoring procedure executes")
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

fn execute_vector_candidate_state_nodes_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    fixture: CandidateStateNodeFixture,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_candidate_state_nodes_score(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
            fixture,
        );
    }
    rows
}

fn execute_vector_candidate_state_expanded_repeated_batch(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    fixture: CandidateStateExpandedFixture,
) -> usize {
    let mut rows = 0;
    for query_index in 0..VECTOR_BATCH_QUERIES {
        rows += execute_vector_candidate_state_expanded_score(
            graph,
            registry,
            cache.as_ref().map(Arc::clone),
            query_index,
            fixture,
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
    let supports = istr("SUPPORTS");
    let supports_half = istr("SUPPORTS_HALF");
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
            for (edge_label, start, len) in [
                (supports.clone(), 0, VECTOR_CANDIDATE_STATE_CANDIDATES),
                (
                    supports.clone(),
                    VECTOR_CANDIDATE_STATE_CANDIDATES,
                    VECTOR_CANDIDATE_STATE_CANDIDATES,
                ),
                (supports_half, 0, VECTOR_CANDIDATE_STATE_HALF_CANDIDATES),
            ] {
                for root in start..start + VECTOR_EXPANDED_ROOTS {
                    for candidate in start + VECTOR_EXPANDED_ROOTS..start + len {
                        mutator
                            .create_edge(
                                edge_label.clone(),
                                node_id(root),
                                node_id(candidate),
                                PropertyMap::new(),
                            )
                            .expect("bench support edge insert succeeds");
                    }
                }
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

fn bind_candidate_state_node_inputs_for(
    session: &mut Session<'_>,
    query_index: usize,
    fixture: CandidateStateNodeFixture,
) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
    session.bind_parameter(istr("nodes"), fixture.nodes());
    session.bind_parameter(istr("operation"), Value::String(istr(fixture.operation())));
}

fn bind_candidate_state_expanded_inputs_for(
    session: &mut Session<'_>,
    query_index: usize,
    fixture: CandidateStateExpandedFixture,
) {
    session.bind_parameter(
        istr("query"),
        Value::Vector(vector_value(query_index, VECTOR_DIMENSION)),
    );
    session.bind_parameter(istr("roots"), fixture.roots());
    session.bind_parameter(
        istr("edge_label"),
        Value::String(istr(fixture.edge_label())),
    );
    session.bind_parameter(istr("operation"), Value::String(istr(fixture.operation())));
}

fn bind_candidate_state_expanded_batch_inputs_for(
    session: &mut Session<'_>,
    fixture: CandidateStateExpandedFixture,
) {
    session.bind_parameter(
        istr("queries"),
        Value::List(
            (0..VECTOR_BATCH_QUERIES)
                .map(|query_index| Value::Vector(vector_value(query_index, VECTOR_DIMENSION)))
                .collect(),
        ),
    );
    session.bind_parameter(
        istr("roots"),
        Value::List((0..VECTOR_BATCH_QUERIES).map(|_| fixture.roots()).collect()),
    );
    session.bind_parameter(
        istr("edge_label"),
        Value::String(istr(fixture.edge_label())),
    );
    session.bind_parameter(istr("operation"), Value::String(istr(fixture.operation())));
}

fn node_list(start: usize, len: usize) -> Value {
    Value::List(
        (start..start + len)
            .map(|idx| Value::NodeRef(NodeId::new((idx + 1) as u64)))
            .collect(),
    )
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
