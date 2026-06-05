//! `selene.vector_search_nodes_ann` native built-in.
//!
//! Read-only graph-tier procedure exposing approximate vector node search over
//! a registered ANN vector index. This surface is separate from
//! `selene.vector_search_nodes` so exact search remains the correctness oracle
//! and approximate recall is an explicit caller choice.

use selene_core::{Value, VectorMetric};
use selene_graph::ApproximateVectorSearchOptions;

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, approximate_vector_search_error, cardinality_arg, invalid_arg, metric_arg,
    query_arg, string_arg,
};
use super::vector_search_ann_defaults::{
    SEARCH_WIDTH_DEFAULT_DOC, default_search_width, optional_search_width_arg,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_nodes_ann";

static VECTOR_SEARCH_ANN_PARAMS: [StaticParameter; 6] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
    StaticParameter::new("k", GqlType::Integer, false).with_description("Maximum result count."),
    StaticParameter::new("metric", GqlType::String, false)
        .with_description("Distance metric.")
        .with_default_doc("squared_euclidean")
        .with_default(ProcedureDefaultValue::String("squared_euclidean")),
    StaticParameter::new("ef_search", GqlType::Integer, true)
        .with_description("ANN search-width hint; NULL uses the index-kind default.")
        .with_default_doc(SEARCH_WIDTH_DEFAULT_DOC)
        .with_default(ProcedureDefaultValue::Null),
];

static VECTOR_SEARCH_ANN_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    VECTOR_SEARCH_ANN_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    VECTOR_SEARCH_ANN_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(4..=6).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 4 to 6 arguments")));
    }

    let label = string_arg(PROC_NAME, &args[0], "label")?;
    let property = string_arg(PROC_NAME, &args[1], "property")?;
    let query = query_arg(PROC_NAME, &args[2])?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;
    let metric = args
        .get(4)
        .map(|arg| metric_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);
    let ef_search = args
        .get(5)
        .map(|value| optional_search_width_arg(PROC_NAME, value))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| {
            default_search_width(ctx.snapshot(), &label, &property, query.dimension(), metric)
        });

    let hits = ctx
        .snapshot()
        .approximate_vector_search_nodes_checked(
            &label,
            &property,
            &query,
            ApproximateVectorSearchOptions::new(metric, k, ef_search),
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            approximate_vector_search_error(
                PROC_NAME,
                error,
                "approximate vector search",
                BatchMismatch::Internal("ANN vector search received batched-only error"),
            )
        })?;
    Ok(ProcedureResult {
        rows: hits
            .into_iter()
            .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
            .collect(),
    })
}
