//! Native centrality procedures (`algo.pagerank`, `algo.betweenness`).
//!
//! Ported from the historical procedure-pack pagerank/betweenness adapters.
//! The runners call `selene_algorithms`' `*_with_checker` algorithm functions
//! directly through [`super::state::with_projection`].

mod pagerank;

pub(super) use pagerank::{pagerank, pagerank_signature};

use selene_algorithms::{BetweennessConfig, betweenness_with_checker};
use selene_core::{CancellationChecker, Value};
use selene_graph::SeleneGraph;

use super::args::{expect_arity, nullable_option_usize, required_string};
use super::error::algorithm_aborted;
use super::meta::{output, parameter};
use super::parallel::parse_parallelism;
use super::state::{AlgorithmCatalogs, with_projection};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const BETWEENNESS_PROC: &str = "algo.betweenness";

pub(super) fn betweenness_signature() -> Vec<ProcedureParameter> {
    vec![
        parameter("projection_name", GqlType::String, false),
        parameter("sample_size", GqlType::Integer, true),
        parameter("parallelism", GqlType::Integer, true),
    ]
}

pub(super) fn node_score_columns() -> Vec<ProcedureOutputColumn> {
    vec![
        output("node_id", GqlType::NodeRef),
        output("score", GqlType::Float),
    ]
}

pub(super) fn betweenness(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let (projection_name, config) = parse_betweenness_args(args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = betweenness_with_checker(projection, config, checker)
            .map_err(algorithm_aborted)?
            .into_iter()
            .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(score)])
            .collect();
        Ok(ProcedureResult { rows })
    })
}

fn parse_betweenness_args(args: &[Value]) -> Result<(String, BetweennessConfig), ProcedureError> {
    expect_arity(BETWEENNESS_PROC, args, 3)?;
    let projection_name = required_string(BETWEENNESS_PROC, args, 0, "projection_name")?;
    let sample_size = nullable_option_usize(BETWEENNESS_PROC, args, 1, "sample_size")?;
    let parallelism = parse_parallelism(BETWEENNESS_PROC, &args[2])?;
    Ok((
        projection_name,
        BetweennessConfig {
            sample_size,
            parallelism,
        },
    ))
}

#[cfg(test)]
mod tests {
    use selene_core::{Value, db_string};

    use super::*;

    fn projection_name() -> Value {
        Value::String(db_string("p").expect("test string fits DB string cap"))
    }

    #[test]
    fn nullable_option_usize_returns_none_for_value_null() {
        let (_, config) = parse_betweenness_args(&[projection_name(), Value::Null, Value::Null])
            .expect("NULL parses");

        assert_eq!(config.sample_size, None);
    }

    #[test]
    fn nullable_option_usize_returns_some_zero_for_value_int_zero() {
        let (_, config) = parse_betweenness_args(&[projection_name(), Value::Int(0), Value::Null])
            .expect("zero parses");

        assert_eq!(config.sample_size, Some(0));
    }

    #[test]
    fn nullable_option_usize_returns_some_value_for_positive_int() {
        let (_, config) = parse_betweenness_args(&[projection_name(), Value::Int(5), Value::Null])
            .expect("value parses");

        assert_eq!(config.sample_size, Some(5));
    }

    #[test]
    fn nullable_option_usize_rejects_negative_int_with_non_negative_detail() {
        let err = parse_betweenness_args(&[projection_name(), Value::Int(-1), Value::Null])
            .expect_err("negative sample_size rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail, "algo.betweenness: sample_size must be non-negative");
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn nullable_option_usize_rejects_u64_max_with_too_large_detail() {
        let err = parse_betweenness_args(&[projection_name(), Value::Uint(u64::MAX), Value::Null])
            .expect_err("oversized unsigned sample_size rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail, "algo.betweenness: sample_size is too large");
    }

    #[test]
    fn nullable_option_usize_accepts_value_uint_on_all_targets() {
        let (_, config) =
            parse_betweenness_args(&[projection_name(), Value::Uint(10), Value::Null])
                .expect("uint parses");

        assert_eq!(config.sample_size, Some(10));
    }

    #[test]
    fn nullable_option_usize_rejects_non_integer_with_integer_or_null_detail() {
        let err = parse_betweenness_args(&[projection_name(), Value::Bool(true), Value::Null])
            .expect_err("bool sample_size rejected");

        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(detail.contains("INTEGER or NULL"));
    }
}
