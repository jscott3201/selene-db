//! Community algorithm procedure adapters.

use std::sync::Arc;

use selene_algorithms::{TriangleCountConfig, label_propagation, louvain, triangle_count};
use selene_core::{NodeId, Value};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, nullable_usize, required_string},
    parallel::parse_parallelism,
    state::{AlgorithmsPackState, with_algorithm_projection},
};

static LABEL_PROPAGATION_NAME: [&str; 2] = ["algo", "label_propagation"];
static LOUVAIN_NAME: [&str; 2] = ["algo", "louvain"];
static TRIANGLE_COUNT_NAME: [&str; 2] = ["algo", "triangle_count"];

const LABEL_PROPAGATION_PROC: &str = "algo.label_propagation";
const LOUVAIN_PROC: &str = "algo.louvain";
const TRIANGLE_COUNT_PROC: &str = "algo.triangle_count";

const DEFAULT_MAX_ITER_LABEL_PROPAGATION: usize = 50;
const DEFAULT_MAX_ITER_LOUVAIN: usize = 50;

pub(crate) fn procedures(state: Arc<AlgorithmsPackState>) -> Vec<Arc<dyn ExternalGraphProcedure>> {
    vec![
        Arc::new(LabelPropagationProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(LouvainProcedure {
            state: Arc::clone(&state),
        }),
        Arc::new(TriangleCountProcedure { state }),
    ]
}

struct LabelPropagationProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for LabelPropagationProcedure {
    fn name(&self) -> &'static [&'static str] {
        &LABEL_PROPAGATION_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("max_iter", GqlType::Integer, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        node_community_columns()
    }
}

impl ExternalGraphProcedure for LabelPropagationProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, max_iter) = parse_label_propagation_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = label_propagation(projection, max_iter)
                .into_iter()
                .map(node_community_row)
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct LouvainProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for LouvainProcedure {
    fn name(&self) -> &'static [&'static str] {
        &LOUVAIN_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("max_iter", GqlType::Integer, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("community", GqlType::NodeRef),
            output("level", GqlType::Uint64),
        ]
    }
}

impl ExternalGraphProcedure for LouvainProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, max_iter) = parse_louvain_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = louvain(projection, max_iter)
                .into_iter()
                .map(|(node_id, community_id, level)| {
                    vec![
                        Value::NodeRef(node_id),
                        Value::NodeRef(NodeId::new(community_id)),
                        Value::Uint(u64::from(level)),
                    ]
                })
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct TriangleCountProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for TriangleCountProcedure {
    fn name(&self) -> &'static [&'static str] {
        &TRIANGLE_COUNT_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("parallelism", GqlType::Integer, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("triangle_count", GqlType::Uint64),
        ]
    }
}

impl ExternalGraphProcedure for TriangleCountProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, config) = parse_triangle_count_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = triangle_count(projection, config)
                .into_iter()
                .map(|(node_id, count)| vec![Value::NodeRef(node_id), Value::Uint(count as u64)])
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

fn parse_label_propagation_args(args: &[Value]) -> Result<(String, usize), ProcedureError> {
    expect_arity(LABEL_PROPAGATION_PROC, args, 2)?;
    let projection_name = required_string(LABEL_PROPAGATION_PROC, args, 0, "projection_name")?;
    let max_iter = nullable_usize(
        LABEL_PROPAGATION_PROC,
        args,
        1,
        "max_iter",
        DEFAULT_MAX_ITER_LABEL_PROPAGATION,
    )?;
    Ok((projection_name, max_iter))
}

fn parse_louvain_args(args: &[Value]) -> Result<(String, usize), ProcedureError> {
    expect_arity(LOUVAIN_PROC, args, 2)?;
    let projection_name = required_string(LOUVAIN_PROC, args, 0, "projection_name")?;
    let max_iter = nullable_usize(LOUVAIN_PROC, args, 1, "max_iter", DEFAULT_MAX_ITER_LOUVAIN)?;
    Ok((projection_name, max_iter))
}

fn parse_triangle_count_args(
    args: &[Value],
) -> Result<(String, TriangleCountConfig), ProcedureError> {
    expect_arity(TRIANGLE_COUNT_PROC, args, 2)?;
    let projection_name = required_string(TRIANGLE_COUNT_PROC, args, 0, "projection_name")?;
    let parallelism = parse_parallelism(TRIANGLE_COUNT_PROC, &args[1])?;
    Ok((projection_name, TriangleCountConfig { parallelism }))
}

fn node_community_columns() -> Vec<ExternalOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("community", GqlType::NodeRef),
    ]
}

fn node_community_row((node_id, community_id): (NodeId, u64)) -> Vec<Value> {
    vec![
        Value::NodeRef(node_id),
        Value::NodeRef(NodeId::new(community_id)),
    ]
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use selene_algorithms::Parallelism;
    use selene_core::{Value, intern};
    use selene_gql::ProcedureError;

    use super::*;

    fn projection_name() -> Value {
        Value::String(intern("p").expect("test string interns"))
    }

    #[test]
    fn nullable_usize_default_applies_for_label_propagation_null() {
        let (_, max_iter) = parse_label_propagation_args(&[projection_name(), Value::Null])
            .expect("NULL max_iter parses");

        assert_eq!(max_iter, DEFAULT_MAX_ITER_LABEL_PROPAGATION);
    }

    #[test]
    fn nullable_usize_zero_means_zero_for_louvain() {
        let (_, max_iter) =
            parse_louvain_args(&[projection_name(), Value::Int(0)]).expect("zero parses");

        assert_eq!(max_iter, 0);
    }

    #[test]
    fn triangle_count_args_rejects_missing_parallelism_argument() {
        let err = parse_triangle_count_args(&[projection_name()])
            .expect_err("parallelism argument is required");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn triangle_count_args_parse_parallelism_null_zero_and_thread_count() {
        let (_, auto) = parse_triangle_count_args(&[projection_name(), Value::Null])
            .expect("NULL parallelism parses");
        let (_, sequential) = parse_triangle_count_args(&[projection_name(), Value::Int(0)])
            .expect("zero parallelism parses");
        let (_, threaded) = parse_triangle_count_args(&[projection_name(), Value::Uint(4)])
            .expect("uint parallelism parses");

        assert_eq!(auto.parallelism, Parallelism::Auto);
        assert_eq!(sequential.parallelism, Parallelism::Sequential);
        assert_eq!(
            threaded.parallelism,
            Parallelism::Threads(NonZeroUsize::new(4).unwrap())
        );
    }

    #[test]
    fn triangle_count_args_reject_negative_parallelism() {
        let err = parse_triangle_count_args(&[projection_name(), Value::Int(-1)])
            .expect_err("negative parallelism rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(detail.contains("parallelism"));
    }

    #[test]
    fn triangle_count_args_reject_parallelism_above_adapter_cap() {
        let err = parse_triangle_count_args(&[projection_name(), Value::Uint(1025)])
            .expect_err("oversized parallelism rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(detail.contains("1024"));
    }

    #[test]
    fn nullable_usize_accepts_value_uint_for_louvain_max_iter() {
        let (_, max_iter) =
            parse_louvain_args(&[projection_name(), Value::Uint(10)]).expect("uint parses");

        assert_eq!(max_iter, 10);
    }
}
