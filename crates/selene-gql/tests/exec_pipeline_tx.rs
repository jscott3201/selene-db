//! Defensive transaction-control pipeline tests.

use selene_core::GraphId;
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ExecutorError,
    ImplDefinedCaps, PipelineOp, TxContext, analyze, execute_pipeline, parse, plan,
};
use selene_graph::SharedGraph;

fn tx_op(source: &str) -> PipelineOp {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let mut plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    plan.pipeline.remove(0)
}

#[test]
fn tx_op_inside_execute_pipeline_returns_implementation_defined() {
    let graph = SharedGraph::new(GraphId::new(3820));
    let op = tx_op("START TRANSACTION");
    let caps = ImplDefinedCaps::default();
    let mut ctx = TxContext::read_only(graph.read(), &caps);
    let input = BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    );

    let err = execute_pipeline(&[op], input, &mut ctx).expect_err("tx op is defensive error");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "TX op surfaced inside execute_pipeline; should be dispatched at statement level"
        }
    ));
}
