//! `selene.vector_search_expanded_candidates_ann` native built-in.
//!
//! Read-only graph-tier procedure that uses an ANN vector index to find root
//! candidates, expands those roots through one labelled graph hop, and exact
//! reranks the expanded set by the same vector-valued node property.

use selene_core::{Value, VectorMetric};
use selene_graph::{ApproximateVectorExpansionOptions, VectorNeighborDirection};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, approximate_vector_search_error, cardinality_arg, expansion_direction_arg,
    invalid_arg, metric_arg, query_arg, string_arg,
};
use super::vector_search_ann_defaults::{
    SEARCH_WIDTH_DEFAULT_DOC, default_search_width, optional_search_width_arg,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_expanded_candidates_ann";

static VECTOR_SEARCH_EXPANDED_ANN_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef)
        .with_description("Scored expanded-candidate node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("label", GqlType::String, false)
            .with_description("ANN root node label."),
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
        StaticParameter::new("root_k", GqlType::Integer, false)
            .with_description("Maximum ANN root count before graph expansion."),
        StaticParameter::new("edge_label", GqlType::String, false)
            .with_description("Edge label used to expand ANN root candidates."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum final result count."),
        StaticParameter::new("direction", GqlType::String, false)
            .with_description("Expansion direction: outgoing, incoming, or both.")
            .with_default_doc("outgoing")
            .with_default(ProcedureDefaultValue::String("outgoing")),
        StaticParameter::new("metric", GqlType::String, false)
            .with_description("Distance metric.")
            .with_default_doc("squared_euclidean")
            .with_default(ProcedureDefaultValue::String("squared_euclidean")),
        StaticParameter::new("ef_search", GqlType::Integer, true)
            .with_description("ANN search-width hint; NULL uses the index-kind default.")
            .with_default_doc(SEARCH_WIDTH_DEFAULT_DOC)
            .with_default(ProcedureDefaultValue::Null),
    ]
    .into_iter()
    .map(StaticParameter::into_parameter)
    .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    VECTOR_SEARCH_EXPANDED_ANN_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(6..=9).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 6 to 9 arguments")));
    }

    let label = string_arg(PROC_NAME, &args[0], "label")?;
    let property = string_arg(PROC_NAME, &args[1], "property")?;
    let query = query_arg(PROC_NAME, &args[2])?;
    let root_k = cardinality_arg(PROC_NAME, &args[3], "root_k")?;
    let edge_label = string_arg(PROC_NAME, &args[4], "edge_label")?;
    let k = cardinality_arg(PROC_NAME, &args[5], "k")?;
    let direction = args
        .get(6)
        .map(|arg| expansion_direction_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorNeighborDirection::Outgoing);
    let metric = args
        .get(7)
        .map(|arg| metric_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);
    let ef_search = args
        .get(8)
        .map(|value| optional_search_width_arg(PROC_NAME, value))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| {
            default_search_width(ctx.snapshot(), &label, &property, query.dimension(), metric)
        });

    let hits = ctx
        .snapshot()
        .approximate_vector_search_expanded_candidates_checked(
            &label,
            &property,
            &query,
            ApproximateVectorExpansionOptions::new(
                &edge_label,
                direction,
                metric,
                root_k,
                k,
                ef_search,
            ),
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            approximate_vector_search_error(
                PROC_NAME,
                error,
                "ANN-expanded vector search",
                BatchMismatch::Internal("ANN-expanded vector search received batched-only error"),
            )
        })?;

    let rows = hits
        .into_iter()
        .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
        .collect();
    Ok(ProcedureResult { rows })
}
