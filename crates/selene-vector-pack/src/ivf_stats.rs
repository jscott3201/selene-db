//! `vector.ivf_stats` procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::IvfStats;

use crate::{
    args::{expect_arity, required_string},
    error::vector_error,
    provider::{reject_non_default_index, with_ivf_provider},
    state::VectorPackState,
};

static IVF_STATS_NAME: [&str; 2] = ["vector", "ivf_stats"];
const IVF_STATS_PROC: &str = "vector.ivf_stats";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(IvfStatsProcedure { state })
}

struct IvfStatsProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for IvfStatsProcedure {
    fn name(&self) -> &'static [&'static str] {
        &IVF_STATS_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![parameter("index_name", GqlType::String, false)]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("state", GqlType::String),
            output("k_coarse", GqlType::Integer),
            output("n_probe_default", GqlType::Integer),
            output(
                "posting_list_lengths",
                GqlType::List(Box::new(GqlType::Integer)),
            ),
            output("unassigned_count", GqlType::Integer),
            output("bytes_coarse_centroids", GqlType::Integer),
            output("bytes_residual_codebook", GqlType::Integer),
            output("bytes_rotation", GqlType::Integer),
            output("bytes_posting_lists", GqlType::Integer),
            output("bytes_reconstructed_norms", GqlType::Integer),
            output("compression_ratio", GqlType::Float),
            output("polysemous", GqlType::Boolean),
            output("observed_vectors", GqlType::Integer),
            output("required", GqlType::Integer),
        ]
    }
}

impl ExternalGraphProcedure for IvfStatsProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        let index_name = parse_ivf_stats_args(args)?;
        with_ivf_provider(ctx, IVF_STATS_PROC, &index_name, |provider| {
            let rows = match provider
                .ivf_stats()
                .map_err(|err| vector_error(IVF_STATS_PROC, err))?
            {
                Some(stats) => vec![stats_row(stats)?],
                None => Vec::new(),
            };
            Ok(ProcedureResult { rows })
        })
    }
}

fn parse_ivf_stats_args(args: &[Value]) -> Result<String, ProcedureError> {
    expect_arity(IVF_STATS_PROC, args, 1)?;
    let index_name = required_string(IVF_STATS_PROC, args, 0, "index_name")?;
    reject_non_default_index(IVF_STATS_PROC, &index_name)?;
    Ok(index_name)
}

fn stats_row(stats: IvfStats) -> Result<Vec<Value>, ProcedureError> {
    match stats {
        IvfStats::Trained {
            k_coarse,
            n_probe_default,
            posting_list_lengths,
            unassigned_count,
            bytes_coarse_centroids,
            bytes_residual_codebook,
            bytes_rotation,
            bytes_posting_lists,
            bytes_reconstructed_norms,
            compression_ratio,
            polysemous,
        } => Ok(vec![
            state_value("trained"),
            Value::Int(i64::from(k_coarse)),
            Value::Int(i64::from(n_probe_default)),
            Value::List(
                posting_list_lengths
                    .into_iter()
                    .map(|value| Value::Int(i64::from(value)))
                    .collect(),
            ),
            usize_value("unassigned_count", unassigned_count)?,
            usize_value("bytes_coarse_centroids", bytes_coarse_centroids)?,
            usize_value("bytes_residual_codebook", bytes_residual_codebook)?,
            usize_value("bytes_rotation", bytes_rotation)?,
            usize_value("bytes_posting_lists", bytes_posting_lists)?,
            usize_value("bytes_reconstructed_norms", bytes_reconstructed_norms)?,
            Value::Float(f64::from(compression_ratio)),
            Value::Bool(polysemous),
            Value::Null,
            Value::Null,
        ]),
        IvfStats::Deferred {
            observed_vectors,
            required,
        } => Ok(vec![
            state_value("deferred"),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            usize_value("observed_vectors", observed_vectors)?,
            usize_value("required", required)?,
        ]),
    }
}

fn usize_value(name: &'static str, value: usize) -> Result<Value, ProcedureError> {
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| ProcedureError::Internal {
            detail: format!("{IVF_STATS_PROC}: {name} exceeds i64::MAX"),
        })
}

fn state_value(state: &'static str) -> Value {
    Value::ExternalString(Arc::<str>::from(state))
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}

#[cfg(test)]
mod tests {
    use selene_core::intern;

    use super::*;

    fn default_index() -> Value {
        Value::String(intern("default").expect("test string interns"))
    }

    #[test]
    fn parse_args_happy_path() {
        parse_ivf_stats_args(&[default_index()]).expect("args parse");
    }

    #[test]
    fn parse_args_rejects_non_default_index() {
        let err = parse_ivf_stats_args(&[Value::String(
            intern("embedding_idx").expect("test string interns"),
        )])
        .expect_err("non-default index rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn map_trained_populates_trained_fields_and_nulls_deferred_fields() {
        let row = stats_row(IvfStats::Trained {
            k_coarse: 4,
            n_probe_default: 2,
            posting_list_lengths: vec![1, 0],
            unassigned_count: 3,
            bytes_coarse_centroids: 4,
            bytes_residual_codebook: 5,
            bytes_rotation: 6,
            bytes_posting_lists: 7,
            bytes_reconstructed_norms: 8,
            compression_ratio: 1.5,
            polysemous: true,
        })
        .expect("trained row maps");

        assert_eq!(row.len(), 14);
        assert_eq!(row[0], state_value("trained"));
        assert_eq!(row[3], Value::List(vec![Value::Int(1), Value::Int(0)]));
        assert_eq!(row[12], Value::Null);
        assert_eq!(row[13], Value::Null);
    }

    #[test]
    fn map_deferred_nulls_trained_fields_and_populates_deferred_fields() {
        let row = stats_row(IvfStats::Deferred {
            observed_vectors: 7,
            required: 256,
        })
        .expect("deferred row maps");

        assert_eq!(row.len(), 14);
        assert_eq!(row[0], state_value("deferred"));
        assert!(row[1..12].iter().all(|value| *value == Value::Null));
        assert_eq!(row[12], Value::Int(7));
        assert_eq!(row[13], Value::Int(256));
    }

    #[test]
    fn map_empty_posting_lengths_emits_empty_list() {
        let row = stats_row(IvfStats::Trained {
            k_coarse: 0,
            n_probe_default: 0,
            posting_list_lengths: Vec::new(),
            unassigned_count: 0,
            bytes_coarse_centroids: 0,
            bytes_residual_codebook: 0,
            bytes_rotation: 0,
            bytes_posting_lists: 0,
            bytes_reconstructed_norms: 0,
            compression_ratio: 0.0,
            polysemous: false,
        })
        .expect("trained row maps");

        assert_eq!(row[3], Value::List(Vec::new()));
    }
}
