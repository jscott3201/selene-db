//! `vector.bulk_delete` mutation procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::VectorBulkDeletePayloadV1;

use crate::{
    args::{expect_arity, required_node_ref_list, required_string},
    bulk_upsert::{parameter, reject_empty_batch, validate_node_ids},
    provider::{
        HNSW_PROVIDER_NAME, emit_payload_bytes, reject_non_default_index, with_hnsw_provider_mut,
    },
    state::VectorPackState,
};

static BULK_DELETE_NAME: [&str; 2] = ["vector", "bulk_delete"];
const BULK_DELETE_PROC: &str = "vector.bulk_delete";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(BulkDeleteProcedure { state })
}

struct BulkDeleteProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for BulkDeleteProcedure {
    fn name(&self) -> &'static [&'static str] {
        &BULK_DELETE_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("node_ids", GqlType::List(Box::new(GqlType::NodeRef)), false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for BulkDeleteProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(BULK_DELETE_PROC, args, 2)?;
        let index_name = required_string(BULK_DELETE_PROC, args, 0, "index_name")?;
        reject_non_default_index(BULK_DELETE_PROC, &index_name)?;
        let node_ids = required_node_ref_list(BULK_DELETE_PROC, args, 1, "node_ids")?;
        reject_empty_batch(BULK_DELETE_PROC, &node_ids)?;
        validate_node_ids(BULK_DELETE_PROC, &node_ids)?;
        with_hnsw_provider_mut(ctx, BULK_DELETE_PROC, |_provider| Ok(()))?;

        let payload = VectorBulkDeletePayloadV1 { node_ids };
        let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
            detail: format!("{BULK_DELETE_PROC}: payload encode failed: {error}"),
        })?;
        emit_payload_bytes(ctx, BULK_DELETE_PROC, HNSW_PROVIDER_NAME, bytes)?;
        Ok(ProcedureResult { rows: Vec::new() })
    }
}
