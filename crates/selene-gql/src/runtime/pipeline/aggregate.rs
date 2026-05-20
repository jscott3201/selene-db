use rustc_hash::FxHashSet;
use selene_core::Value;

use crate::{
    Aggregate, SourceSpan,
    runtime::{
        Binding, BindingTableSchema, EvalCtx, ExecutorError, evaluator, value_compare,
        value_key::RuntimeEqKey,
    },
};

pub(super) struct AggregateSlot {
    aggregate: Aggregate,
    state: AggregateState,
    seen: FxHashSet<RuntimeEqKey>,
}

impl AggregateSlot {
    pub(super) fn new(aggregate: &Aggregate) -> Result<Self, ExecutorError> {
        Ok(Self {
            aggregate: aggregate.clone(),
            state: AggregateState::new(classify(aggregate)?),
            seen: FxHashSet::default(),
        })
    }

    pub(super) fn observe(
        &mut self,
        row: &Binding,
        schema: &BindingTableSchema,
        ctx: &EvalCtx<'_, '_, '_, '_>,
    ) -> Result<(), ExecutorError> {
        if matches!(self.state, AggregateState::CountStar { .. }) {
            return self.state.observe(None, self.aggregate.span);
        }
        let arg = self
            .aggregate
            .args
            .first()
            .ok_or(ExecutorError::ImplementationDefined {
                detail: "aggregate argument missing",
            })?;
        let value = evaluator::evaluate(&arg.expr, row, schema, ctx)?;
        if self.state.skips_null() && matches!(value, Value::Null) {
            return Ok(());
        }
        if self.aggregate.distinct {
            let key = RuntimeEqKey::from_row(vec![value.clone()]);
            if !self.seen.insert(key) {
                return Ok(());
            }
        }
        self.state.observe(Some(value), self.aggregate.span)
    }

    pub(super) fn finalize_values(self) -> Result<Vec<Value>, ExecutorError> {
        let value = self.state.finalize(self.aggregate.span)?;
        Ok(vec![value])
    }
}

pub(super) fn output_names(aggregate: &Aggregate) -> Vec<selene_core::IStr> {
    vec![aggregate.output_name]
}

#[derive(Clone, Copy)]
enum AggregateFn {
    Count,
    CountStar,
    Sum,
    Avg,
    StddevPop,
    StddevSamp,
    Min,
    Max,
    Collect,
}

enum AggregateState {
    Count { count: u64 },
    CountStar { count: u64 },
    Sum { sum: Option<NumericSum> },
    Avg { sum: Option<NumericSum>, count: u64 },
    StddevPop { stats: Welford },
    StddevSamp { stats: Welford },
    Min { value: Option<Value> },
    Max { value: Option<Value> },
    Collect { values: Vec<Value> },
}

impl AggregateState {
    fn new(function: AggregateFn) -> Self {
        match function {
            AggregateFn::Count => Self::Count { count: 0 },
            AggregateFn::CountStar => Self::CountStar { count: 0 },
            AggregateFn::Sum => Self::Sum { sum: None },
            AggregateFn::Avg => Self::Avg {
                sum: None,
                count: 0,
            },
            AggregateFn::StddevPop => Self::StddevPop {
                stats: Welford::default(),
            },
            AggregateFn::StddevSamp => Self::StddevSamp {
                stats: Welford::default(),
            },
            AggregateFn::Min => Self::Min { value: None },
            AggregateFn::Max => Self::Max { value: None },
            AggregateFn::Collect => Self::Collect { values: Vec::new() },
        }
    }

    fn skips_null(&self) -> bool {
        !matches!(self, Self::CountStar { .. } | Self::Collect { .. })
    }

    fn observe(&mut self, value: Option<Value>, span: SourceSpan) -> Result<(), ExecutorError> {
        match self {
            Self::Count { count } => {
                *count = count.saturating_add(1);
                Ok(())
            }
            Self::CountStar { count } => {
                *count = count.saturating_add(1);
                Ok(())
            }
            Self::Sum { sum } => {
                let value = value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?;
                *sum = Some(add_numeric(sum.take(), value, span)?);
                Ok(())
            }
            Self::Avg { sum, count } => {
                let value = value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?;
                *sum = Some(add_numeric(sum.take(), value, span)?);
                *count = count.saturating_add(1);
                Ok(())
            }
            Self::StddevPop { stats } | Self::StddevSamp { stats } => {
                let value = value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?;
                stats.observe(value, span)
            }
            Self::Min { value: current } => update_min_max(current, value, span, true),
            Self::Max { value: current } => update_min_max(current, value, span, false),
            Self::Collect { values } => {
                values.push(value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?);
                Ok(())
            }
        }
    }

    fn finalize(self, span: SourceSpan) -> Result<Value, ExecutorError> {
        match self {
            Self::Count { count } | Self::CountStar { count } => count_to_value(count, span),
            Self::Sum { sum } => Ok(sum.map_or(Value::Int(0), NumericSum::into_value)),
            Self::Avg { sum, count } => avg_to_value(sum, count, span),
            Self::StddevPop { stats } => stddev_pop_to_value(stats, span),
            Self::StddevSamp { stats } => stddev_samp_to_value(stats, span),
            Self::Min { value } | Self::Max { value } => Ok(value.unwrap_or(Value::Null)),
            Self::Collect { values } => Ok(Value::List(values)),
        }
    }
}

#[derive(Clone)]
enum NumericSum {
    Int(i64),
    Float(f64),
}

impl NumericSum {
    fn into_value(self) -> Value {
        match self {
            Self::Int(value) => Value::Int(value),
            Self::Float(value) => Value::Float(value),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Welford {
    count: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    fn observe(&mut self, value: Value, span: SourceSpan) -> Result<(), ExecutorError> {
        let value = numeric_sum_to_f64(numeric_value(value, span)?, span)?;
        let count = self
            .count
            .checked_add(1)
            .ok_or_else(|| data_exception_value("aggregate count is out of range", span))?;
        let delta = value - self.mean;
        self.count = count;
        self.mean += delta / count as f64;
        let delta2 = value - self.mean;
        self.m2 = finite_float(self.m2 + delta * delta2, span)?;
        Ok(())
    }
}

fn classify(aggregate: &Aggregate) -> Result<AggregateFn, ExecutorError> {
    let name = aggregate.function.as_str();
    if aggregate.star {
        return if name == "count" && !aggregate.distinct {
            Ok(AggregateFn::CountStar)
        } else {
            Err(ExecutorError::ImplementationDefined {
                detail: "aggregate star form not implemented",
            })
        };
    }
    if aggregate.args.len() != 1 {
        return Err(ExecutorError::ImplementationDefined {
            detail: "aggregate arity not implemented",
        });
    }
    match name {
        "count" => Ok(AggregateFn::Count),
        "sum" => Ok(AggregateFn::Sum),
        "avg" | "average" => Ok(AggregateFn::Avg),
        "stddev_pop" => Ok(AggregateFn::StddevPop),
        "stddev_samp" => Ok(AggregateFn::StddevSamp),
        "min" => Ok(AggregateFn::Min),
        "max" => Ok(AggregateFn::Max),
        "collect" | "collect_list" => Ok(AggregateFn::Collect),
        _ => Err(ExecutorError::ImplementationDefined {
            detail: "aggregate function not implemented",
        }),
    }
}

fn update_min_max(
    current: &mut Option<Value>,
    next: Option<Value>,
    span: SourceSpan,
    keep_min: bool,
) -> Result<(), ExecutorError> {
    let next = next.ok_or(ExecutorError::ImplementationDefined {
        detail: "aggregate value missing",
    })?;
    let Some(current_value) = current else {
        *current = Some(next);
        return Ok(());
    };
    let ordering = value_compare::compare_non_null(&next, current_value)
        .ok_or_else(|| data_exception_value("aggregate value is not order-comparable", span))?;
    if (keep_min && ordering.is_lt()) || (!keep_min && ordering.is_gt()) {
        *current_value = next;
    }
    Ok(())
}

fn add_numeric(
    current: Option<NumericSum>,
    value: Value,
    span: SourceSpan,
) -> Result<NumericSum, ExecutorError> {
    let next = numeric_value(value, span)?;
    match (current, next) {
        (None, next) => Ok(next),
        (Some(NumericSum::Int(lhs)), NumericSum::Int(rhs)) => lhs
            .checked_add(rhs)
            .map(NumericSum::Int)
            .ok_or_else(|| data_exception_value("integer aggregate overflow", span)),
        (Some(lhs), rhs) => {
            let lhs = numeric_sum_to_f64(lhs, span)?;
            let rhs = numeric_sum_to_f64(rhs, span)?;
            finite_float(lhs + rhs, span).map(NumericSum::Float)
        }
    }
}

fn numeric_value(value: Value, span: SourceSpan) -> Result<NumericSum, ExecutorError> {
    match value {
        Value::Int(value) => Ok(NumericSum::Int(value)),
        Value::Uint(value) => i64::try_from(value)
            .map(NumericSum::Int)
            .map_err(|_| data_exception_value("unsigned aggregate value is out of range", span)),
        Value::Float(value) => finite_float(value, span).map(NumericSum::Float),
        Value::Float32(value) => finite_float(f64::from(value), span).map(NumericSum::Float),
        _ => Err(data_exception_value("aggregate value is not numeric", span)),
    }
}

fn numeric_sum_to_f64(value: NumericSum, span: SourceSpan) -> Result<f64, ExecutorError> {
    match value {
        NumericSum::Int(value) => i64_to_f64_exact(value).ok_or_else(|| {
            data_exception_value(
                "integer aggregate value is not exactly float-representable",
                span,
            )
        }),
        NumericSum::Float(value) => Ok(value),
    }
}

fn avg_to_value(
    sum: Option<NumericSum>,
    count: u64,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(sum) = sum else {
        return Ok(Value::Null);
    };
    if count == 0 {
        return Ok(Value::Null);
    }
    let sum = numeric_sum_to_f64(sum, span)?;
    finite_float(sum / count as f64, span).map(Value::Float)
}

fn stddev_pop_to_value(stats: Welford, span: SourceSpan) -> Result<Value, ExecutorError> {
    if stats.count == 0 {
        return Ok(Value::Null);
    }
    finite_float((stats.m2 / stats.count as f64).sqrt(), span).map(Value::Float)
}

fn stddev_samp_to_value(stats: Welford, span: SourceSpan) -> Result<Value, ExecutorError> {
    if stats.count < 2 {
        return Ok(Value::Null);
    }
    finite_float((stats.m2 / (stats.count - 1) as f64).sqrt(), span).map(Value::Float)
}

fn finite_float(value: f64, span: SourceSpan) -> Result<f64, ExecutorError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(data_exception_value(
            "floating-point aggregate produced non-finite value",
            span,
        ))
    }
}

fn count_to_value(count: u64, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(count)
        .map(Value::Int)
        .map_err(|_| data_exception_value("aggregate count is out of range", span))
}

fn i64_to_f64_exact(value: i64) -> Option<f64> {
    u64_representable_by_binary_float(value.unsigned_abs(), 53).then_some(value as f64)
}

fn u64_representable_by_binary_float(value: u64, significand_bits: u32) -> bool {
    if value == 0 {
        return true;
    }
    let exponent = u64::BITS - 1 - value.leading_zeros();
    if exponent < significand_bits {
        return true;
    }
    let low_bits = exponent + 1 - significand_bits;
    let mask = (1_u64 << low_bits) - 1;
    value & mask == 0
}

fn data_exception_value(message: impl Into<String>, span: SourceSpan) -> ExecutorError {
    ExecutorError::DataException {
        message: message.into(),
        span,
    }
}
