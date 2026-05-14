//! `vector.ivf_search` procedure adapter.

use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::Value;
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{
        expect_arity, nullable_node_ref_list, nullable_option_u32, required_f32_list,
        required_string, required_usize, try_filter_from_node_refs,
    },
    error::vector_error,
    provider::{reject_non_default_index, with_ivf_provider},
    state::VectorPackState,
};

static IVF_SEARCH_NAME: [&str; 2] = ["vector", "ivf_search"];
const IVF_SEARCH_PROC: &str = "vector.ivf_search";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(IvfSearchProcedure { state })
}

struct IvfSearchProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for IvfSearchProcedure {
    fn name(&self) -> &'static [&'static str] {
        &IVF_SEARCH_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("query", GqlType::List(Box::new(GqlType::Float)), false),
            parameter("k", GqlType::Integer, false),
            parameter("n_probe", GqlType::Integer, true),
            parameter(
                "filter_nodes",
                GqlType::List(Box::new(GqlType::NodeRef)),
                true,
            ),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("score", GqlType::Float),
        ]
    }
}

impl ExternalGraphProcedure for IvfSearchProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        let parsed = parse_ivf_search_args(args)?;
        with_ivf_provider(ctx, IVF_SEARCH_PROC, |provider| {
            let rows = provider
                .search(
                    &parsed.query,
                    parsed.k,
                    parsed.n_probe,
                    parsed.filter.as_ref(),
                )
                .map_err(|err| vector_error(IVF_SEARCH_PROC, err))?
                .into_iter()
                .map(|(node_id, score)| {
                    vec![Value::NodeRef(node_id), Value::Float(f64::from(score))]
                })
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

#[derive(Debug)]
struct IvfSearchArgs {
    query: Vec<f32>,
    k: usize,
    n_probe: Option<u32>,
    filter: Option<RoaringBitmap>,
}

fn parse_ivf_search_args(args: &[Value]) -> Result<IvfSearchArgs, ProcedureError> {
    expect_arity(IVF_SEARCH_PROC, args, 5)?;
    let index_name = required_string(IVF_SEARCH_PROC, args, 0, "index_name")?;
    reject_non_default_index(IVF_SEARCH_PROC, &index_name)?;
    let query = required_f32_list(IVF_SEARCH_PROC, args, 1, "query")?;
    let k = required_usize(IVF_SEARCH_PROC, args, 2, "k")?;
    let n_probe = nullable_option_u32(IVF_SEARCH_PROC, args, 3, "n_probe")?;
    let filter = nullable_node_ref_list(IVF_SEARCH_PROC, args, 4, "filter_nodes")?
        .as_deref()
        .map(try_filter_from_node_refs)
        .transpose()?;
    Ok(IvfSearchArgs {
        query,
        k,
        n_probe,
        filter,
    })
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}

#[cfg(test)]
mod tests {
    use selene_core::{NodeId, intern};

    use super::*;

    fn default_index() -> Value {
        Value::String(intern("default").expect("test string interns"))
    }

    #[test]
    fn parse_args_happy_path() {
        let parsed = parse_ivf_search_args(&[
            default_index(),
            Value::List(vec![Value::Float(1.0), Value::Float(0.0)]),
            Value::Int(10),
            Value::Int(2),
            Value::List(vec![Value::NodeRef(NodeId::new(1))]),
        ])
        .expect("args parse");

        assert_eq!(parsed.query, vec![1.0, 0.0]);
        assert_eq!(parsed.k, 10);
        assert_eq!(parsed.n_probe, Some(2));
        assert!(
            parsed
                .filter
                .as_ref()
                .is_some_and(|filter| filter.contains(1))
        );
    }

    #[test]
    fn parse_args_rejects_non_default_index() {
        let err = parse_ivf_search_args(&[
            Value::String(intern("embedding_idx").expect("test string interns")),
            Value::List(vec![Value::Float(1.0)]),
            Value::Int(1),
            Value::Null,
            Value::Null,
        ])
        .expect_err("non-default index rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }

    #[test]
    fn parse_args_rejects_wrong_arity() {
        let err = parse_ivf_search_args(&[default_index()]).expect_err("arity rejected");

        assert!(matches!(err, ProcedureError::InvalidArgument { .. }));
    }
}
