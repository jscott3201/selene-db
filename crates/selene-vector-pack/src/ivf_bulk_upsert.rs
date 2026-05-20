//! `vector.ivf_bulk_upsert` mutation procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{IvfBulkInsertRow, VectorIvfBulkInsertV1};

use crate::{
    args::{expect_arity, required_f32_matrix, required_node_ref_list, required_string},
    bulk_upsert::{parameter, reject_empty_batch, validate_node_ids, validate_vectors},
    error::check_cancellation_stride,
    provider::{IVF_PROVIDER_NAME, emit_payload_bytes, with_ivf_provider_mut},
    state::VectorPackState,
};

static IVF_BULK_UPSERT_NAME: [&str; 2] = ["vector", "ivf_bulk_upsert"];
const IVF_BULK_UPSERT_PROC: &str = "vector.ivf_bulk_upsert";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(IvfBulkUpsertProcedure { state })
}

struct IvfBulkUpsertProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for IvfBulkUpsertProcedure {
    fn name(&self) -> &'static [&'static str] {
        &IVF_BULK_UPSERT_NAME
    }

    fn description(&self) -> &'static str {
        "Bulk upsert vectors into an IVF index."
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("node_ids", GqlType::List(Box::new(GqlType::NodeRef)), false),
            parameter(
                "vectors",
                GqlType::List(Box::new(GqlType::List(Box::new(GqlType::Float)))),
                false,
            ),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for IvfBulkUpsertProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(IVF_BULK_UPSERT_PROC, args, 3)?;
        let index_name = required_string(IVF_BULK_UPSERT_PROC, args, 0, "index_name")?;
        let node_ids = required_node_ref_list(IVF_BULK_UPSERT_PROC, args, 1, "node_ids")?;
        let vectors =
            required_f32_matrix(IVF_BULK_UPSERT_PROC, args, 2, "vectors", node_ids.len())?;
        reject_empty_batch(IVF_BULK_UPSERT_PROC, &node_ids)?;
        let checker = ctx.cancellation_checker();

        with_ivf_provider_mut(ctx, IVF_BULK_UPSERT_PROC, &index_name, |provider| {
            validate_vectors(
                IVF_BULK_UPSERT_PROC,
                &vectors,
                provider.config().dim,
                checker,
            )?;
            validate_node_ids(IVF_BULK_UPSERT_PROC, &node_ids, checker)
        })?;

        let mut rows = Vec::with_capacity(node_ids.len());
        let mut rows_since_check = 0usize;
        for (node_id, vector) in node_ids.into_iter().zip(vectors) {
            check_cancellation_stride(checker, &mut rows_since_check)?;
            rows.push(IvfBulkInsertRow { node_id, vector });
        }
        let payload = VectorIvfBulkInsertV1 { rows };
        let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
            detail: format!("{IVF_BULK_UPSERT_PROC}: payload encode failed: {error}"),
        })?;
        emit_payload_bytes(
            ctx,
            IVF_BULK_UPSERT_PROC,
            IVF_PROVIDER_NAME,
            &index_name,
            bytes,
        )?;
        Ok(ProcedureResult { rows: Vec::new() })
    }
}
