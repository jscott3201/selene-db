//! `selene.vector_search_nodes_ann_batch` native built-in.
//!
//! Read-only graph-tier procedure exposing batched approximate vector node
//! search over one registered ANN vector index. The procedure accepts
//! `LIST<VECTOR>` query parameters and emits a `query_index` column so callers
//! can regroup hits without issuing one `CALL` per embedding.

use selene_core::{Value, VectorMetric};
use selene_graph::ApproximateVectorSearchOptions;

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, approximate_vector_search_error, cardinality_arg, invalid_arg, queries_arg,
    query_index_too_large, string_arg,
};
use super::vector_search_ann_defaults::{
    ANN_METRIC_DEFAULT_DOC, DEFAULT_HNSW_SEARCH_WIDTH, SEARCH_WIDTH_DEFAULT_DOC, default_metric,
    default_search_width, optional_metric_arg, optional_search_width_arg,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_nodes_ann_batch";

static VECTOR_SEARCH_ANN_BATCH_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn::new("query_index", GqlType::Uint64)
        .with_description("Zero-based query position."),
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Matched node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("queries", GqlType::List(Box::new(GqlType::Vector)), false)
            .with_description("Query vectors."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count per query."),
        StaticParameter::new("metric", GqlType::String, true)
            .with_description(
                "Distance metric; NULL uses the matching index metric when available.",
            )
            .with_default_doc(ANN_METRIC_DEFAULT_DOC)
            .with_default(ProcedureDefaultValue::Null),
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
    VECTOR_SEARCH_ANN_BATCH_OUTPUTS
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
    let queries = queries_arg(PROC_NAME, &args[2])?;
    let k = cardinality_arg(PROC_NAME, &args[3], "k")?;
    let metric = args
        .get(4)
        .map(|arg| optional_metric_arg(PROC_NAME, arg))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| {
            queries
                .first()
                .map_or(VectorMetric::SquaredEuclidean, |query| {
                    default_metric(ctx.snapshot(), &label, &property, query.dimension())
                })
        });
    let ef_search = args
        .get(5)
        .map(|value| optional_search_width_arg(PROC_NAME, value))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| {
            queries.first().map_or(DEFAULT_HNSW_SEARCH_WIDTH, |query| {
                default_search_width(ctx.snapshot(), &label, &property, query.dimension(), metric)
            })
        });

    let batch_hits = ctx
        .snapshot()
        .approximate_vector_search_nodes_batch_checked(
            &label,
            &property,
            &queries,
            ApproximateVectorSearchOptions::new(metric, k, ef_search),
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            approximate_vector_search_error(
                PROC_NAME,
                error,
                "batched approximate vector search",
                BatchMismatch::Internal(
                    "batched ANN vector search received candidate-scoring error",
                ),
            )
        })?;

    let mut rows = Vec::with_capacity(batch_hits.iter().map(Vec::len).sum());
    for (query_index, hits) in batch_hits.into_iter().enumerate() {
        let query_index =
            u64::try_from(query_index).map_err(|err| query_index_too_large(PROC_NAME, err))?;
        for hit in hits {
            rows.push(vec![
                Value::Uint(query_index),
                Value::NodeRef(hit.node_id),
                Value::Float(hit.distance),
            ]);
        }
    }
    Ok(ProcedureResult { rows })
}
