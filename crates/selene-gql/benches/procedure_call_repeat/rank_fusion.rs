use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput};
use selene_core::{DbString, GraphId, NodeId, Value};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache, Session, StatementOutput};
use selene_graph::SharedGraph;

const RRF_SOURCE: &str = "CALL selene.reciprocal_rank_fusion($rankings, 10) YIELD node_id, score";
const RRF_CASES: [RrfCase; 9] = [
    RrfCase::new(2, 64),
    RrfCase::new(2, 256),
    RrfCase::new(2, 1_024),
    RrfCase::new(4, 64),
    RrfCase::new(4, 256),
    RrfCase::new(4, 1_024),
    RrfCase::new(8, 64),
    RrfCase::new(8, 256),
    RrfCase::new(8, 1_024),
];

#[derive(Clone, Copy)]
struct RrfCase {
    ranking_count: usize,
    ranking_width: usize,
}

impl RrfCase {
    const fn new(ranking_count: usize, ranking_width: usize) -> Self {
        Self {
            ranking_count,
            ranking_width,
        }
    }

    fn candidate_visits(self) -> u64 {
        (self.ranking_count * self.ranking_width) as u64
    }

    fn name(self) -> String {
        format!(
            "shared_cache_rankings{}x{}_k10",
            self.ranking_count, self.ranking_width
        )
    }
}

pub(super) fn bench_rank_fusion_procedure(c: &mut Criterion) {
    let registry = BuiltinProcedureRegistry::new();
    let graph = SharedGraph::new(GraphId::new(71_009));

    let mut group = c.benchmark_group("procedure_reciprocal_rank_fusion");
    for case in RRF_CASES {
        let rankings = rrf_rankings(case);
        let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        warm_rrf_cache(&graph, &registry, Arc::clone(&cache), &rankings);

        group.throughput(Throughput::Elements(case.candidate_visits()));
        group.bench_function(case.name(), |b| {
            b.iter(|| {
                std::hint::black_box(execute_rrf(
                    &graph,
                    &registry,
                    Some(Arc::clone(&cache)),
                    &rankings,
                ));
            });
        });
    }
    group.finish();
}

fn warm_rrf_cache(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Arc<CallPlanCache>,
    rankings: &Value,
) {
    let mut session = Session::new(graph).with_call_plan_cache(cache);
    bind_rrf_inputs(&mut session, rankings);
    session
        .execute_source(RRF_SOURCE, registry)
        .expect("warmup reciprocal rank fusion executes");
}

fn execute_rrf(
    graph: &SharedGraph,
    registry: &BuiltinProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
    rankings: &Value,
) -> usize {
    let mut session = Session::new(graph);
    if let Some(cache) = cache {
        session = session.with_call_plan_cache(cache);
    }
    bind_rrf_inputs(&mut session, rankings);
    match session
        .execute_source(RRF_SOURCE, registry)
        .expect("reciprocal rank fusion procedure executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn bind_rrf_inputs(session: &mut Session<'_>, rankings: &Value) {
    session.bind_parameter(db_string("rankings"), rankings.clone());
}

fn rrf_rankings(case: RrfCase) -> Value {
    let overlap_step = (case.ranking_width / 2).max(1);
    Value::List(
        (0..case.ranking_count)
            .map(|ranking_index| {
                let start = ranking_index * overlap_step;
                Value::List(
                    (start..start + case.ranking_width)
                        .map(|node_index| Value::NodeRef(NodeId::new((node_index + 1) as u64)))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}
