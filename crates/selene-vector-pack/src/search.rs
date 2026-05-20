//! `vector.search` procedure adapter.
//!
//! The final nullable `metric` argument accepts `cosine`, `l2`, or `dot` and
//! changes query-time scoring on the existing HNSW topology.

use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::Value;
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};

use crate::{
    args::{
        expect_arity, nullable_distance_metric, nullable_node_ref_list, nullable_option_usize,
        required_f32_list, required_string, required_usize, try_filter_from_node_refs,
    },
    error::vector_error,
    provider::with_hnsw_provider,
    state::VectorPackState,
};

static SEARCH_NAME: [&str; 2] = ["vector", "search"];
const SEARCH_PROC: &str = "vector.search";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(SearchProcedure { state })
}

struct SearchProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for SearchProcedure {
    fn name(&self) -> &'static [&'static str] {
        &SEARCH_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("query", GqlType::List(Box::new(GqlType::Float)), false),
            parameter("k", GqlType::Integer, false),
            parameter("ef_search", GqlType::Integer, true),
            parameter(
                "filter_nodes",
                GqlType::List(Box::new(GqlType::NodeRef)),
                true,
            ),
            parameter("metric", GqlType::String, true),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("node_id", GqlType::NodeRef),
            output("score", GqlType::Float),
        ]
    }
}

impl ExternalGraphProcedure for SearchProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        let parsed = parse_search_args(args)?;
        with_hnsw_provider(ctx, SEARCH_PROC, &parsed.index_name, |provider| {
            let rows = provider
                .search(
                    &parsed.query,
                    parsed.k,
                    parsed.ef_search,
                    parsed.filter.as_ref(),
                    parsed.metric,
                )
                .map_err(|err| vector_error(SEARCH_PROC, err))?
                .into_iter()
                .map(|(node_id, score)| {
                    vec![Value::NodeRef(node_id), Value::Float(f64::from(score))]
                })
                .collect();
            Ok(ProcedureResult { rows })
        })
    }
}

struct SearchArgs {
    index_name: String,
    query: Vec<f32>,
    k: usize,
    ef_search: Option<usize>,
    filter: Option<RoaringBitmap>,
    metric: Option<selene_vector::DistanceMetric>,
}

fn parse_search_args(args: &[Value]) -> Result<SearchArgs, ProcedureError> {
    expect_arity(SEARCH_PROC, args, 6)?;
    let index_name = required_string(SEARCH_PROC, args, 0, "index_name")?;
    let query = required_f32_list(SEARCH_PROC, args, 1, "query")?;
    let k = required_usize(SEARCH_PROC, args, 2, "k")?;
    let ef_search = nullable_option_usize(SEARCH_PROC, args, 3, "ef_search")?;
    let filter = nullable_node_ref_list(SEARCH_PROC, args, 4, "filter_nodes")?
        .as_deref()
        .map(try_filter_from_node_refs)
        .transpose()?;
    let metric = nullable_distance_metric(SEARCH_PROC, args, 5, "metric")?;
    Ok(SearchArgs {
        index_name,
        query,
        k,
        ef_search,
        filter,
        metric,
    })
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn { name, ty }
}
