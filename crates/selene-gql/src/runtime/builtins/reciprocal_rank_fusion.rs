//! `selene.reciprocal_rank_fusion` native built-in.
//!
//! Read-only graph-tier procedure that fuses pre-ranked node lists with
//! Reciprocal Rank Fusion. The procedure does not produce candidates itself:
//! vector, text, JSON, graph-state, and algorithm surfaces remain separate
//! policy-neutral producers.

use rust_decimal::prelude::ToPrimitive;
use selene_algorithms::{
    DEFAULT_RRF_RANK_CONSTANT, ReciprocalRankFusionError, reciprocal_rank_fusion,
};
use selene_core::Value;

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{cardinality_arg, invalid_arg, node_list_sets_arg};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.reciprocal_rank_fusion";

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new(
            "rankings",
            GqlType::List(Box::new(GqlType::List(Box::new(GqlType::NodeRef)))),
            false,
        )
        .with_description("Ranked node lists to fuse, each best-first."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum fused result count."),
        StaticParameter::new("rank_constant", GqlType::Float64, false)
            .with_description("Positive RRF rank constant.")
            .with_default_doc("60")
            .with_default(ProcedureDefaultValue::Integer(
                DEFAULT_RRF_RANK_CONSTANT as i64,
            )),
        StaticParameter::new("weights", GqlType::List(Box::new(GqlType::Float)), true)
            .with_description("Optional non-negative weight per ranking.")
            .with_default_doc("NULL (all rankings weight 1.0)")
            .with_default(ProcedureDefaultValue::Null),
    ]
    .into_iter()
    .map(StaticParameter::into_parameter)
    .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    [
        StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Fused node id."),
        StaticOutputColumn::new("score", GqlType::Float64)
            .with_description("Higher-is-better RRF score."),
    ]
    .into_iter()
    .map(StaticOutputColumn::into_output_column)
    .collect()
}

pub(super) fn execute(
    _ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(2..=4).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 2 to 4 arguments")));
    }

    let rankings = node_list_sets_arg(PROC_NAME, &args[0], "rankings")?;
    if rankings.is_empty() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} rankings must include at least one ranking"
        )));
    }
    let k = cardinality_arg(PROC_NAME, &args[1], "k")?;
    let rank_constant = args
        .get(2)
        .map(rank_constant_arg)
        .transpose()?
        .unwrap_or(DEFAULT_RRF_RANK_CONSTANT);
    let weights = args.get(3).map(weights_arg).transpose()?.flatten();

    let hits = reciprocal_rank_fusion(&rankings, weights.as_deref(), rank_constant, k)
        .map_err(rrf_error)?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.score)])
            .collect(),
    })
}

fn rank_constant_arg(value: &Value) -> Result<f64, ProcedureError> {
    let value = numeric_f64(value).ok_or_else(|| {
        invalid_arg(format!(
            "{PROC_NAME} rank_constant must be a positive finite FLOAT64"
        ))
    })?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(invalid_arg(format!(
            "{PROC_NAME} rank_constant must be a positive finite FLOAT64"
        )))
    }
}

fn weights_arg(value: &Value) -> Result<Option<Vec<f64>>, ProcedureError> {
    let Value::List(values) = value else {
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        return Err(invalid_arg(format!(
            "{PROC_NAME} weights must be NULL or a LIST<FLOAT>"
        )));
    };
    let mut weights = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Some(weight) = numeric_f64(value) else {
            return Err(invalid_arg(format!(
                "{PROC_NAME} weights[{index}] must be a non-negative finite FLOAT"
            )));
        };
        if !weight.is_finite() || weight < 0.0 {
            return Err(invalid_arg(format!(
                "{PROC_NAME} weights[{index}] must be a non-negative finite FLOAT"
            )));
        }
        weights.push(weight);
    }
    Ok(Some(weights))
}

fn numeric_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Float(value) => Some(*value),
        Value::Float32(value) => Some(f64::from(*value)),
        Value::Int(value) => Some(*value as f64),
        Value::Uint(value) => Some(*value as f64),
        Value::Decimal(value) => value.to_f64(),
        _ => None,
    }
}

fn rrf_error(error: ReciprocalRankFusionError) -> ProcedureError {
    match error {
        ReciprocalRankFusionError::InvalidRankConstant => invalid_arg(format!(
            "{PROC_NAME} rank_constant must be a positive finite FLOAT64"
        )),
        ReciprocalRankFusionError::WeightCountMismatch { rankings, weights } => invalid_arg(
            format!("{PROC_NAME} weights length {weights} must match rankings length {rankings}"),
        ),
        ReciprocalRankFusionError::InvalidWeight { index } => invalid_arg(format!(
            "{PROC_NAME} weights[{index}] must be a non-negative finite FLOAT"
        )),
        _ => invalid_arg(format!("{PROC_NAME} invalid RRF argument: {error}")),
    }
}
