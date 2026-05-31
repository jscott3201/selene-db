use rust_decimal::prelude::ToPrimitive;
use rustc_hash::FxHashSet;
use selene_core::Value;

use crate::{
    Aggregate, GqlStatus, SourceSpan,
    runtime::{
        Binding, BindingTableSchema, DataExceptionSubclass, EvalCtx, ExecutorError,
        ExecutorWarning, evaluator, value_compare, value_key::RuntimeEqKey,
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
        if self.state.needs_percentile() {
            let arg = self
                .aggregate
                .args
                .get(1)
                .ok_or(ExecutorError::ImplementationDefined {
                    detail: "PERCENTILE independent argument missing",
                })?;
            let value = evaluator::evaluate(&arg.expr, row, schema, ctx)?;
            let percentile = percentile_value(value, self.aggregate.span)?;
            self.state.set_percentile(percentile);
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
            ctx.tx.emit_warning_once(ExecutorWarning {
                code: GqlStatus::NULL_VALUE_ELIMINATED_IN_SET_FUNCTION,
                message: "null value eliminated in set function".to_owned(),
                span: self.aggregate.span,
            });
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
    PercentileCont,
    PercentileDisc,
}

enum AggregateState {
    Count {
        count: u64,
    },
    CountStar {
        count: u64,
    },
    Sum {
        sum: Option<NumericSum>,
    },
    Avg {
        sum: Option<NumericSum>,
        count: u64,
    },
    StddevPop {
        stats: Welford,
    },
    StddevSamp {
        stats: Welford,
    },
    Min {
        value: Option<Value>,
    },
    Max {
        value: Option<Value>,
    },
    Collect {
        values: Vec<Value>,
    },
    PercentileCont {
        values: Vec<f64>,
        percentile: Option<Option<f64>>,
    },
    PercentileDisc {
        values: Vec<Value>,
        percentile: Option<Option<f64>>,
    },
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
            AggregateFn::PercentileCont => Self::PercentileCont {
                values: Vec::new(),
                percentile: None,
            },
            AggregateFn::PercentileDisc => Self::PercentileDisc {
                values: Vec::new(),
                percentile: None,
            },
        }
    }

    fn skips_null(&self) -> bool {
        !matches!(self, Self::CountStar { .. } | Self::Collect { .. })
    }

    fn needs_percentile(&self) -> bool {
        matches!(
            self,
            Self::PercentileCont {
                percentile: None,
                ..
            } | Self::PercentileDisc {
                percentile: None,
                ..
            }
        )
    }

    fn set_percentile(&mut self, next: Option<f64>) {
        match self {
            Self::PercentileCont { percentile, .. } | Self::PercentileDisc { percentile, .. }
                if percentile.is_none() =>
            {
                *percentile = Some(next);
            }
            _ => {}
        }
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
            Self::PercentileCont { values, .. } => {
                let value = value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?;
                values.push(numeric_sum_to_f64(numeric_value(value, span)?, span)?);
                Ok(())
            }
            Self::PercentileDisc { values, .. } => {
                let value = value.ok_or(ExecutorError::ImplementationDefined {
                    detail: "aggregate value missing",
                })?;
                numeric_value(value.clone(), span)?;
                values.push(value);
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
            Self::PercentileCont { values, percentile } => {
                percentile_cont_to_value(values, percentile, span)
            }
            Self::PercentileDisc { values, percentile } => {
                percentile_disc_to_value(values, percentile, span)
            }
        }
    }
}

/// Running accumulator for `SUM` / `AVG`.
///
/// The variants form a widening lattice that mirrors `eval_arithmetic`'s
/// promotion order: `Int` (i64) widens to `Int128` (i128) on i64 overflow or
/// when mixed with a 128-bit operand; `Decimal` is the exact base-10 channel
/// for `DECIMAL` inputs (and integers mixed with them); `Float` (f64) is the
/// lossy top reached as soon as any binary float participates. 128-bit and
/// DECIMAL inputs keep `GV13`/`GV14`/`GV17` honest end-to-end (GQLRT-27).
#[derive(Clone)]
enum NumericSum {
    Int(i64),
    Int128(i128),
    Decimal(rust_decimal::Decimal),
    Float(f64),
}

impl NumericSum {
    fn into_value(self) -> Value {
        match self {
            Self::Int(value) => Value::Int(value),
            Self::Int128(value) => Value::Int128(value),
            Self::Decimal(value) => Value::Decimal(value),
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
        let count = self.count.checked_add(1).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "aggregate count is out of range",
                span,
            )
        })?;
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
    if matches!(name, "percentile_cont" | "percentile_disc") {
        if aggregate.args.len() != 2 || aggregate.distinct {
            return Err(ExecutorError::ImplementationDefined {
                detail: "PERCENTILE aggregate arity not implemented",
            });
        }
        return match name {
            "percentile_cont" => Ok(AggregateFn::PercentileCont),
            "percentile_disc" => Ok(AggregateFn::PercentileDisc),
            _ => unreachable!("matches! limited percentile names"),
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
    let ordering = value_compare::compare_non_null(&next, current_value).ok_or_else(|| {
        data_exception_value(
            DataExceptionSubclass::ValuesNotComparable,
            "aggregate value is not order-comparable",
            span,
        )
    })?;
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
        (Some(NumericSum::Int(lhs)), NumericSum::Int(rhs)) => match lhs.checked_add(rhs) {
            Some(value) => Ok(NumericSum::Int(value)),
            // i64 overflow widens to i128 rather than failing outright.
            None => add_i128(i128::from(lhs), i128::from(rhs), span),
        },
        // Integer family (i64 + i128 in any order) accumulates in i128.
        (Some(NumericSum::Int(lhs)), NumericSum::Int128(rhs))
        | (Some(NumericSum::Int128(rhs)), NumericSum::Int(lhs)) => {
            add_i128(i128::from(lhs), rhs, span)
        }
        (Some(NumericSum::Int128(lhs)), NumericSum::Int128(rhs)) => add_i128(lhs, rhs, span),
        // Decimal channel: Decimal mixed with the integer family stays exact.
        (Some(NumericSum::Decimal(lhs)), NumericSum::Decimal(rhs)) => add_decimal(lhs, rhs, span),
        (Some(NumericSum::Decimal(lhs)), NumericSum::Int(rhs))
        | (Some(NumericSum::Int(rhs)), NumericSum::Decimal(lhs)) => {
            add_decimal(lhs, rust_decimal::Decimal::from(rhs), span)
        }
        (Some(NumericSum::Decimal(lhs)), NumericSum::Int128(rhs))
        | (Some(NumericSum::Int128(rhs)), NumericSum::Decimal(lhs)) => {
            let rhs = rust_decimal::Decimal::try_from_i128_with_scale(rhs, 0).map_err(|_| {
                data_exception_value(
                    DataExceptionSubclass::NumericValueOutOfRange,
                    "128-bit aggregate value exceeds DECIMAL range",
                    span,
                )
            })?;
            add_decimal(lhs, rhs, span)
        }
        // Any float participant collapses the running sum to f64.
        (Some(lhs), rhs) => {
            let lhs = numeric_sum_to_f64(lhs, span)?;
            let rhs = numeric_sum_to_f64(rhs, span)?;
            finite_float(lhs + rhs, span).map(NumericSum::Float)
        }
    }
}

fn add_i128(lhs: i128, rhs: i128, span: SourceSpan) -> Result<NumericSum, ExecutorError> {
    lhs.checked_add(rhs).map(NumericSum::Int128).ok_or_else(|| {
        data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "integer aggregate overflow",
            span,
        )
    })
}

fn add_decimal(
    lhs: rust_decimal::Decimal,
    rhs: rust_decimal::Decimal,
    span: SourceSpan,
) -> Result<NumericSum, ExecutorError> {
    lhs.checked_add(rhs)
        .map(NumericSum::Decimal)
        .ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "decimal aggregate overflow",
                span,
            )
        })
}

fn numeric_value(value: Value, span: SourceSpan) -> Result<NumericSum, ExecutorError> {
    match value {
        Value::Int(value) => Ok(NumericSum::Int(value)),
        Value::Uint(value) => Ok(i64::try_from(value)
            .map_or_else(|_| NumericSum::Int128(i128::from(value)), NumericSum::Int)),
        Value::Int128(value) => Ok(NumericSum::Int128(value)),
        Value::Uint128(value) => i128::try_from(value).map(NumericSum::Int128).map_err(|_| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "unsigned 128-bit aggregate value is out of range",
                span,
            )
        }),
        Value::Decimal(value) => Ok(NumericSum::Decimal(value)),
        Value::Float(value) => finite_float(value, span).map(NumericSum::Float),
        Value::Float32(value) => finite_float(f64::from(value), span).map(NumericSum::Float),
        _ => Err(data_exception_value(
            DataExceptionSubclass::InvalidValueType,
            "aggregate value is not numeric",
            span,
        )),
    }
}

fn percentile_value(value: Value, span: SourceSpan) -> Result<Option<f64>, ExecutorError> {
    if matches!(value, Value::Null) {
        return Ok(None);
    }
    let percentile = numeric_sum_to_f64(numeric_value(value, span)?, span)?;
    if (0.0..=1.0).contains(&percentile) {
        Ok(Some(percentile))
    } else {
        Err(data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "PERCENTILE independent value must be between 0 and 1",
            span,
        ))
    }
}

fn numeric_sum_to_f64(value: NumericSum, span: SourceSpan) -> Result<f64, ExecutorError> {
    match value {
        NumericSum::Int(value) => i64_to_f64_exact(value).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "integer aggregate value is not exactly float-representable",
                span,
            )
        }),
        NumericSum::Int128(value) => i128_to_f64_exact(value).ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "128-bit aggregate value is not exactly float-representable",
                span,
            )
        }),
        // DECIMAL → f64 is intrinsically lossy; statistics (STDDEV / percentile)
        // and the float-collapse SUM path accept the nearest f64.
        NumericSum::Decimal(value) => value.to_f64().ok_or_else(|| {
            data_exception_value(
                DataExceptionSubclass::NumericValueOutOfRange,
                "decimal aggregate value is out of float range",
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
    // DECIMAL averages stay in the exact base-10 channel; every other numeric
    // channel divides in f64 (ISO `AVG` returns an approximate numeric).
    if let NumericSum::Decimal(sum) = sum
        && let Some(avg) = sum.checked_div(rust_decimal::Decimal::from(count))
    {
        return Ok(Value::Decimal(avg));
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

fn percentile_cont_to_value(
    mut values: Vec<f64>,
    percentile: Option<Option<f64>>,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(Some(percentile)) = percentile else {
        return Ok(Value::Null);
    };
    if values.is_empty() {
        return Ok(Value::Null);
    }
    values.sort_by(f64::total_cmp);
    if values.len() == 1 {
        return finite_float(values[0], span).map(Value::Float);
    }
    let index = 1.0 + percentile * (values.len() as f64 - 1.0);
    let floor = index.floor();
    let ceil = index.ceil();
    let floor_value = values[floor as usize - 1];
    let ceil_value = values[ceil as usize - 1];
    let value = if floor == ceil {
        floor_value
    } else {
        (ceil - index) * floor_value + (index - floor) * ceil_value
    };
    finite_float(value, span).map(Value::Float)
}

fn percentile_disc_to_value(
    mut values: Vec<Value>,
    percentile: Option<Option<f64>>,
    _span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let Some(Some(percentile)) = percentile else {
        return Ok(Value::Null);
    };
    if values.is_empty() {
        return Ok(Value::Null);
    }
    values.sort_by(|lhs, rhs| {
        value_compare::compare_non_null(lhs, rhs)
            .expect("PERCENTILE_DISC values are validated as comparable numeric values")
    });
    let index = 1.0 + percentile * (values.len() as f64 - 1.0);
    let rounded = index.round_ties_even().clamp(1.0, values.len() as f64);
    Ok(values[rounded as usize - 1].clone())
}

fn finite_float(value: f64, span: SourceSpan) -> Result<f64, ExecutorError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "floating-point aggregate produced non-finite value",
            span,
        ))
    }
}

fn count_to_value(count: u64, span: SourceSpan) -> Result<Value, ExecutorError> {
    i64::try_from(count).map(Value::Int).map_err(|_| {
        data_exception_value(
            DataExceptionSubclass::NumericValueOutOfRange,
            "aggregate count is out of range",
            span,
        )
    })
}

fn i64_to_f64_exact(value: i64) -> Option<f64> {
    u128_representable_by_binary_float(u128::from(value.unsigned_abs()), 53).then_some(value as f64)
}

fn i128_to_f64_exact(value: i128) -> Option<f64> {
    u128_representable_by_binary_float(value.unsigned_abs(), 53).then_some(value as f64)
}

fn u128_representable_by_binary_float(value: u128, significand_bits: u32) -> bool {
    if value == 0 {
        return true;
    }
    let exponent = u128::BITS - 1 - value.leading_zeros();
    if exponent < significand_bits {
        return true;
    }
    let low_bits = exponent + 1 - significand_bits;
    let mask = (1_u128 << low_bits) - 1;
    value & mask == 0
}

fn data_exception_value(
    subclass: DataExceptionSubclass,
    message: impl Into<String>,
    span: SourceSpan,
) -> ExecutorError {
    ExecutorError::data_exception(subclass, message, span)
}
