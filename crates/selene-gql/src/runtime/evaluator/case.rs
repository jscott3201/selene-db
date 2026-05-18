//! CASE-expression evaluation.

use selene_core::Value;

use crate::{
    SourceSpan, ValueExpr,
    runtime::{Binding, BindingTableSchema, ExecutorError, TxContext},
};

use super::{binary_ops::data_exception, evaluate};

pub(super) fn eval_case(
    branches: &[(ValueExpr, ValueExpr)],
    else_branch: Option<&ValueExpr>,
    span: SourceSpan,
    binding: &Binding,
    schema: &BindingTableSchema,
    ctx: &TxContext<'_, '_>,
) -> Result<Value, ExecutorError> {
    for (condition, value) in branches {
        match evaluate(condition, binding, schema, ctx)? {
            Value::Bool(true) => return evaluate(value, binding, schema, ctx),
            Value::Bool(false) | Value::Null => {}
            _ => return data_exception("CASE condition is not boolean", span),
        }
    }
    if let Some(else_branch) = else_branch {
        evaluate(else_branch, binding, schema, ctx)
    } else {
        Ok(Value::Null)
    }
}
