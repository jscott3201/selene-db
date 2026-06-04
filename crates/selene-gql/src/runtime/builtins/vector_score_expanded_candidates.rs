//! `selene.vector_score_expanded_candidates` native built-in.
//!
//! Read-only graph-tier procedure that treats an explicit `LIST<NODE>` as root
//! candidates, expands that set through one labelled graph hop, and scores the
//! canonical expanded set by a vector-valued node property. This is the native
//! GQL entry point for graph-augmented vector reranking without adding
//! non-standard syntax.

use std::num::TryFromIntError;

use selene_core::{CoreError, IStr, Value, VectorMetric, VectorValue};
use selene_graph::{GraphError, VectorCandidateSet, VectorNeighborDirection, VectorSearchError};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_score_expanded_candidates";

static VECTOR_SCORE_EXPANDED_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef)
        .with_description("Scored expanded-candidate node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
        StaticParameter::new("roots", GqlType::List(Box::new(GqlType::NodeRef)), false)
            .with_description("Root candidate nodes to preserve and graph-expand."),
        StaticParameter::new("edge_label", GqlType::String, false)
            .with_description("Edge label used to expand root candidates."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count."),
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
    VECTOR_SCORE_EXPANDED_OUTPUTS
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
    let query = query_arg(&args[1])?;
    let roots = roots_arg(&args[2])?;
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

    let expanded = ctx
        .snapshot()
        .expand_vector_candidate_set_checked(
            &roots,
            &edge_label,
            direction,
            ctx.cancellation_checker(),
        )
        .map_err(vector_search_error)?;
    let hits = ctx
        .snapshot()
        .score_vector_candidate_set_checked(
            &property,
            &query,
            &expanded,
            metric,
            k,
            ctx.cancellation_checker(),
        )
        .map_err(vector_search_error)?;

    let rows = hits
        .into_iter()
        .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
        .collect();
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

fn query_arg(value: &Value) -> Result<VectorValue, ProcedureError> {
    let Value::Vector(query) = value else {
        return Err(invalid_arg(format!("{PROC_NAME} query must be a VECTOR")));
    };
    Ok(query.clone())
}

fn roots_arg(value: &Value) -> Result<VectorCandidateSet, ProcedureError> {
    let Value::List(values) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} roots must be a LIST<NODE>"
        )));
    };
    let mut roots = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Value::NodeRef(node_id) = value else {
            return Err(invalid_arg(format!(
                "{PROC_NAME} roots[{index}] must be a NODE"
            )));
        };
        roots.push(*node_id);
    }
    Ok(VectorCandidateSet::from_nodes(roots))
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

fn graph_error(error: GraphError) -> ProcedureError {
    match error {
        GraphError::Core(core @ CoreError::VectorDimensionMismatch { .. })
        | GraphError::Core(core @ CoreError::VectorZeroNorm { .. }) => {
            invalid_arg(format!("{core}"))
        }
        GraphError::Inconsistent { reason } => ProcedureError::Internal {
            detail: format!(
                "graph inconsistency during expanded vector candidate scoring: {reason}"
            ),
        },
        other => ProcedureError::Internal {
            detail: format!(
                "unexpected graph error during expanded vector candidate scoring: {other}"
            ),
        },
    }
}

fn vector_search_error(error: VectorSearchError) -> ProcedureError {
    match error {
        VectorSearchError::Graph(error) => graph_error(error),
        VectorSearchError::Cancelled => ProcedureError::Cancelled,
        VectorSearchError::Timeout { elapsed } => ProcedureError::Timeout { elapsed },
        VectorSearchError::BatchLengthMismatch { .. } => ProcedureError::Internal {
            detail: format!(
                "expanded vector candidate scoring received batched-only error: {error}"
            ),
        },
        VectorSearchError::ApproximateIndexMissing
        | VectorSearchError::ApproximateMetricMismatch { .. } => ProcedureError::Internal {
            detail: format!(
                "expanded vector candidate scoring received approximate-only error: {error}"
            ),
        },
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
