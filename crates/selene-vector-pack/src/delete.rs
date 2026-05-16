//! `vector.delete` mutation procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{VectorOp, VectorUpsertPayloadV1};

use crate::{
    args::{expect_arity, required_node_ref, required_string},
    provider::{reject_non_default_index, with_hnsw_provider_mut},
    state::VectorPackState,
    upsert::emit_payload,
};

static DELETE_NAME: [&str; 2] = ["vector", "delete"];
const DELETE_PROC: &str = "vector.delete";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(DeleteProcedure { state })
}

struct DeleteProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for DeleteProcedure {
    fn name(&self) -> &'static [&'static str] {
        &DELETE_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("node_id", GqlType::NodeRef, false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for DeleteProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(DELETE_PROC, args, 2)?;
        let index_name = required_string(DELETE_PROC, args, 0, "index_name")?;
        reject_non_default_index(DELETE_PROC, &index_name)?;
        let node_id = required_node_ref(DELETE_PROC, args, 1, "node_id")?;

        with_hnsw_provider_mut(ctx, DELETE_PROC, &index_name, |_provider| Ok(()))?;
        emit_payload(
            ctx,
            DELETE_PROC,
            VectorUpsertPayloadV1 {
                op: VectorOp::Delete,
                node_id,
                vector: Vec::new(),
                max_layer: 0,
            },
        )?;

        Ok(ProcedureResult { rows: Vec::new() })
    }
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}
