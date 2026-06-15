//! Flagger feature-gating coverage.

use selene_core::GraphId;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, TxContext, analyze,
    execute_pattern, execute_pipeline, parse, plan,
};
use selene_graph::SharedGraph;

#[path = "flagger/ddl_features.rs"]
mod ddl_features;
#[path = "flagger/path_features.rs"]
mod path_features;
#[path = "flagger/value_features.rs"]
mod value_features;

fn assert_read_plan(source: &str) {
    let _ = read_plan(source);
}

fn read_plan(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect(source);
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect(source)
}

fn assert_read_execution(source: &str) {
    let plan = read_plan(source);
    let graph = SharedGraph::new(GraphId::new(9151));
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx).expect(source)
    } else {
        BindingTable::new(
            BindingTableSchema {
                columns: Vec::new(),
            },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect(source);
}
