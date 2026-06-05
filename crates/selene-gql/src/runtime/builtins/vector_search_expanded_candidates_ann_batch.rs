//! `selene.vector_search_expanded_candidates_ann_batch` native built-in.
//!
//! Read-only graph-tier procedure that batches ANN-root graph expansion. Each
//! query vector finds ANN roots from one registered index, expands its roots
//! through one labelled graph hop, then exact reranks its expanded set.

use selene_core::{Value, VectorMetric};
use selene_graph::{ApproximateVectorExpansionOptions, VectorNeighborDirection};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, approximate_vector_search_error, cardinality_arg, expansion_direction_arg,
    invalid_arg, metric_arg, queries_arg, query_index_too_large, string_arg,
};
use super::vector_search_ann_defaults::{
    DEFAULT_HNSW_SEARCH_WIDTH, SEARCH_WIDTH_DEFAULT_DOC, default_search_width,
    optional_search_width_arg,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_expanded_candidates_ann_batch";

static VECTOR_SEARCH_EXPANDED_ANN_BATCH_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn::new("query_index", GqlType::Uint64)
        .with_description("Zero-based query position."),
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
        StaticParameter::new("queries", GqlType::List(Box::new(GqlType::Vector)), false)
            .with_description("Query vectors."),
        StaticParameter::new("root_k", GqlType::Integer, false)
            .with_description("Maximum ANN root count per query before graph expansion."),
        StaticParameter::new("edge_label", GqlType::String, false)
            .with_description("Edge label used to expand ANN root candidates."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum final result count per query."),
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
    VECTOR_SEARCH_EXPANDED_ANN_BATCH_OUTPUTS
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
    let queries = queries_arg(PROC_NAME, &args[2])?;
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
            queries.first().map_or(DEFAULT_HNSW_SEARCH_WIDTH, |query| {
                default_search_width(ctx.snapshot(), &label, &property, query.dimension(), metric)
            })
        });

    let batch_hits = ctx
        .snapshot()
        .approximate_vector_search_expanded_candidates_batch_checked(
            &label,
            &property,
            &queries,
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
                "batched ANN-expanded vector search",
                BatchMismatch::Internal(
                    "batched ANN-expanded vector search received candidate-set mismatch",
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
