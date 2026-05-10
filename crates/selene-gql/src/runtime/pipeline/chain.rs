//! NEXT-chain pipeline operator.

use crate::{
    AggregateArg, BindingTableSchema, ExecutionPlan, FilterPredicate, PipelineOp, ProjectExpr,
    ValueExpr,
    runtime::{BindingTable, ExecutorError, TxContext, execute_plan, pipeline},
};

pub(super) fn execute(
    rhs: &ExecutionPlan,
    input: BindingTable,
    ctx: &TxContext<'_>,
) -> Result<BindingTable, ExecutorError> {
    if rhs.pattern_plan.is_none() && pipeline_references_schema(&rhs.pipeline, input.schema()) {
        pipeline::execute_pipeline(rhs.pipeline.as_slice(), input, ctx)
    } else {
        execute_plan(rhs, ctx)
    }
}

fn pipeline_references_schema(pipeline: &[PipelineOp], schema: &BindingTableSchema) -> bool {
    pipeline
        .iter()
        .any(|op| pipeline_op_references_schema(op, schema))
}

fn pipeline_op_references_schema(op: &PipelineOp, schema: &BindingTableSchema) -> bool {
    match op {
        PipelineOp::Filter(predicate) => predicate_references_schema(predicate, schema),
        PipelineOp::Project(items) | PipelineOp::Let(items) => items
            .iter()
            .any(|item| project_references_schema(item, schema)),
        PipelineOp::Unwind { source, .. } => project_references_schema(source, schema),
        PipelineOp::OrderBy(keys) => keys
            .iter()
            .any(|key| !key.binding_refs.is_empty() || expr_references_schema(&key.expr, schema)),
        PipelineOp::TopK { keys, .. } => keys
            .iter()
            .any(|key| !key.binding_refs.is_empty() || expr_references_schema(&key.expr, schema)),
        PipelineOp::GroupBy { keys, aggregates } => {
            keys.iter()
                .any(|key| project_references_schema(key, schema))
                || aggregates.iter().any(|aggregate| {
                    aggregate
                        .args
                        .iter()
                        .any(|arg| aggregate_arg_references_schema(arg, schema))
                })
        }
        PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
            rhs.pattern_plan.is_none() && pipeline_references_schema(&rhs.pipeline, schema)
        }
        PipelineOp::Distinct
        | PipelineOp::Limit { .. }
        | PipelineOp::Call(_)
        | PipelineOp::Mutation(_)
        | PipelineOp::Catalog(_)
        | PipelineOp::Tx(_) => false,
    }
}

fn predicate_references_schema(predicate: &FilterPredicate, schema: &BindingTableSchema) -> bool {
    !predicate.binding_refs.is_empty() || expr_references_schema(&predicate.expr, schema)
}

fn project_references_schema(item: &ProjectExpr, schema: &BindingTableSchema) -> bool {
    !item.binding_refs.is_empty() || expr_references_schema(&item.expr, schema)
}

fn aggregate_arg_references_schema(arg: &AggregateArg, schema: &BindingTableSchema) -> bool {
    expr_references_schema(&arg.expr, schema)
}

fn expr_references_schema(expr: &ValueExpr, schema: &BindingTableSchema) -> bool {
    match expr {
        ValueExpr::Variable { name, .. } => schema
            .columns
            .iter()
            .any(|column| column.name == Some(*name)),
        ValueExpr::Parameter { .. } | ValueExpr::Literal(_) => false,
        ValueExpr::PropertyAccess { target, .. } => expr_references_schema(target, schema),
        ValueExpr::ListAccess { target, index, .. } => {
            expr_references_schema(target, schema) || expr_references_schema(index, schema)
        }
        ValueExpr::ListLiteral { items, .. } => items
            .iter()
            .any(|item| expr_references_schema(item, schema)),
        ValueExpr::RecordLiteral { fields, .. } => fields
            .iter()
            .any(|(_, value)| expr_references_schema(value, schema)),
        ValueExpr::BinaryOp { lhs, rhs, .. } => {
            expr_references_schema(lhs, schema) || expr_references_schema(rhs, schema)
        }
        ValueExpr::UnaryOp { operand, .. } | ValueExpr::IsCheck { operand, .. } => {
            expr_references_schema(operand, schema)
        }
        ValueExpr::FunctionCall { args, .. }
        | ValueExpr::AllDifferent { items: args, .. }
        | ValueExpr::Same { items: args, .. } => {
            args.iter().any(|arg| expr_references_schema(arg, schema))
        }
        ValueExpr::InList { operand, list, .. } => {
            expr_references_schema(operand, schema)
                || list.iter().any(|item| expr_references_schema(item, schema))
        }
        ValueExpr::Like {
            operand, pattern, ..
        } => expr_references_schema(operand, schema) || expr_references_schema(pattern, schema),
        ValueExpr::Between {
            operand, low, high, ..
        } => {
            expr_references_schema(operand, schema)
                || expr_references_schema(low, schema)
                || expr_references_schema(high, schema)
        }
        ValueExpr::PropertyExists { target, .. } => expr_references_schema(target, schema),
        ValueExpr::Case {
            branches,
            else_branch,
            ..
        } => {
            branches.iter().any(|(condition, value)| {
                expr_references_schema(condition, schema) || expr_references_schema(value, schema)
            }) || else_branch
                .as_deref()
                .is_some_and(|value| expr_references_schema(value, schema))
        }
        ValueExpr::Exists { .. } | ValueExpr::CountSubquery { .. } => false,
    }
}
