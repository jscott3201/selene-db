//! Pathfinding algorithm procedure adapters.

use std::num::NonZeroUsize;
use std::sync::Arc;

use selene_algorithms::{ApspConfig, Parallelism, apsp, dijkstra, sssp};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, required_node_ref, required_nonnegative_usize, required_string},
    error::{invalid_argument, pathfinding_error},
    state::{AlgorithmsPackState, with_algorithm_projection},
};

static DIJKSTRA_NAME: [&str; 2] = ["algo", "dijkstra"];
static SSSP_NAME: [&str; 2] = ["algo", "sssp"];
static APSP_NAME: [&str; 2] = ["algo", "apsp"];

const DIJKSTRA_PROC: &str = "algo.dijkstra";
const SSSP_PROC: &str = "algo.sssp";
const APSP_PROC: &str = "algo.apsp";
const MAX_PARALLELISM_THREADS: usize = 1024;

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
            parameter("parallelism", GqlType::Integer, true),
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
        let (projection_name, config) = parse_apsp_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = apsp(projection, config)
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

fn parse_apsp_args(args: &[Value]) -> Result<(String, ApspConfig), ProcedureError> {
    expect_arity(APSP_PROC, args, 3)?;
    let projection_name = required_string(APSP_PROC, args, 0, "projection_name")?;
    let max_nodes = required_nonnegative_usize(APSP_PROC, args, 1, "max_nodes")?;
    let parallelism = parse_parallelism(APSP_PROC, &args[2])?;
    Ok((
        projection_name,
        ApspConfig {
            max_nodes,
            parallelism,
        },
    ))
}

fn parse_parallelism(
    procedure: &'static str,
    value: &Value,
) -> Result<Parallelism, ProcedureError> {
    match value {
        Value::Null => Ok(Parallelism::Auto),
        Value::Int(0) => Ok(Parallelism::Sequential),
        Value::Int(value) if *value > 0 => {
            let threads = usize::try_from(*value)
                .map_err(|_| invalid_argument(format!("{procedure}: parallelism is too large")))?;
            threads_parallelism(procedure, threads)
        }
        Value::Int(_) => Err(invalid_argument(format!(
            "{procedure}: parallelism must be NULL, 0, or a positive thread count"
        ))),
        Value::Uint(0) => Ok(Parallelism::Sequential),
        Value::Uint(value) => {
            let threads = usize::try_from(*value)
                .map_err(|_| invalid_argument(format!("{procedure}: parallelism is too large")))?;
            threads_parallelism(procedure, threads)
        }
        other => Err(invalid_argument(format!(
            "{procedure}: expected parallelism to be INTEGER or NULL, got {other:?}"
        ))),
    }
}

fn threads_parallelism(
    procedure: &'static str,
    threads: usize,
) -> Result<Parallelism, ProcedureError> {
    if threads > MAX_PARALLELISM_THREADS {
        return Err(invalid_argument(format!(
            "{procedure}: parallelism exceeds adapter-side cap of {MAX_PARALLELISM_THREADS} threads"
        )));
    }
    Ok(Parallelism::Threads(
        NonZeroUsize::new(threads).expect("positive thread count"),
    ))
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
        let err = parse_apsp_args(&[projection_name(), Value::Int(-1), Value::Null])
            .expect_err("negative rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn apsp_args_accept_unsigned_max_nodes() {
        let (projection, config) =
            parse_apsp_args(&[projection_name(), Value::Uint(12), Value::Null])
                .expect("unsigned max_nodes parses");

        assert_eq!(projection, "p");
        assert_eq!(config.max_nodes, 12);
        assert_eq!(config.parallelism, Parallelism::Auto);
    }

    #[test]
    fn apsp_args_parse_parallelism_null_zero_and_thread_count() {
        let (_, auto) = parse_apsp_args(&[projection_name(), Value::Int(12), Value::Null])
            .expect("NULL parallelism parses");
        let (_, sequential) = parse_apsp_args(&[projection_name(), Value::Int(12), Value::Int(0)])
            .expect("zero parallelism parses");
        let (_, threaded) = parse_apsp_args(&[projection_name(), Value::Int(12), Value::Uint(4)])
            .expect("uint parallelism parses");

        assert_eq!(auto.parallelism, Parallelism::Auto);
        assert_eq!(sequential.parallelism, Parallelism::Sequential);
        assert_eq!(
            threaded.parallelism,
            Parallelism::Threads(NonZeroUsize::new(4).unwrap())
        );
    }

    #[test]
    fn apsp_args_reject_negative_parallelism() {
        let err = parse_apsp_args(&[projection_name(), Value::Int(12), Value::Int(-1)])
            .expect_err("negative parallelism rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(detail.contains("parallelism"));
    }

    #[test]
    fn apsp_args_reject_parallelism_above_adapter_cap() {
        let err = parse_apsp_args(&[projection_name(), Value::Int(12), Value::Uint(1025)])
            .expect_err("oversized parallelism rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(detail.contains("1024"));
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn apsp_args_reject_unsigned_max_nodes_overflow() {
        let err = parse_apsp_args(&[projection_name(), Value::Uint(u64::MAX), Value::Null])
            .expect_err("oversized unsigned max_nodes rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail, "algo.apsp: max_nodes is too large");
    }
}
