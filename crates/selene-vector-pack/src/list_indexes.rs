//! `vector.list_indexes` graph procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, GraphContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{DistanceMetric, VectorIndexKind};

use crate::{args::expect_arity, provider::catalog_from_graph, state::VectorPackState};

static LIST_INDEXES_NAME: [&str; 2] = ["vector", "list_indexes"];
const LIST_INDEXES_PROC: &str = "vector.list_indexes";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalGraphProcedure> {
    Arc::new(ListIndexesProcedure { state })
}

struct ListIndexesProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for ListIndexesProcedure {
    fn name(&self) -> &'static [&'static str] {
        &LIST_INDEXES_NAME
    }

    fn description(&self) -> &'static str {
        "List registered vector indexes."
    }

    fn since_version(&self) -> &'static str {
        "1.1.0"
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        Vec::new()
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        vec![
            output("name", GqlType::String),
            output("kind", GqlType::String),
            output("dim", GqlType::Integer),
            output("metric", GqlType::String),
            output("vector_count", GqlType::Integer),
        ]
    }
}

impl ExternalGraphProcedure for ListIndexesProcedure {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(LIST_INDEXES_PROC, args, 0)?;
        let catalog = catalog_from_graph(ctx, LIST_INDEXES_PROC)?;
        let rows = catalog
            .list_indexes()
            .into_iter()
            .map(|entry| {
                vec![
                    Value::ExternalString(entry.name),
                    Value::ExternalString(kind_name(entry.kind).into()),
                    Value::Int(entry.dim as i64),
                    Value::ExternalString(metric_name(entry.metric).into()),
                    Value::Int(entry.vector_count as i64),
                ]
            })
            .collect();
        Ok(ProcedureResult { rows })
    }
}

fn kind_name(kind: VectorIndexKind) -> &'static str {
    match kind {
        VectorIndexKind::Hnsw => "hnsw",
        VectorIndexKind::Ivf => "ivf",
    }
}

fn metric_name(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::L2 => "l2",
        DistanceMetric::Dot => "dot",
        _ => "unknown",
    }
}

fn output(name: &'static str, ty: GqlType) -> ExternalOutputColumn {
    ExternalOutputColumn::new(name, ty).with_description("Procedure output column.")
}
