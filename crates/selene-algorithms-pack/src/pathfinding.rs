//! Pathfinding algorithm procedure adapters.

use std::sync::Arc;

use selene_algorithms::{apsp, dijkstra, sssp};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, required_node_ref, required_nonnegative_usize, required_string},
    error::pathfinding_error,
    state::{AlgorithmsPackState, with_algorithm_projection},
};

static DIJKSTRA_NAME: [&str; 2] = ["algo", "dijkstra"];
static SSSP_NAME: [&str; 2] = ["algo", "sssp"];
static APSP_NAME: [&str; 2] = ["algo", "apsp"];

const DIJKSTRA_PROC: &str = "algo.dijkstra";
const SSSP_PROC: &str = "algo.sssp";
const APSP_PROC: &str = "algo.apsp";

pub(crate) fn procedures(state: Arc<AlgorithmsPackState>) -> Vec<Arc<dyn ExternalGraphProcedure>> {
    vec![
        Arc::new(DijkstraProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(SsspProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(ApspProcedure { state }),
    ]
}

struct DijkstraProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for DijkstraProcedure {
    fn name(&self) -> &'static [&'static str] {
        &DIJKSTRA_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("from", GqlType::NodeRef, false),
            parameter("to", GqlType::NodeRef, false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("cost", GqlType::Float),
            output("path", GqlType::List(Box::new(GqlType::NodeRef))),
            output("length", GqlType::Uint64),
        ]
    }
}

impl ExternalGraphProcedure for DijkstraProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, from, to) = parse_dijkstra_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let Some(result) = dijkstra(projection, from, to)
                .map_err(|error| pathfinding_error(DIJKSTRA_PROC, error))?
            else {
                return Ok(ProcedureResult { rows: Vec::new() });
            };
            let length = result.nodes.len() as u64;
            let path = result.nodes.into_iter().map(Value::NodeRef).collect();
            Ok(ProcedureResult {
                rows: vec![vec![
                    Value::Float(result.cost),
                    Value::List(path),
                    Value::Uint(length),
                ]],
            })
        })
    }
}

struct SsspProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for SsspProcedure {
    fn name(&self) -> &'static [&'static str] {
        &SSSP_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("source", GqlType::NodeRef, false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("target_node", GqlType::NodeRef),
            output("cost", GqlType::Float),
        ]
    }
}

impl ExternalGraphProcedure for SsspProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, source) = parse_sssp_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = sssp(projection, source)
                .map_err(|error| pathfinding_error(SSSP_PROC, error))?
                .into_iter()
                .map(|(target_node, cost)| vec![Value::NodeRef(target_node), Value::Float(cost)])
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct ApspProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for ApspProcedure {
    fn name(&self) -> &'static [&'static str] {
        &APSP_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("max_nodes", GqlType::Integer, false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("source_node", GqlType::NodeRef),
            output("target_node", GqlType::NodeRef),
            output("cost", GqlType::Float),
        ]
    }
}

impl ExternalGraphProcedure for ApspProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, max_nodes) = parse_apsp_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = apsp(projection, max_nodes)
                .map_err(|error| pathfinding_error(APSP_PROC, error))?
                .into_iter()
                .map(|(source_node, target_node, cost)| {
                    vec![
                        Value::NodeRef(source_node),
                        Value::NodeRef(target_node),
                        Value::Float(cost),
                    ]
                })
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

fn parse_dijkstra_args(
    args: &[Value],
) -> Result<(String, selene_core::NodeId, selene_core::NodeId), ProcedureError> {
    expect_arity(DIJKSTRA_PROC, args, 3)?;
    let projection_name = required_string(DIJKSTRA_PROC, args, 0, "projection_name")?;
    let from = required_node_ref(DIJKSTRA_PROC, args, 1, "from")?;
    let to = required_node_ref(DIJKSTRA_PROC, args, 2, "to")?;
    Ok((projection_name, from, to))
}

fn parse_sssp_args(args: &[Value]) -> Result<(String, selene_core::NodeId), ProcedureError> {
    expect_arity(SSSP_PROC, args, 2)?;
    let projection_name = required_string(SSSP_PROC, args, 0, "projection_name")?;
    let source = required_node_ref(SSSP_PROC, args, 1, "source")?;
    Ok((projection_name, source))
}

fn parse_apsp_args(args: &[Value]) -> Result<(String, usize), ProcedureError> {
    expect_arity(APSP_PROC, args, 2)?;
    let projection_name = required_string(APSP_PROC, args, 0, "projection_name")?;
    let max_nodes = required_nonnegative_usize(APSP_PROC, args, 1, "max_nodes")?;
    Ok((projection_name, max_nodes))
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}

#[cfg(test)]
mod tests {
    use selene_core::{NodeId, Value, intern};

    use super::*;

    fn projection_name() -> Value {
        Value::String(intern("p").expect("test string interns"))
    }

    #[test]
    fn dijkstra_args_accept_projection_and_node_refs() {
        let (projection, from, to) = parse_dijkstra_args(&[
            projection_name(),
            Value::NodeRef(NodeId::new(1)),
            Value::NodeRef(NodeId::new(2)),
        ])
        .expect("dijkstra args parse");

        assert_eq!(projection, "p");
        assert_eq!(from, NodeId::new(1));
        assert_eq!(to, NodeId::new(2));
    }

    #[test]
    fn sssp_args_reject_integer_source() {
        let err = parse_sssp_args(&[projection_name(), Value::Int(1)])
            .expect_err("source must be NodeRef");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn apsp_args_reject_negative_max_nodes() {
        let err =
            parse_apsp_args(&[projection_name(), Value::Int(-1)]).expect_err("negative rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }
}
