//! `selene.vector_search_nodes` native built-in.
//!
//! Read-only graph-tier procedure exposing the exact vector search oracle over
//! vector-valued node properties. This is intentionally a `CALL selene.*`
//! surface rather than grammar syntax: vectors are first-class engine values,
//! while vector search remains an implementation-defined built-in procedure.

use std::num::TryFromIntError;

use selene_core::{CoreError, IStr, Value, VectorMetric, VectorValue};
use selene_graph::{GraphError, VectorSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_nodes";

static VECTOR_SEARCH_PARAMS: [StaticParameter; 5] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
    StaticParameter::new("k", GqlType::Integer, false).with_description("Maximum result count."),
    StaticParameter::new("metric", GqlType::String, false)
        .with_description("Distance metric.")
        .with_default_doc("squared_euclidean")
        .with_default(ProcedureDefaultValue::String("squared_euclidean")),
];

static VECTOR_SEARCH_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    VECTOR_SEARCH_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    VECTOR_SEARCH_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(4..=5).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 4 or 5 arguments")));
    }

    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;
    let query = vector_arg(&args[2])?;
    let k = k_arg(&args[3])?;
    let metric = args
        .get(4)
        .map(metric_arg)
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);

    let hits = ctx
        .snapshot()
        .exact_vector_search_nodes_checked(
            &label,
            &property,
            query,
            metric,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(vector_search_error)?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
            .collect(),
    })
}

fn string_arg(value: &Value, name: &'static str) -> Result<IStr, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    };
    if value.as_str().is_empty() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    }
    Ok(value.clone())
}

fn vector_arg(value: &Value) -> Result<&VectorValue, ProcedureError> {
    let Value::Vector(value) = value else {
        return Err(invalid_arg(format!("{PROC_NAME} query must be a VECTOR")));
    };
    Ok(value)
}

fn k_arg(value: &Value) -> Result<usize, ProcedureError> {
    match value {
        Value::Int(value) if *value >= 0 => usize::try_from(*value).map_err(k_too_large),
        Value::Uint(value) => usize::try_from(*value).map_err(k_too_large),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} k must be a non-negative INTEGER"
        ))),
    }
}

fn metric_arg(value: &Value) -> Result<VectorMetric, ProcedureError> {
    let metric = string_arg(value, "metric")?;
    let raw = metric.as_str();
    match raw.to_ascii_lowercase().as_str() {
        "squared_euclidean" | "sq_l2" | "l2" | "euclidean" => Ok(VectorMetric::SquaredEuclidean),
        "cosine" => Ok(VectorMetric::Cosine),
        "negative_inner_product" | "inner_product" | "mips" | "dot" => {
            Ok(VectorMetric::NegativeInnerProduct)
        }
        _ => Err(invalid_arg(format!(
            "unknown vector metric '{raw}'; expected squared_euclidean, cosine, or negative_inner_product"
        ))),
    }
}

fn k_too_large(_err: TryFromIntError) -> ProcedureError {
    invalid_arg(format!("{PROC_NAME} k is too large for this platform"))
}

fn graph_error(error: GraphError) -> ProcedureError {
    match error {
        GraphError::Core(core @ CoreError::VectorDimensionMismatch { .. })
        | GraphError::Core(core @ CoreError::VectorZeroNorm { .. }) => {
            invalid_arg(format!("{core}"))
        }
        GraphError::Inconsistent { reason } => ProcedureError::Internal {
            detail: format!("graph inconsistency during vector search: {reason}"),
        },
        other => ProcedureError::Internal {
            detail: format!("unexpected graph error during vector search: {other}"),
        },
    }
}

fn vector_search_error(error: VectorSearchError) -> ProcedureError {
    match error {
        VectorSearchError::Graph(error) => graph_error(error),
        VectorSearchError::Cancelled => ProcedureError::Cancelled,
        VectorSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
