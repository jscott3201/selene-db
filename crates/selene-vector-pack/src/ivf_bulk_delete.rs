//! `vector.ivf_bulk_delete` mutation procedure adapter.

use std::sync::Arc;

use selene_core::{CancellationChecker, Value};
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::VectorIvfBulkDeleteV1;

use crate::{
    args::{expect_arity, required_node_ref_list, required_string},
    bulk_upsert::{parameter, reject_empty_batch, validate_node_ids},
    error::check_cancellation,
    provider::{IVF_PROVIDER_NAME, emit_payload_bytes, with_ivf_provider_mut},
    state::VectorPackState,
};

static IVF_BULK_DELETE_NAME: [&str; 2] = ["vector", "ivf_bulk_delete"];
const IVF_BULK_DELETE_PROC: &str = "vector.ivf_bulk_delete";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(IvfBulkDeleteProcedure { state })
}

struct IvfBulkDeleteProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for IvfBulkDeleteProcedure {
    fn name(&self) -> &'static [&'static str] {
        &IVF_BULK_DELETE_NAME
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

impl ExternalMutationProcedure for IvfBulkDeleteProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(IVF_BULK_DELETE_PROC, args, 2)?;
        let index_name = required_string(IVF_BULK_DELETE_PROC, args, 0, "index_name")?;
        let node_ids = required_node_ref_list(IVF_BULK_DELETE_PROC, args, 1, "node_ids")?;
        reject_empty_batch(IVF_BULK_DELETE_PROC, &node_ids)?;
        check_cancellation(ctx.cancellation_checker())?;
        validate_node_ids(
            IVF_BULK_DELETE_PROC,
            &node_ids,
            CancellationChecker::disabled(),
        )?;
        with_ivf_provider_mut(ctx, IVF_BULK_DELETE_PROC, &index_name, |_provider| Ok(()))?;

        let payload = VectorIvfBulkDeleteV1 { node_ids };
        let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
            detail: format!("{IVF_BULK_DELETE_PROC}: payload encode failed: {error}"),
        })?;
        emit_payload_bytes(
            ctx,
            IVF_BULK_DELETE_PROC,
            IVF_PROVIDER_NAME,
            &index_name,
            bytes,
        )?;
        Ok(ProcedureResult { rows: Vec::new() })
    }
}
