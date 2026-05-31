//! Native centrality procedures (`algo.pagerank`, `algo.betweenness`).
//!
//! Ported from the historical procedure-pack pagerank/betweenness adapters.
//! Argument parsing, the damping/tolerance validation, output columns, and row
//! shapes are preserved verbatim. The runners call `selene_algorithms`'
//! `*_with_checker` algorithm functions directly (through
//! [`super::state::with_projection`]) so error rendering matches the pack era.

use selene_algorithms::{
    BetweennessConfig, PageRankConfig, betweenness_with_checker, pagerank_with_checker,
};
use selene_core::{CancellationChecker, Value};
use selene_graph::SeleneGraph;

use super::args::{
    expect_arity, nullable_f64, nullable_option_usize, nullable_usize, required_string,
};
use super::error::{algorithm_aborted, invalid_argument};
use super::meta::{output, parameter};
use super::parallel::parse_parallelism;
use super::state::{AlgorithmCatalogs, with_projection};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

/// Default damping factor used when the GQL argument is NULL.
pub(super) const DEFAULT_DAMPING: f64 = 0.85;
/// Default maximum iteration count used when the GQL argument is NULL.
pub(super) const DEFAULT_MAX_ITERATIONS: usize = 100;
/// Default convergence tolerance used when the GQL argument is NULL.
pub(super) const DEFAULT_TOLERANCE: f64 = 1e-6;

const PAGERANK_PROC: &str = "algo.pagerank";
const BETWEENNESS_PROC: &str = "algo.betweenness";
const DAMPING_CONVERGENCE_DETAIL: &str = "algo.pagerank damping must be finite and in [0.0, 1.0) so PageRank keeps a positive teleport floor and retains its convergence guarantee";

pub(super) fn pagerank_signature() -> Vec<ProcedureParameter> {
    vec![
        parameter("projection_name", GqlType::String, false),
        parameter("damping", GqlType::Float, true),
        parameter("max_iterations", GqlType::Integer, true),
        parameter("tolerance", GqlType::Float, true),
        parameter("parallelism", GqlType::Integer, true),
    ]
}

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

pub(super) fn pagerank(
    catalogs: &AlgorithmCatalogs,
    snapshot: &SeleneGraph,
    args: &[Value],
    checker: CancellationChecker<'_>,
) -> Result<ProcedureResult, ProcedureError> {
    let (projection_name, config) = parse_pagerank_args(args)?;
    with_projection(catalogs, snapshot, &projection_name, |projection| {
        let rows = pagerank_with_checker(projection, config, checker)
            .map_err(algorithm_aborted)?
            .into_iter()
            .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(score)])
            .collect();
        Ok(ProcedureResult { rows })
    })
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

fn parse_pagerank_args(args: &[Value]) -> Result<(String, PageRankConfig), ProcedureError> {
    expect_arity(PAGERANK_PROC, args, 5)?;
    let projection_name = required_string(PAGERANK_PROC, args, 0, "projection_name")?;
    let damping = nullable_f64(PAGERANK_PROC, args, 1, "damping", DEFAULT_DAMPING)?;
    let max_iter = nullable_usize(
        PAGERANK_PROC,
        args,
        2,
        "max_iterations",
        DEFAULT_MAX_ITERATIONS,
    )?;
    let tolerance = nullable_f64(PAGERANK_PROC, args, 3, "tolerance", DEFAULT_TOLERANCE)?;
    let parallelism = parse_parallelism(PAGERANK_PROC, &args[4])?;
    validate_config(damping, tolerance)?;
    Ok((
        projection_name,
        PageRankConfig {
            damping,
            max_iter,
            tolerance,
            parallelism,
        },
    ))
}

fn validate_config(damping: f64, tolerance: f64) -> Result<(), ProcedureError> {
    if !damping.is_finite() || !(0.0..1.0).contains(&damping) {
        return Err(invalid_argument(DAMPING_CONVERGENCE_DETAIL));
    }
    if !tolerance.is_finite() {
        return Err(invalid_argument("algo.pagerank tolerance must be finite"));
    }
    if tolerance < 0.0 {
        return Err(invalid_argument(
            "algo.pagerank tolerance must be non-negative",
        ));
    }
    Ok(())
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
    use selene_core::{Value, intern_with_admission};

    use super::*;

    // `intern_with_admission` (not bare `intern`) keeps the runtime-path DoS
    // guard `tests/dos_guard.rs::no_unbudgeted_intern_call_in_selene_gql` green.
    fn projection_name() -> Value {
        Value::String(intern_with_admission("p").expect("test string interns").0)
    }

    fn invalid_argument_detail(err: ProcedureError) -> String {
        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        detail
    }

    // --- PageRank ---------------------------------------------------------

    #[test]
    fn null_args_resolve_to_defaults() {
        let (_, config) = parse_pagerank_args(&[
            projection_name(),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ])
        .expect("NULL args resolve");

        assert_eq!(config.damping, DEFAULT_DAMPING);
        assert_eq!(config.max_iter, DEFAULT_MAX_ITERATIONS);
        assert_eq!(config.tolerance, DEFAULT_TOLERANCE);
        assert_eq!(config.parallelism, selene_algorithms::Parallelism::Auto);
    }

    #[test]
    fn zero_max_iterations_is_valid() {
        let (_, config) = parse_pagerank_args(&[
            projection_name(),
            Value::Float(DEFAULT_DAMPING),
            Value::Int(0),
            Value::Float(DEFAULT_TOLERANCE),
            Value::Null,
        ])
        .expect("zero max_iter is accepted");

        assert_eq!(config.max_iter, 0);
    }

    #[test]
    fn pagerank_rejects_damping_one_with_clear_error() {
        let err = parse_pagerank_args(&[
            projection_name(),
            Value::Float(1.0),
            Value::Null,
            Value::Null,
            Value::Null,
        ])
        .expect_err("damping one rejected");

        let detail = invalid_argument_detail(err);
        assert!(detail.contains("[0.0, 1.0)"));
        assert!(detail.contains("teleport"));
        assert!(detail.contains("convergence guarantee"));
    }

    #[test]
    fn pagerank_rejects_damping_nan_or_inf() {
        for damping in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = parse_pagerank_args(&[
                projection_name(),
                Value::Float(damping),
                Value::Null,
                Value::Null,
                Value::Null,
            ])
            .expect_err("non-finite damping rejected");

            let detail = invalid_argument_detail(err);
            assert!(detail.contains("finite"));
            assert!(detail.contains("[0.0, 1.0)"));
            assert!(detail.contains("convergence guarantee"));
        }
    }

    #[test]
    fn out_of_range_damping_rejected() {
        let err = parse_pagerank_args(&[
            projection_name(),
            Value::Float(1.1),
            Value::Null,
            Value::Null,
            Value::Null,
        ])
        .expect_err("out-of-range damping rejected");

        let detail = invalid_argument_detail(err);
        assert!(detail.contains("[0.0, 1.0)"));
    }

    #[test]
    fn negative_tolerance_rejected() {
        let err = parse_pagerank_args(&[
            projection_name(),
            Value::Null,
            Value::Null,
            Value::Float(-0.1),
            Value::Null,
        ])
        .expect_err("negative tolerance rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    // --- Betweenness ------------------------------------------------------

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
