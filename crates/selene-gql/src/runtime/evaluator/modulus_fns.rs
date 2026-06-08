//! ISO `MOD` scalar-function evaluation.

use selene_core::Value;

use crate::{
    BinaryOp, GqlType, SourceSpan,
    runtime::{DataExceptionSubclass, EvalCtx, ExecutorError},
};

use super::{
    binary_ops::{data_exception_value_with, eval_binary},
    cast,
    concat_ops::ConcatCaps,
};

pub(super) fn eval_mod(
    args: Vec<Value>,
    span: SourceSpan,
    ctx: &EvalCtx<'_, '_, '_, '_>,
) -> Result<Value, ExecutorError> {
    let [dividend, divisor]: [Value; 2] =
        args.try_into().expect("eval_fixed_args enforces MOD arity");
    if matches!(dividend, Value::Null) || matches!(divisor, Value::Null) {
        return Ok(Value::Null);
    }
    let target_type = divisor_numeric_type(&divisor).ok_or_else(|| {
        data_exception_value_with(
            DataExceptionSubclass::InvalidValueType,
            "MOD divisor is not numeric",
            span,
        )
    })?;
    let remainder = eval_binary(
        BinaryOp::Mod,
        dividend,
        divisor,
        span,
        ConcatCaps::from_impl_defined(ctx.impl_defined_caps()),
    )?;
    cast::eval_cast(remainder, &target_type, span, ctx)
}

fn divisor_numeric_type(value: &Value) -> Option<GqlType> {
    Some(match value {
        Value::Int(_) => GqlType::Integer,
        Value::Uint(_) => GqlType::Uint64,
        Value::Int128(_) => GqlType::Int128,
        Value::Uint128(_) => GqlType::Uint128,
        Value::Float(_) => GqlType::Float,
        Value::Float32(_) => GqlType::Float32,
        Value::Decimal(_) => GqlType::Decimal,
        _ => return None,
    })
}
