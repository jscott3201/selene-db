//! Standalone execution-plan runner used by recursive runtime operators.

use crate::{
    BindingTableSchema, ExecutionPlan,
    runtime::{Binding, BindingTable, EvalCtx, ExecutorError, TxContext, pattern, pipeline},
};

pub(crate) fn execute_plan(
    plan: &ExecutionPlan,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let table = {
        let eval_ctx = EvalCtx {
            tx: ctx,
            expr_ids: &plan.expr_ids,
            subqueries: &plan.subqueries,
        };
        match &plan.pattern_plan {
            Some(pattern_plan) => {
                pattern::execute_pattern_with_seed(pattern_plan, None, &eval_ctx)?
            }
            None => seed_table(),
        }
    };
    pipeline::execute_pipeline_with_plan(
        plan.pipeline.as_slice(),
        table,
        ctx,
        &plan.expr_ids,
        &plan.subqueries,
    )
}

pub(crate) fn execute_plan_read_only(
    plan: &ExecutionPlan,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    execute_plan_read_only_with_seed(plan, None, ctx)
}

pub(crate) fn execute_plan_read_only_with_seed(
    plan: &ExecutionPlan,
    seed: Option<BindingTable>,
    ctx: &TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    let table = {
        let eval_ctx = EvalCtx {
            tx: ctx,
            expr_ids: &plan.expr_ids,
            subqueries: &plan.subqueries,
        };
        match (&plan.pattern_plan, seed) {
            (Some(pattern_plan), Some(seed)) => {
                let (schema, rows) = seed.into_parts();
                let Some(row) = rows.first() else {
                    return Ok(BindingTable::new(schema, Vec::new()));
                };
                pattern::execute_pattern_with_seed_and_schema(
                    pattern_plan,
                    Some(row),
                    schema,
                    &eval_ctx,
                )?
            }
            (Some(pattern_plan), None) => {
                pattern::execute_pattern_with_seed(pattern_plan, None, &eval_ctx)?
            }
            (None, Some(seed)) => seed,
            (None, None) => seed_table(),
        }
    };
    pipeline::execute_pipeline_read_only_with_plan(
        plan.pipeline.as_slice(),
        table,
        ctx,
        &plan.expr_ids,
        &plan.subqueries,
    )
}

pub(crate) fn seed_table() -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: Vec::new(),
        },
        vec![Binding::empty()],
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::{GraphId, LabelSet, Value};
    use selene_graph::SharedGraph;

    use crate::{
        EmptyProcedureRegistry, ExecutionPlan, analyze, parse, plan,
        runtime::{TxContext, plan_runner::execute_plan},
    };

    fn planned(source: &str) -> ExecutionPlan {
        let statement = parse(source).expect("test input parses");
        let analyzed =
            analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
        plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
    }

    fn context(plan: &ExecutionPlan) -> TxContext<'_, '_> {
        TxContext::read_only(
            Arc::new(selene_graph::SeleneGraph::new(GraphId::new(991))),
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            &[],
        )
    }

    #[test]
    fn execute_plan_with_no_pattern_plan_seeds_single_empty_row() {
        let plan = planned("RETURN 1 AS n");
        let mut ctx = context(&plan);

        let table = execute_plan(&plan, &mut ctx).expect("plan executes");

        assert_eq!(table.row_count(), 1);
        assert_eq!(table.rows()[0].values(), &[Value::Int(1)]);
    }

    #[test]
    fn execute_plan_with_pattern_plan_runs_pattern_then_pipeline() {
        let graph = SharedGraph::new(GraphId::new(992));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), Default::default())
                .expect("node inserts");
            txn.commit().expect("fixture commits");
        }
        let plan = planned("MATCH (n) RETURN n");
        let mut ctx = TxContext::read_only(
            graph.read(),
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            graph.index_providers(),
        );

        let table = execute_plan(&plan, &mut ctx).expect("plan executes");

        assert_eq!(table.row_count(), 1);
        assert!(matches!(table.rows()[0].values(), [Value::NodeRef(_)]));
    }

    #[test]
    fn execute_plan_returns_pattern_table_when_pipeline_empty() {
        let graph = SharedGraph::new(GraphId::new(993));
        {
            let mut txn = graph.begin_write();
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::new(), Default::default())
                .expect("node inserts");
            txn.commit().expect("fixture commits");
        }
        let mut plan = planned("MATCH (n) RETURN n");
        plan.pipeline.clear();
        let mut ctx = TxContext::read_only(
            graph.read(),
            &plan.impl_defined_caps,
            &EmptyProcedureRegistry,
            graph.index_providers(),
        );

        let table = execute_plan(&plan, &mut ctx).expect("plan executes");

        assert_eq!(table.row_count(), 1);
        assert_eq!(table.schema().columns.len(), 1);
        assert!(matches!(table.rows()[0].values(), [Value::NodeRef(_)]));
    }
}
