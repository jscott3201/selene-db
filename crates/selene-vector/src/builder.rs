//! Apply vector mutation payloads against cloned HNSW snapshots.

use std::sync::Arc;

use crate::hnsw::InternalIndex;
use crate::hnsw::build::{insert_node, validate_finite};
use crate::payload::{VectorBulkInsertPayloadV1, VectorOp, VectorUpsertPayloadV1};
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

/// Apply one bulk-insert mutation to a freshly cloned graph.
///
/// # Errors
///
/// Returns [`VectorError`] when any row is malformed, duplicates another row
/// or an existing graph node, exceeds the internal index space, or disagrees
/// with the configured vector dimension.
pub(crate) fn apply_bulk_upsert(
    prev: &HnswGraph,
    payload: &VectorBulkInsertPayloadV1,
    config: &HnswConfig,
) -> Result<HnswGraph, VectorError> {
    payload.validate()?;
    for row in &payload.rows {
        if row.vector.len() != config.dim {
            return Err(VectorError::DimensionsLocked {
                expected: config.dim,
                observed: row.vector.len(),
            });
        }
    }
    ensure_bulk_capacity(prev.len(), payload.rows.len())?;
    for row in &payload.rows {
        if prev.idx_for(row.node_id).is_some() {
            return Err(VectorError::DuplicateNodeId {
                node_id: row.node_id,
            });
        }
    }

    let mut next = prev.clone_for_mutation();
    let params = HnswParams::from_config(config);
    for row in &payload.rows {
        insert_node(
            &mut next,
            row.node_id,
            Arc::from(row.vector.as_slice()),
            row.max_layer,
            &params,
        )?;
    }
    Ok(next)
}

fn ensure_bulk_capacity(current: usize, rows: usize) -> Result<(), VectorError> {
    let projected = current
        .checked_add(rows)
        .ok_or(VectorError::InternalIndexExhausted {
            current: usize::MAX,
        })?;
    if projected > InternalIndex::MAX as usize {
        return Err(VectorError::InternalIndexExhausted { current: projected });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use selene_core::NodeId;

    use crate::hnsw::build::insert_node;
    use crate::payload::BulkInsertRow;

    use super::*;

    fn config(dim: usize) -> HnswConfig {
        HnswConfig::new(dim).expect("config is valid")
    }

    fn row(raw: u64, vector: Vec<f32>) -> BulkInsertRow {
        BulkInsertRow {
            node_id: NodeId::new(raw),
            vector,
            max_layer: 0,
        }
    }

    fn graph_with_node() -> HnswGraph {
        let config = config(2);
        let mut graph = HnswGraph::empty(2);
        insert_node(
            &mut graph,
            NodeId::new(1),
            Arc::from([1.0, 0.0]),
            0,
            &HnswParams::from_config(&config),
        )
        .expect("seed insert succeeds");
        graph
    }

    #[test]
    fn apply_bulk_revalidates_directly_constructed_payload() {
        let payload = VectorBulkInsertPayloadV1 {
            rows: vec![row(2, Vec::new())],
        };

        let err = apply_bulk_upsert(&HnswGraph::empty(2), &payload, &config(2))
            .expect_err("direct invalid payload rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("non-empty")
        ));
    }

    #[test]
    fn apply_bulk_reports_dimension_before_existing_duplicate() {
        let prev = graph_with_node();
        let payload = VectorBulkInsertPayloadV1 {
            rows: vec![row(1, vec![1.0])],
        };

        let err = apply_bulk_upsert(&prev, &payload, &config(2))
            .expect_err("dimension wins before duplicate");

        assert!(matches!(
            err,
            VectorError::DimensionsLocked {
                expected: 2,
                observed: 1
            }
        ));
    }

    #[test]
    fn capacity_preflight_rejects_projected_overflow() {
        let err = ensure_bulk_capacity(InternalIndex::MAX as usize, 1)
            .expect_err("projected count overflows InternalIndex");

        assert!(matches!(
            err,
            VectorError::InternalIndexExhausted { current }
                if current == InternalIndex::MAX as usize + 1
        ));
    }
}
