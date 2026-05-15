//! Betweenness centrality procedure adapter.

use std::num::NonZeroUsize;
use std::sync::Arc;

use selene_algorithms::{BetweennessConfig, Parallelism, betweenness};
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult, Value};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{expect_arity, nullable_option_usize, required_string},
    error::invalid_argument,
    state::{AlgorithmsPackState, with_algorithm_projection},
};

static BETWEENNESS_NAME: [&str; 2] = ["algo", "betweenness"];
const BETWEENNESS_PROC: &str = "algo.betweenness";
const MAX_PARALLELISM_THREADS: usize = 1024;

pub(crate) fn procedure(state: Arc<AlgorithmsPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(BetweennessProcedure { state })
}

struct BetweennessProcedure {
    state: Arc<AlgorithmsPackState>,
}

impl ExternalProcedureMetadata for BetweennessProcedure {
    fn name(&self) -> &'static [&'static str] {
        &BETWEENNESS_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("projection_name", GqlType::String, false),
            parameter("sample_size", GqlType::Integer, true),
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

impl ExternalGraphProcedure for BetweennessProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let (projection_name, config) = parse_betweenness_args(args)?;
        with_algorithm_projection(&self.state, ctx, &projection_name, |projection| {
            let rows = betweenness(projection, config)
                .into_iter()
                .map(|(node_id, score)| vec![Value::NodeRef(node_id), Value::Float(score)])
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
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
    use selene_core::{Value, intern};
    use selene_gql::ProcedureError;

    use super::*;

    fn projection_name() -> Value {
        Value::String(intern("p").expect("test string interns"))
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
