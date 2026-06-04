//! `selene.vector_score_expanded_candidates_batch` native built-in.
//!
//! Read-only graph-tier procedure that expands one root candidate set per
//! query vector through a labelled one-hop graph neighborhood, then reranks
//! each canonical expanded set by a vector-valued node property.

use std::num::TryFromIntError;

use selene_core::{CoreError, IStr, Value, VectorMetric, VectorValue};
use selene_graph::{GraphError, VectorCandidateSet, VectorNeighborDirection, VectorSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_score_expanded_candidates_batch";

static VECTOR_SCORE_EXPANDED_BATCH_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn::new("query_index", GqlType::Uint64)
        .with_description("Zero-based query position."),
    StaticOutputColumn::new("node_id", GqlType::NodeRef)
        .with_description("Scored expanded-candidate node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("queries", GqlType::List(Box::new(GqlType::Vector)), false)
            .with_description("Query vectors."),
        StaticParameter::new(
            "roots",
            GqlType::List(Box::new(GqlType::List(Box::new(GqlType::NodeRef)))),
            false,
        )
        .with_description("Per-query root candidate nodes to preserve and graph-expand."),
        StaticParameter::new("edge_label", GqlType::String, false)
            .with_description("Edge label used to expand root candidates."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count per query."),
        StaticParameter::new("direction", GqlType::String, false)
            .with_description("Expansion direction: outgoing, incoming, or both.")
            .with_default_doc("outgoing")
            .with_default(ProcedureDefaultValue::String("outgoing")),
        StaticParameter::new("metric", GqlType::String, false)
            .with_description("Distance metric.")
            .with_default_doc("squared_euclidean")
            .with_default(ProcedureDefaultValue::String("squared_euclidean")),
    ]
    .into_iter()
    .map(StaticParameter::into_parameter)
    .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    VECTOR_SCORE_EXPANDED_BATCH_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(5..=7).contains(&args.len()) {
        return Err(invalid_arg(format!(
            "{PROC_NAME} expects 5, 6, or 7 arguments"
        )));
    }

    let property = string_arg(&args[0], "property")?;
    let queries = queries_arg(&args[1])?;
    let root_sets = root_sets_arg(&args[2])?;
    if queries.len() != root_sets.len() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} queries and roots must have the same length"
        )));
    }
    let edge_label = string_arg(&args[3], "edge_label")?;
    let k = cardinality_arg(&args[4], "k")?;
    let direction = args
        .get(5)
        .map(direction_arg)
        .transpose()?
        .unwrap_or(VectorNeighborDirection::Outgoing);
    let metric = args
        .get(6)
        .map(metric_arg)
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);

    let snapshot = ctx.snapshot();
    let mut expanded_sets = Vec::with_capacity(root_sets.len());
    for roots in &root_sets {
        expanded_sets.push(
            snapshot
                .expand_vector_candidate_set_checked(
                    roots,
                    &edge_label,
                    direction,
                    ctx.cancellation_checker(),
                )
                .map_err(vector_search_error)?,
        );
    }
    let batch_hits = snapshot
        .score_vector_candidate_sets_batch_checked(
            &property,
            &queries,
            &expanded_sets,
            metric,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(vector_search_error)?;

    let mut rows = Vec::with_capacity(batch_hits.iter().map(Vec::len).sum());
    for (query_index, hits) in batch_hits.into_iter().enumerate() {
        let query_index = u64::try_from(query_index).map_err(query_index_too_large)?;
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

fn queries_arg(value: &Value) -> Result<Vec<VectorValue>, ProcedureError> {
    let Value::List(values) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} queries must be a LIST<VECTOR>"
        )));
    };
    let mut queries = Vec::with_capacity(values.len());
    let mut first_dimension = None;
    for (index, value) in values.iter().enumerate() {
        let Value::Vector(vector) = value else {
            return Err(invalid_arg(format!(
                "{PROC_NAME} queries[{index}] must be a VECTOR"
            )));
        };
        match first_dimension {
            Some(dimension) if vector.dimension() != dimension => {
                return Err(invalid_arg(format!(
                    "{PROC_NAME} queries must all have the same VECTOR dimension"
                )));
            }
            Some(_) => {}
            None => first_dimension = Some(vector.dimension()),
        }
        queries.push(vector.clone());
    }
    Ok(queries)
}

fn root_sets_arg(value: &Value) -> Result<Vec<VectorCandidateSet>, ProcedureError> {
    let Value::List(values) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} roots must be a LIST<LIST<NODE>>"
        )));
    };
    let mut root_sets = Vec::with_capacity(values.len());
    for (set_index, value) in values.iter().enumerate() {
        let Value::List(values) = value else {
            return Err(invalid_arg(format!(
                "{PROC_NAME} roots[{set_index}] must be a LIST<NODE>"
            )));
        };
        let mut roots = Vec::with_capacity(values.len());
        for (node_index, value) in values.iter().enumerate() {
            let Value::NodeRef(node_id) = value else {
                return Err(invalid_arg(format!(
                    "{PROC_NAME} roots[{set_index}][{node_index}] must be a NODE"
                )));
            };
            roots.push(*node_id);
        }
        root_sets.push(VectorCandidateSet::from_nodes(roots));
    }
    Ok(root_sets)
}

fn direction_arg(value: &Value) -> Result<VectorNeighborDirection, ProcedureError> {
    let direction = string_arg(value, "direction")?;
    let raw = direction.as_str();
    match raw.to_ascii_lowercase().as_str() {
        "outgoing" | "out" => Ok(VectorNeighborDirection::Outgoing),
        "incoming" | "in" => Ok(VectorNeighborDirection::Incoming),
        "both" | "any" => Ok(VectorNeighborDirection::Both),
        _ => Err(invalid_arg(format!(
            "unknown vector expansion direction '{raw}'; expected outgoing, incoming, or both"
        ))),
    }
}

fn cardinality_arg(value: &Value, name: &'static str) -> Result<usize, ProcedureError> {
    match value {
        Value::Int(value) if *value >= 0 => {
            usize::try_from(*value).map_err(|err| too_large(err, name))
        }
        Value::Uint(value) => usize::try_from(*value).map_err(|err| too_large(err, name)),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-negative INTEGER"
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

fn too_large(_err: TryFromIntError, name: &'static str) -> ProcedureError {
    invalid_arg(format!("{PROC_NAME} {name} is too large for this platform"))
}

fn query_index_too_large(_err: TryFromIntError) -> ProcedureError {
    invalid_arg(format!(
        "{PROC_NAME} query count is too large for this platform"
    ))
}

fn graph_error(error: GraphError) -> ProcedureError {
    match error {
        GraphError::Core(core @ CoreError::VectorDimensionMismatch { .. })
        | GraphError::Core(core @ CoreError::VectorZeroNorm { .. }) => {
            invalid_arg(format!("{core}"))
        }
        GraphError::Inconsistent { reason } => ProcedureError::Internal {
            detail: format!(
                "graph inconsistency during batched expanded vector candidate scoring: {reason}"
            ),
        },
        other => ProcedureError::Internal {
            detail: format!(
                "unexpected graph error during batched expanded vector candidate scoring: {other}"
            ),
        },
    }
}

fn vector_search_error(error: VectorSearchError) -> ProcedureError {
    match error {
        VectorSearchError::Graph(error) => graph_error(error),
        VectorSearchError::Cancelled => ProcedureError::Cancelled,
        VectorSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        VectorSearchError::BatchLengthMismatch { .. } => invalid_arg(format!("{error}")),
        VectorSearchError::ApproximateIndexMissing
        | VectorSearchError::ApproximateMetricMismatch { .. } => ProcedureError::Internal {
            detail: format!(
                "batched expanded vector candidate scoring received approximate-only error: {error}"
            ),
        },
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
