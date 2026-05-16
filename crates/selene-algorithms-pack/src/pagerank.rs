//! PageRank procedure adapter.

use std::sync::Arc;

use selene_algorithms::{AlgorithmsError, PageRankConfig, pagerank};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, nullable_f64, nullable_usize, required_string},
    error::{algorithm_error, invalid_argument},
    parallel::parse_parallelism,
    state::AlgorithmsPackState,
};

/// Default damping factor used when the GQL argument is NULL.
pub const DEFAULT_DAMPING: f64 = 0.85;
/// Default maximum iteration count used when the GQL argument is NULL.
pub const DEFAULT_MAX_ITERATIONS: usize = 100;
/// Default convergence tolerance used when the GQL argument is NULL.
pub const DEFAULT_TOLERANCE: f64 = 1e-6;

static PAGERANK_NAME: [&str; 2] = ["algo", "pagerank"];
const PAGERANK_PROC: &str = "algo.pagerank";
const DAMPING_CONVERGENCE_DETAIL: &str = "algo.pagerank damping must be finite and in [0.0, 1.0) so PageRank keeps a positive teleport floor and retains its convergence guarantee";

pub(crate) fn procedure(state: Arc<AlgorithmsPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(PageRankProcedure { state })
}

struct PageRankProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for PageRankProcedure {
    fn name(&self) -> &'static [&'static str] {
        &PAGERANK_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("damping", GqlType::Float, true),
            parameter("max_iterations", GqlType::Integer, true),
            parameter("tolerance", GqlType::Float, true),
            parameter("parallelism", GqlType::Integer, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("score", GqlType::Float),
        ]
    }
}

impl ExternalGraphProcedure for PageRankProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, config) = parse_pagerank_args(args)?;
        let graph_id = ctx.snapshot().graph_id();
        self.state.with_catalog(graph_id, |catalog| {
            catalog
                .ensure_fresh(ctx.snapshot(), &projection_name)
                .map_err(algorithm_error)?;
            let projection = catalog.get(&projection_name).ok_or_else(|| {
                algorithm_error(AlgorithmsError::NoSuchProjection {
                    name: projection_name.clone(),
                })
            })?;
            let rows = pagerank(projection.projection(), config)
                .into_iter()
                .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(score)])
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

pub(crate) fn parse_pagerank_args(
    args: &[Value],
) -> Result<(String, PageRankConfig), ProcedureError> {
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

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}

#[cfg(test)]
mod tests {
    use selene_core::{Value, intern};

    use super::*;

    fn projection_name() -> Value {
        Value::String(intern("p").expect("test string interns"))
    }

    fn invalid_argument_detail(err: ProcedureError) -> String {
        let ProcedureError::InvalidArgument { detail } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        detail
    }

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
}
