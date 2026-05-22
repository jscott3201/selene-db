#![allow(missing_docs)]
//! Criterion benches for repeated procedure-call planning.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::{num::NonZeroUsize, sync::Arc};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, Value, intern};
use selene_gql::{
    CallPlanCache, GqlType, ProcedureContext, ProcedureError, ProcedureHandle, ProcedureMetadata,
    ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema, ProcedureRegistry,
    ProcedureResult, ProcedureSignature, ProcedureTier, Session, StatementOutput,
};
use selene_graph::SharedGraph;

const SOURCE: &str = "CALL bench.repeat() YIELD n";
const REPEATS: usize = 100;

struct RepeatRegistry {
    name: Box<[IStr]>,
    metadata: ProcedureMetadata,
}

impl RepeatRegistry {
    fn new() -> Self {
        let name = Box::from([istr("bench"), istr("repeat")]);
        Self {
            name,
            metadata: ProcedureMetadata::new(
                ProcedureHandle::new(1),
                ProcedureSignature::new(Vec::new()),
                ProcedureOutputSchema {
                    columns: vec![ProcedureOutputColumn::new(istr("n"), GqlType::Integer)],
                },
                ProcedureTier::Graph,
                ProcedureMutability::Read,
                None,
            ),
        }
    }
}

impl ProcedureRegistry for RepeatRegistry {
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata> {
        (name == self.name.as_ref()).then(|| self.metadata.clone())
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        Ok(ProcedureResult {
            rows: vec![vec![Value::Int(1)]],
        })
    }
}

fn bench_procedure_call_repeat(c: &mut Criterion) {
    let registry = RepeatRegistry::new();
    let graph = SharedGraph::new(GraphId::new(71_001));
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source(SOURCE, &registry)
        .expect("warmup call executes");

    let mut group = c.benchmark_group("procedure_call_repeat");
    group.throughput(Throughput::Elements(REPEATS as u64));
    group.bench_function("no_cache", |b| {
        b.iter(|| std::hint::black_box(execute_repeated(&graph, &registry, None)));
    });
    group.bench_function("shared_cache", |b| {
        b.iter(|| {
            std::hint::black_box(execute_repeated(
                &graph,
                &registry,
                Some(Arc::clone(&cache)),
            ));
        });
    });
    group.finish();
}

fn execute_repeated(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    cache: Option<Arc<CallPlanCache>>,
) -> usize {
    let mut rows = 0;
    for _ in 0..REPEATS {
        let mut session = Session::new(graph);
        if let Some(cache) = cache.as_ref() {
            session = session.with_call_plan_cache(Arc::clone(cache));
        }
        match session
            .execute_source(SOURCE, registry)
            .expect("procedure call executes")
        {
            StatementOutput::Rows(table) => rows += table.row_count(),
            other => panic!("unexpected output: {other:?}"),
        }
    }
    rows
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}

criterion_group! {
    name = procedure_call_repeat_group;
    config = common::criterion_config();
    targets = bench_procedure_call_repeat
}
criterion_main!(procedure_call_repeat_group);
