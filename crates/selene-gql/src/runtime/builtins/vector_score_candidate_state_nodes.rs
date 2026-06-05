//! `selene.vector_score_candidate_state_nodes` native built-in.
//!
//! Read-only graph-tier procedure that composes a named maintained candidate
//! state with explicit node candidates, then exact-reranks the composed set by a
//! vector-valued node property. The maintained set is generation-checked
//! against the statement snapshot before composition.

use selene_core::{Value, VectorMetric};
use selene_graph::{ProviderError, VectorCandidateSet};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::vector_common::{
    BatchMismatch, candidate_set_arg, cardinality_arg, invalid_arg, metric_arg, query_arg,
    string_arg, vector_search_error,
};
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, GraphContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.vector_score_candidate_state_nodes";

static VECTOR_SCORE_OUTPUTS: [StaticOutputColumn; 2] = [
    StaticOutputColumn::new("node_id", GqlType::NodeRef).with_description("Scored node id."),
    StaticOutputColumn::new("distance", GqlType::Float64)
        .with_description("Lower-is-better distance."),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateStateNodeOperation {
    Intersection,
    Union,
    StateDifference,
    NodesDifference,
}

impl CandidateStateNodeOperation {
    fn parse(raw: &str) -> Result<Self, ProcedureError> {
        match raw.to_ascii_lowercase().as_str() {
            "intersection" | "intersect" => Ok(Self::Intersection),
            "union" => Ok(Self::Union),
            "state_difference" | "state_minus_nodes" => Ok(Self::StateDifference),
            "nodes_difference" | "nodes_minus_state" => Ok(Self::NodesDifference),
            _ => Err(invalid_arg(format!(
                "{PROC_NAME} operation must be intersection, union, state_difference, or nodes_difference"
            ))),
        }
    }

    fn compose(self, state: &VectorCandidateSet, nodes: &VectorCandidateSet) -> VectorCandidateSet {
        match self {
            Self::Intersection => state.intersection(nodes),
            Self::Union => state.union(nodes),
            Self::StateDifference => state.difference(nodes),
            Self::NodesDifference => nodes.difference(state),
        }
    }
}

pub(super) fn signature() -> Vec<ProcedureParameter> {
    [
        StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
        StaticParameter::new("query", GqlType::Vector, false).with_description("Query vector."),
        StaticParameter::new("state_name", GqlType::String, false)
            .with_description("Maintained candidate-state name."),
        StaticParameter::new("nodes", GqlType::List(Box::new(GqlType::NodeRef)), false)
            .with_description("Explicit candidate nodes to compose with the maintained state."),
        StaticParameter::new("k", GqlType::Integer, false)
            .with_description("Maximum result count."),
        StaticParameter::new("operation", GqlType::String, false)
            .with_description("Candidate-set algebra operation.")
            .with_default_doc("intersection")
            .with_default(ProcedureDefaultValue::String("intersection")),
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
    VECTOR_SCORE_OUTPUTS
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
        return Err(invalid_arg(format!("{PROC_NAME} expects 5 to 7 arguments")));
    }

    let property = string_arg(PROC_NAME, &args[0], "property")?;
    let query = query_arg(PROC_NAME, &args[1])?;
    let state_name = string_arg(PROC_NAME, &args[2], "state_name")?;
    let nodes = candidate_set_arg(PROC_NAME, &args[3], "nodes")?;
    let k = cardinality_arg(PROC_NAME, &args[4], "k")?;
    let operation = args
        .get(5)
        .map(operation_arg)
        .transpose()?
        .unwrap_or(CandidateStateNodeOperation::Intersection);
    let metric = args
        .get(6)
        .map(|arg| metric_arg(PROC_NAME, arg))
        .transpose()?
        .unwrap_or(VectorMetric::SquaredEuclidean);

    let state = ctx
        .vector_candidate_set(&state_name)
        .map_err(candidate_state_error)?
        .ok_or_else(|| {
            invalid_arg(format!(
                "{PROC_NAME} unknown maintained candidate-state set '{}'",
                state_name.as_str()
            ))
        })?;
    let candidates = operation.compose(&state, &nodes);

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
                "maintained candidate-state node-composition vector scoring",
                BatchMismatch::Internal(
                    "maintained candidate-state node-composition scoring received batched-only error",
                ),
                "maintained candidate-state node-composition vector scoring",
            )
        })?;

    let rows = hits
        .into_iter()
        .map(|hit| vec![Value::NodeRef(hit.node_id), Value::Float(hit.distance)])
        .collect();
    Ok(ProcedureResult { rows })
}

fn operation_arg(value: &Value) -> Result<CandidateStateNodeOperation, ProcedureError> {
    let operation = string_arg(PROC_NAME, value, "operation")?;
    CandidateStateNodeOperation::parse(operation.as_str())
}

fn candidate_state_error(error: ProviderError) -> ProcedureError {
    ProcedureError::Internal {
        detail: format!("candidate-state provider error during {PROC_NAME}: {error}"),
    }
}
