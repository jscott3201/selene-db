#![allow(missing_docs)]
//! Criterion benches for scalar expression execution throughput.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::GraphId;
use selene_gql::{
    EmptyProcedureRegistry, ExecutionPlan, Session, StatementOutput, analyze, execute_statement,
    parse, plan,
};
use selene_graph::SharedGraph;

struct ExpressionCase {
    group: &'static str,
    name: &'static str,
    source: &'static str,
}

const CASES: &[ExpressionCase] = &[
    ExpressionCase {
        group: "predicate",
        name: "starts_with",
        source: "RETURN 'alphabet' STARTS WITH 'a' AS v",
    },
    ExpressionCase {
        group: "predicate",
        name: "range",
        source: "RETURN 5 >= 1 AND 5 <= 10 AS v",
    },
    ExpressionCase {
        group: "scalar_fn",
        name: "length",
        source: "RETURN length('alphabet') AS v",
    },
    ExpressionCase {
        group: "scalar_fn",
        name: "abs",
        source: "RETURN abs(-42) AS v",
    },
    ExpressionCase {
        group: "scalar_fn",
        name: "substring",
        source: "RETURN substring('alphabet', 2, 4) AS v",
    },
    ExpressionCase {
        group: "case",
        name: "searched",
        source: "RETURN CASE WHEN false THEN 1 WHEN true THEN 2 ELSE 3 END AS v",
    },
    ExpressionCase {
        group: "collection",
        name: "list_access",
        source: "RETURN [10, 20, 30][1] AS v",
    },
    ExpressionCase {
        group: "binary_op",
        name: "concat",
        source: "RETURN 'alpha' || 'beta' AS v",
    },
    ExpressionCase {
        group: "binary_op",
        name: "starts_with",
        source: "RETURN 'alphabet' STARTS WITH 'alpha' AS v",
    },
];

fn bench_expression_eval(c: &mut Criterion) {
    let empty = EmptyProcedureRegistry;
    let graph = SharedGraph::new(GraphId::new(116));
    let plans = CASES
        .iter()
        .map(|case| (case, plan_expression(case.source)))
        .collect::<Vec<_>>();

    let mut group = c.benchmark_group("gql_expression_eval");
    for (case, plan) in &plans {
        group.bench_function(BenchmarkId::new(case.group, case.name), |b| {
            let mut session = Session::new(&graph);
            b.iter(|| {
                let output = execute_statement(std::hint::black_box(plan), &mut session, &empty)
                    .expect("expression statement executes");
                std::hint::black_box(output_rows(output));
            });
        });
    }
    group.finish();
}

fn plan_expression(source: &str) -> ExecutionPlan {
    let statement = parse(source).expect("expression bench source parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None)
        .expect("expression bench source analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("expression bench source plans")
}

fn output_rows(output: StatementOutput) -> usize {
    match output {
        StatementOutput::Rows(table) => table.row_count(),
        StatementOutput::Written(outcome) => {
            outcome.rows.as_ref().map_or(0, |table| table.row_count())
        }
        StatementOutput::Empty => 0,
        _ => panic!("unexpected expression bench output"),
    }
}

criterion_group! {
    name = expression_eval_group;
    config = common::criterion_config();
    targets = bench_expression_eval
}
criterion_main!(expression_eval_group);
