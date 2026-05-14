//! `vector.upsert` mutation procedure adapter.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{HnswParams, VectorOp, VectorUpsertPayloadV1, random_layer_default};

use crate::{
    args::{expect_arity, required_f32_list, required_node_ref, required_string},
    error::invalid_argument,
    provider::{
        HNSW_PROVIDER_NAME, emit_payload_bytes, reject_non_default_index, with_hnsw_provider_mut,
    },
    state::VectorPackState,
};

static UPSERT_NAME: [&str; 2] = ["vector", "upsert"];
const UPSERT_PROC: &str = "vector.upsert";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(UpsertProcedure { state })
}

struct UpsertProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for UpsertProcedure {
    fn name(&self) -> &'static [&'static str] {
        &UPSERT_NAME
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("index_name", GqlType::String, false),
            parameter("node_id", GqlType::NodeRef, false),
            parameter("vector", GqlType::List(Box::new(GqlType::Float)), false),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for UpsertProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(UPSERT_PROC, args, 3)?;
        let index_name = required_string(UPSERT_PROC, args, 0, "index_name")?;
        reject_non_default_index(UPSERT_PROC, &index_name)?;
        let node_id = required_node_ref(UPSERT_PROC, args, 1, "node_id")?;
        let vector = required_f32_list(UPSERT_PROC, args, 2, "vector")?;

        let max_layer = with_hnsw_provider_mut(ctx, UPSERT_PROC, |provider| {
            let dim = provider.config().dim;
            if vector.len() != dim {
                return Err(invalid_argument(format!(
                    "{UPSERT_PROC}: vector length {} does not match index dim {dim}",
                    vector.len()
                )));
            }
            for (index, value) in vector.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err(invalid_argument(format!(
                        "{UPSERT_PROC}: vector[{index}] is NaN or infinity"
                    )));
                }
            }
            let params = HnswParams::from_config(provider.config());
            Ok(random_layer_default(params.level_factor))
        })?;

        let payload = VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id,
            vector,
            max_layer,
        };
        emit_payload(ctx, UPSERT_PROC, payload)?;
        Ok(ProcedureResult { rows: Vec::new() })
    }
}

pub(crate) fn emit_payload(
    ctx: &mut MutationContext<'_, '_>,
    procedure: &'static str,
    payload: VectorUpsertPayloadV1,
) -> Result<(), ProcedureError> {
    let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
        detail: format!("{procedure}: payload encode failed: {error}"),
    })?;
    emit_payload_bytes(ctx, procedure, HNSW_PROVIDER_NAME, bytes)
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}
