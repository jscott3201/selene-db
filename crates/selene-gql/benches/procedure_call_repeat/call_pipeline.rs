use std::num::NonZeroUsize;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{GraphId, LabelSet, PropertyMap};
use selene_gql::{Session, StatementOutput};
use selene_graph::SharedGraph;
use selene_testing::BenchProfile;

use super::{RepeatRegistry, db_string};

const PIPELINE_SOURCE: &str = "MATCH (input:CallInput) \
    CALL bench.repeat() YIELD n RETURN input, n";

pub(crate) fn bench_call_pipeline(c: &mut Criterion) {
    let registry = RepeatRegistry::new();
    let mut group = c.benchmark_group("procedure_call_pipeline");
    for &scale in BenchProfile::from_env().scales() {
        let graph = call_input_graph(scale);
        let mut session =
            Session::new(&graph).with_plan_cache(NonZeroUsize::new(16).expect("nonzero"));
        let primed = execute_pipeline(&mut session, &registry);
        assert_eq!(
            primed, scale,
            "procedure CALL pipeline should emit one row per input node"
        );

        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::new("match_call_repeat", scale), |b| {
            b.iter(|| std::hint::black_box(execute_pipeline(&mut session, &registry)));
        });
    }
    group.finish();
}

fn execute_pipeline(session: &mut Session<'_>, registry: &RepeatRegistry) -> usize {
    match session
        .execute_source(PIPELINE_SOURCE, registry)
        .expect("procedure CALL pipeline executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("unexpected output: {other:?}"),
    }
}

fn call_input_graph(scale: usize) -> SharedGraph {
    let graph = SharedGraph::new(GraphId::new(71_011));
    let label = db_string("CallInput");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for _ in 0..scale {
                mutator
                    .create_node(LabelSet::single(label.clone()), PropertyMap::new())
                    .expect("bench CALL input node insert succeeds");
            }
        }
        txn.commit().expect("bench CALL input fixture commits");
    }
    graph
}
