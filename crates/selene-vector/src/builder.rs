//! Apply vector mutation payloads against cloned HNSW snapshots.

use std::sync::Arc;

use crate::hnsw::build::{insert_node, validate_finite};
use crate::payload::{VectorOp, VectorUpsertPayloadV1};
use crate::{HnswConfig, HnswGraph, HnswParams, VectorError};

/// Apply one vector mutation to a freshly cloned graph.
///
/// # Errors
///
/// Returns [`VectorError`] when the payload is malformed for the configured
/// graph or asks for an operation deferred beyond BRIEF-59.
pub(crate) fn apply_upsert(
    prev: &HnswGraph,
    payload: &VectorUpsertPayloadV1,
    config: &HnswConfig,
) -> Result<HnswGraph, VectorError> {
    match payload.op {
        VectorOp::Insert => apply_insert(prev, payload, config),
        VectorOp::Update | VectorOp::Delete => Err(VectorError::OperationNotSupportedYet {
            op: payload.op,
            node_id: payload.node_id,
            brief: "BRIEF-65",
        }),
    }
}

fn apply_insert(
    prev: &HnswGraph,
    payload: &VectorUpsertPayloadV1,
    config: &HnswConfig,
) -> Result<HnswGraph, VectorError> {
    if payload.vector.len() != config.dim {
        return Err(VectorError::DimensionsLocked {
            expected: config.dim,
            observed: payload.vector.len(),
        });
    }
    validate_finite(payload.node_id, &payload.vector)?;

    let mut next = prev.clone_for_mutation();
    let params = HnswParams::from_config(config);
    insert_node(
        &mut next,
        payload.node_id,
        Arc::from(payload.vector.as_slice()),
        payload.max_layer,
        &params,
    )?;
    Ok(next)
}
