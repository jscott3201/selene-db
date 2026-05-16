//! `vector.bulk_upsert` mutation procedure adapter.

use std::collections::HashSet;
use std::sync::Arc;

use selene_core::{NodeId, Value};
use selene_gql::{GqlType, MutationContext, ProcedureError, ProcedureResult};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{BulkInsertRow, HnswParams, VectorBulkInsertPayloadV1, random_layer_default};

use crate::{
    args::{expect_arity, required_f32_matrix, required_node_ref_list, required_string},
    error::invalid_argument,
    provider::{
        HNSW_PROVIDER_NAME, emit_payload_bytes, reject_non_default_index, with_hnsw_provider_mut,
    },
    state::VectorPackState,
};

static BULK_UPSERT_NAME: [&str; 2] = ["vector", "bulk_upsert"];
const BULK_UPSERT_PROC: &str = "vector.bulk_upsert";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(BulkUpsertProcedure { state })
}

struct BulkUpsertProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for BulkUpsertProcedure {
    fn name(&self) -> &'static [&'static str] {
        &BULK_UPSERT_NAME
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

impl ExternalMutationProcedure for BulkUpsertProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        expect_arity(BULK_UPSERT_PROC, args, 3)?;
        let index_name = required_string(BULK_UPSERT_PROC, args, 0, "index_name")?;
        reject_non_default_index(BULK_UPSERT_PROC, &index_name)?;
        let node_ids = required_node_ref_list(BULK_UPSERT_PROC, args, 1, "node_ids")?;
        let vectors = required_f32_matrix(BULK_UPSERT_PROC, args, 2, "vectors", node_ids.len())?;
        reject_empty_batch(BULK_UPSERT_PROC, &node_ids)?;
        let state = Arc::clone(&self.state);

        let max_layers = with_hnsw_provider_mut(ctx, BULK_UPSERT_PROC, &index_name, |provider| {
            validate_vectors(BULK_UPSERT_PROC, &vectors, provider.config().dim)?;
            validate_node_ids(BULK_UPSERT_PROC, &node_ids)?;
            let params = HnswParams::from_config(provider.config());
            if state.config().deterministic_seed.is_some() {
                Ok(node_ids
                    .iter()
                    .copied()
                    .map(|node_id| {
                        let mut rng = state
                            .layer_rng_for_node(node_id)
                            .expect("deterministic seed is present");
                        random_layer_default(&mut rng, params.level_factor)
                    })
                    .collect::<Vec<_>>())
            } else {
                let mut rng = fastrand::Rng::new();
                Ok((0..node_ids.len())
                    .map(|_| random_layer_default(&mut rng, params.level_factor))
                    .collect::<Vec<_>>())
            }
        })?;

        let payload = VectorBulkInsertPayloadV1 {
            rows: node_ids
                .into_iter()
                .zip(vectors)
                .zip(max_layers)
                .map(|((node_id, vector), max_layer)| BulkInsertRow {
                    node_id,
                    vector,
                    max_layer,
                })
                .collect(),
        };
        let bytes = payload.encode().map_err(|error| ProcedureError::Internal {
            detail: format!("{BULK_UPSERT_PROC}: payload encode failed: {error}"),
        })?;
        emit_payload_bytes(ctx, BULK_UPSERT_PROC, HNSW_PROVIDER_NAME, bytes)?;
        Ok(ProcedureResult { rows: Vec::new() })
    }
}

pub(crate) fn reject_empty_batch(
    procedure: &'static str,
    node_ids: &[NodeId],
) -> Result<(), ProcedureError> {
    if node_ids.is_empty() {
        return Err(invalid_argument(format!(
            "{procedure}: node_ids must contain at least one node"
        )));
    }
    Ok(())
}

pub(crate) fn validate_vectors(
    procedure: &'static str,
    vectors: &[Vec<f32>],
    dim: usize,
) -> Result<(), ProcedureError> {
    for (row_index, vector) in vectors.iter().enumerate() {
        if vector.len() != dim {
            return Err(invalid_argument(format!(
                "{procedure}: vectors[{row_index}] length {} does not match index dim {dim}",
                vector.len()
            )));
        }
        for (col_index, value) in vector.iter().copied().enumerate() {
            if !value.is_finite() {
                return Err(invalid_argument(format!(
                    "{procedure}: vectors[{row_index}][{col_index}] is NaN or infinity"
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_node_ids(
    procedure: &'static str,
    node_ids: &[NodeId],
) -> Result<(), ProcedureError> {
    let mut seen = HashSet::with_capacity(node_ids.len());
    for (row_index, node_id) in node_ids.iter().copied().enumerate() {
        if node_id == NodeId::TOMBSTONE {
            return Err(invalid_argument(format!(
                "{procedure}: node_ids[{row_index}] is TOMBSTONE"
            )));
        }
        if !seen.insert(node_id) {
            return Err(invalid_argument(format!(
                "{procedure}: duplicate node_id {node_id}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    ExternalParameter { name, ty, nullable }
}
