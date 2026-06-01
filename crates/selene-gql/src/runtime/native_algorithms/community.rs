//! Native community procedures (`algo.label_propagation`, `algo.louvain`,
//! `algo.triangle_count`).
//!
//! Ported from the historical procedure-pack community adapter. Defaults,
//! argument parsing, output columns, and row shapes are preserved verbatim.

use selene_algorithms::{
    TriangleCountConfig, label_propagation_with_checker, louvain_with_checker,
    triangle_count_with_checker,
};
use selene_core::{CancellationChecker, NodeId, Value};
use selene_graph::SeleneGraph;

use super::args::{expect_arity, nullable_usize, required_string};
use super::error::algorithm_aborted;
use super::meta::{output, parameter};
use super::parallel::parse_parallelism;
use super::state::{AlgorithmCatalogs, with_projection};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const LABEL_PROPAGATION_PROC: &str = "algo.label_propagation";
const LOUVAIN_PROC: &str = "algo.louvain";
const TRIANGLE_COUNT_PROC: &str = "algo.triangle_count";

const DEFAULT_MAX_ITER_LABEL_PROPAGATION: usize = 50;
const DEFAULT_MAX_ITER_LOUVAIN: usize = 50;

pub(super) fn label_propagation_signature() -> Vec<ProcedureParameter> {
    vec![
        parameter("projection_name", GqlType::String, false),
        parameter("max_iter", GqlType::Integer, true),
    ]
}

pub(super) fn louvain_signature() -> Vec<ProcedureParameter> {
    vec![
        parameter("projection_name", GqlType::String, false),
        parameter("max_iter", GqlType::Integer, true),
    ]
}

pub(super) fn triangle_count_signature() -> Vec<ProcedureParameter> {
    vec![
        parameter("projection_name", GqlType::String, false),
        parameter("parallelism", GqlType::Integer, true),
    ]
}

pub(super) fn node_community_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("community", GqlType::NodeRef),
    ]
}

pub(super) fn louvain_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("community", GqlType::NodeRef),
        output("level", GqlType::Uint64),
    ]
}

pub(super) fn triangle_count_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("triangle_count", GqlType::Uint64),
    ]
}

pub(super) fn label_propagation(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let (projection_name, max_iter) = parse_label_propagation_args(args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = label_propagation_with_checker(projection, max_iter, checker)
            .map_err(algorithm_aborted)?
            .into_iter()
            .map(node_community_row)
            .collect();
        Ok(ProcedureResult { rows })
    })
}

pub(super) fn louvain(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let (projection_name, max_iter) = parse_louvain_args(args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = louvain_with_checker(projection, max_iter, checker)
            .map_err(algorithm_aborted)?
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

pub(super) fn triangle_count(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let (projection_name, config) = parse_triangle_count_args(args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = triangle_count_with_checker(projection, config, checker)
            .map_err(algorithm_aborted)?
            .into_iter()
            .map(|(node_id, count)| vec![Value::NodeRef(node_id), Value::Uint(count as u64)])
            .collect();
        Ok(ProcedureResult { rows })
    })
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

fn node_community_row((node_id, community_id): (NodeId, u64)) -> Vec<Value> {
    vec![
        Value::NodeRef(node_id),
        Value::NodeRef(NodeId::new(community_id)),
    ]
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use selene_algorithms::Parallelism;
    use selene_core::{Value, intern};

    use super::*;

    // `intern` (not bare `intern`) keeps the runtime-path DoS
    // guard `tests/dos_guard.rs::no_unbudgeted_intern_call_in_selene_gql` green.
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
