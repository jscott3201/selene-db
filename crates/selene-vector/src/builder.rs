//! Apply vector mutation payloads against cloned HNSW snapshots.

use std::sync::Arc;

use crate::hnsw::InternalIndex;
use crate::hnsw::build::{insert_node, validate_finite};
use crate::hnsw::delete::{recompute_entry_point, tombstone_node, tombstone_node_no_recompute};
use crate::payload::{
    VectorBulkDeletePayloadV1, VectorBulkInsertPayloadV1, VectorOp, VectorUpsertPayloadV1,
};
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
        VectorOp::Delete => apply_delete(prev, payload),
        VectorOp::Update => Err(VectorError::OperationNotSupportedYet {
            op: payload.op,
            node_id: payload.node_id,
            brief: "future",
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

fn apply_delete(
    prev: &HnswGraph,
    payload: &VectorUpsertPayloadV1,
) -> Result<HnswGraph, VectorError> {
    let mut next = prev.clone_for_mutation();
    tombstone_node(&mut next, payload.node_id)?;
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

/// Apply one bulk-delete mutation to a freshly cloned graph.
///
/// # Errors
///
/// Returns [`VectorError`] when the payload is malformed. Missing node IDs are
/// treated as successful no-ops, matching single-row delete semantics.
pub(crate) fn apply_bulk_delete(
    prev: &HnswGraph,
    payload: &VectorBulkDeletePayloadV1,
) -> Result<HnswGraph, VectorError> {
    payload.validate()?;
    let mut next = prev.clone_for_mutation();
    let mut needs_recompute = false;
    for node_id in &payload.node_ids {
        needs_recompute |= tombstone_node_no_recompute(&mut next, *node_id)?;
    }
    if needs_recompute {
        recompute_entry_point(&mut next);
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
    use crate::hnsw::search::search;
    use crate::payload::{BulkInsertRow, VectorBulkDeletePayloadV1};

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

    fn graph_with_rows(rows: &[(u64, [f32; 2], u8)]) -> HnswGraph {
        let config = config(2);
        let params = HnswParams::from_config(&config);
        let mut graph = HnswGraph::empty(2);
        for (raw, vector, max_layer) in rows {
            insert_node(
                &mut graph,
                NodeId::new(*raw),
                Arc::from(*vector),
                *max_layer,
                &params,
            )
            .expect("seed insert succeeds");
        }
        graph
    }

    fn bulk_delete_payload(raw_ids: &[u64]) -> VectorBulkDeletePayloadV1 {
        VectorBulkDeletePayloadV1 {
            node_ids: raw_ids.iter().map(|raw| NodeId::new(*raw)).collect(),
        }
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
    fn apply_insert_reports_duplicate_node_id() {
        let prev = graph_with_node();
        let payload = VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new(1),
            vector: vec![0.0, 1.0],
            max_layer: 0,
        };

        let err =
            apply_upsert(&prev, &payload, &config(2)).expect_err("duplicate node id rejected");

        assert!(matches!(
            err,
            VectorError::DuplicateNodeId { node_id } if node_id == NodeId::new(1)
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

    #[test]
    fn hnsw_apply_bulk_delete_removes_nodes_from_search() {
        let graph = graph_with_rows(&[
            (1, [1.0, 0.0], 0),
            (2, [2.0, 0.0], 0),
            (3, [3.0, 0.0], 0),
            (4, [4.0, 0.0], 0),
            (5, [5.0, 0.0], 0),
        ]);
        let params = HnswParams::from_config(&config(2));

        let next =
            apply_bulk_delete(&graph, &bulk_delete_payload(&[2, 4, 5])).expect("delete succeeds");
        let results = search(&next, &[3.0, 0.0], 5, 16, &params, None).expect("search succeeds");

        assert_eq!(next.live_len(), 2);
        let mut result_ids = results
            .iter()
            .map(|(node_id, _)| *node_id)
            .collect::<Vec<_>>();
        result_ids.sort_unstable();
        assert_eq!(result_ids, vec![NodeId::new(1), NodeId::new(3)]);
    }

    #[test]
    fn hnsw_apply_bulk_delete_idempotent_on_missing_node_ids() {
        let graph = graph_with_rows(&[(1, [1.0, 0.0], 0), (2, [2.0, 0.0], 0), (3, [3.0, 0.0], 0)]);

        let next =
            apply_bulk_delete(&graph, &bulk_delete_payload(&[2, 8, 9])).expect("delete succeeds");

        assert_eq!(next.live_len(), 2);
        assert!(next.idx_for(NodeId::new(2)).is_none());
        assert!(next.idx_for(NodeId::new(1)).is_some());
        assert!(next.idx_for(NodeId::new(3)).is_some());
    }

    #[test]
    fn hnsw_apply_bulk_delete_recomputes_entry_point_like_repeated_delete() {
        let graph = graph_with_rows(&[
            (1, [1.0, 0.0], 3),
            (2, [2.0, 0.0], 2),
            (3, [3.0, 0.0], 1),
            (4, [4.0, 0.0], 0),
        ]);
        let mut repeated = graph.clone_for_mutation();
        tombstone_node(&mut repeated, NodeId::new(1)).expect("single delete succeeds");
        tombstone_node(&mut repeated, NodeId::new(2)).expect("single delete succeeds");

        let bulk =
            apply_bulk_delete(&graph, &bulk_delete_payload(&[1, 2])).expect("bulk delete succeeds");

        assert_eq!(bulk.entry_point(), repeated.entry_point());
        assert_eq!(bulk.max_layer(), repeated.max_layer());
        assert_eq!(bulk.live_len(), repeated.live_len());
    }

    #[test]
    fn hnsw_apply_bulk_delete_rejects_empty_batch() {
        let err = apply_bulk_delete(
            &HnswGraph::empty(2),
            &VectorBulkDeletePayloadV1 {
                node_ids: Vec::new(),
            },
        )
        .expect_err("empty delete rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("at least one node id")
        ));
    }

    #[test]
    fn hnsw_apply_bulk_delete_rejects_duplicate_in_batch() {
        let err = apply_bulk_delete(&HnswGraph::empty(2), &bulk_delete_payload(&[1, 1]))
            .expect_err("duplicate rejected");

        assert!(matches!(
            err,
            VectorError::DuplicateNodeId { node_id } if node_id == NodeId::new(1)
        ));
    }

    #[test]
    fn hnsw_apply_bulk_delete_rejects_tombstone_node_id() {
        let err = apply_bulk_delete(
            &HnswGraph::empty(2),
            &VectorBulkDeletePayloadV1 {
                node_ids: vec![NodeId::TOMBSTONE],
            },
        )
        .expect_err("tombstone rejected");

        assert!(matches!(
            err,
            VectorError::InvalidNodeId { node_id, .. } if node_id == NodeId::TOMBSTONE
        ));
    }
}
