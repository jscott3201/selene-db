//! `selene.vector_search_candidate_state_expanded_ann` native built-in.
//!
//! Read-only graph-tier procedure that uses an ANN vector index to find root
//! nodes, expands those roots through one labelled graph hop, composes the
//! expanded set with a named maintained candidate-state set, then exact-reranks
//! the composed set by the same vector-valued node property.

use selene_core::{Value, VectorMetric};
use selene_graph::{ApproximateVectorSearchOptions, VectorCandidateSet, VectorNeighborDirection};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_candidate_state_common::{
    CandidateStateOperation, candidate_state_error, operation_arg,
};
use super::vector_common::{
    BatchMismatch, approximate_vector_search_error, cardinality_arg, expansion_direction_arg,
    invalid_arg, metric_arg, query_arg, string_arg, vector_search_error,
};
use super::vector_search_ann_defaults::{
    SEARCH_WIDTH_DEFAULT_DOC, default_search_width, optional_search_width_arg,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_search_candidate_state_expanded_ann";

static VECTOR_SEARCH_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef)
        .with_description("Scored composed-candidate node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("label", GqlType::String, false)
            .with_description("ANN root node label."),
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
        StaticParameter::new("state_name", GqlType::String, false)
            .with_description("Maintained candidate-state name."),
        StaticParameter::new("root_k", GqlType::Integer, false)
            .with_description("Maximum ANN root count before graph expansion."),
        StaticParameter::new("edge_label", GqlType::String, false)
            .with_description("Edge label used to expand ANN root candidates."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum final result count."),
        StaticParameter::new("operation", GqlType::String, false)
            .with_description("Candidate-set algebra operation.")
            .with_default_doc("intersection")
            .with_default(ProcedureDefaultValue::String("intersection")),
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
    if !(7..=11).contains(&args.len()) {
        return Err(invalid_arg(format!(
            "{PROC_NAME} expects 7 to 11 arguments"
        )));
    }

    let label = string_arg(PROC_NAME, &args[0], "label")?;
    let property = string_arg(PROC_NAME, &args[1], "property")?;
    let query = query_arg(PROC_NAME, &args[2])?;
    let state_name = string_arg(PROC_NAME, &args[3], "state_name")?;
    let root_k = cardinality_arg(PROC_NAME, &args[4], "root_k")?;
    let edge_label = string_arg(PROC_NAME, &args[5], "edge_label")?;
    let k = cardinality_arg(PROC_NAME, &args[6], "k")?;
    let operation = args
        .get(7)
        .map(|arg| operation_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(CandidateStateOperation::Intersection);
    let direction = args
        .get(8)
        .map(|arg| expansion_direction_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorNeighborDirection::Outgoing);
    let metric = args
        .get(9)
        .map(|arg| metric_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);
    let ef_search = args
        .get(10)
        .map(|value| optional_search_width_arg(PROC_NAME, value))
        .transpose()?
        .flatten()
        .unwrap_or_else(|| {
            default_search_width(ctx.snapshot(), &label, &property, query.dimension(), metric)
        });

    let state = ctx
        .vector_candidate_set(&state_name)
        .map_err(|error| candidate_state_error(PROC_NAME, error))?
        .ok_or_else(|| {
            invalid_arg(format!(
                "{PROC_NAME} unknown maintained candidate-state set '{}'",
                state_name.as_str()
            ))
        })?;
    let root_hits = ctx
        .snapshot()
        .approximate_vector_search_nodes_checked(
            &label,
            &property,
            &query,
            ApproximateVectorSearchOptions::new(metric, root_k, ef_search),
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            approximate_vector_search_error(
                PROC_NAME,
                error,
                "ANN candidate-state expanded vector search",
                BatchMismatch::Internal(
                    "ANN candidate-state expanded vector search received batched-only error",
                ),
            )
        })?;
    let roots = VectorCandidateSet::from_search_hits(&root_hits);
    let expanded = ctx
        .snapshot()
        .expand_vector_candidate_set_checked(
            &roots,
            &edge_label,
            direction,
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            vector_search_error(
                error,
                "ANN candidate-state expanded vector search",
                BatchMismatch::Internal(
                    "ANN candidate-state expanded vector search received batched-only error",
                ),
                "ANN candidate-state expanded vector search",
            )
        })?;
    let candidates = operation.compose(&state, &expanded);

    let hits = ctx
        .snapshot()
        .score_vector_candidate_set_checked(
            &property,
            &query,
            &candidates,
            metric,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(|error| {
            vector_search_error(
                error,
                "ANN candidate-state expanded vector search",
                BatchMismatch::Internal(
                    "ANN candidate-state expanded vector search received batched-only error",
                ),
                "ANN candidate-state expanded vector search",
            )
        })?;

    let rows = hits
        .into_iter()
        .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
        .collect();
    Ok(ProcedureResult { rows })
}
