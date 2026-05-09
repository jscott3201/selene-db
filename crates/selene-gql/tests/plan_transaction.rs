//! BRIEF-27 transaction-control planner lowering tests.

use selene_gql::{EmptyProcedureRegistry, PipelineOp, TxOp, analyze, parse, plan};

fn plan_one(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

#[test]
fn transaction_control_lowers_to_tx_ops() {
    let cases = [
        ("START TRANSACTION", "start"),
        ("COMMIT", "commit"),
        ("ROLLBACK", "rollback"),
    ];
    for (source, expected) in cases {
        let plan = plan_one(source);
        assert!(plan.output_schema.columns.is_empty());
        let [PipelineOp::Tx(op)] = plan.pipeline.as_slice() else {
            panic!("expected tx op for {source}");
        };
        let actual = match op {
            TxOp::Start { .. } => "start",
            TxOp::Commit { .. } => "commit",
            TxOp::Rollback { .. } => "rollback",
        };
        assert_eq!(actual, expected);
    }
}
