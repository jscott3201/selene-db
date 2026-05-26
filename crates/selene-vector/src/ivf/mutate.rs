//! IVF-PQ mutation application helpers.

use std::sync::Arc;

use selene_core::NodeId;

use super::train::append_encoded;
use super::validate::validate_vector;
use super::{IvfIndex, RawVector};
use crate::payload::{
    PAYLOAD_MAGIC_IVF, PAYLOAD_MAGIC_IVF_BULK_DELETE, PAYLOAD_MAGIC_IVF_BULK_INSERT,
    VectorIvfBulkDeleteV1, VectorIvfBulkInsertV1, VectorIvfUpsertV1,
};
use crate::{IvfConfig, VectorError, VectorOp};

pub(crate) enum IvfEventKind {
    Upsert(VectorIvfUpsertV1),
    BulkInsert(VectorIvfBulkInsertV1),
    BulkDelete(VectorIvfBulkDeleteV1),
}

pub(crate) fn decode_ivf_event(bytes: &[u8]) -> Result<IvfEventKind, VectorError> {
    let Some((magic, _body)) = bytes.split_at_checked(4) else {
        return Err(invalid_payload(
            "IVF vector event truncated: less than 4 magic bytes",
        ));
    };
    let magic: [u8; 4] = magic
        .try_into()
        .expect("split_at_checked guarantees four magic bytes");
    match magic {
        PAYLOAD_MAGIC_IVF => VectorIvfUpsertV1::decode(bytes).map(IvfEventKind::Upsert),
        PAYLOAD_MAGIC_IVF_BULK_INSERT => {
            VectorIvfBulkInsertV1::decode(bytes).map(IvfEventKind::BulkInsert)
        }
        PAYLOAD_MAGIC_IVF_BULK_DELETE => {
            VectorIvfBulkDeleteV1::decode(bytes).map(IvfEventKind::BulkDelete)
        }
        other => Err(invalid_payload(format!(
            "unknown IVF vector event magic {}",
            String::from_utf8_lossy(&other)
        ))),
    }
}

pub(crate) fn apply_payload(
    index: &mut IvfIndex,
    payload: &VectorIvfUpsertV1,
    config: &IvfConfig,
) -> Result<(), VectorError> {
    match payload.op {
        VectorOp::Insert => {}
        VectorOp::Delete => {
            apply_delete(index, payload.node_id);
            return Ok(());
        }
        VectorOp::Update => {
            return Err(VectorError::OperationNotSupportedYet {
                op: payload.op,
                node_id: payload.node_id,
                brief: "future",
            });
        }
    }
    validate_vector(payload.node_id, &payload.vector, config)?;
    if index.contains_node(payload.node_id) {
        return Err(VectorError::DuplicateNodeId {
            node_id: payload.node_id,
        });
    }
    insert_validated(
        index,
        payload.node_id,
        Arc::from(payload.vector.as_slice()),
        config,
    )
}

pub(crate) fn apply_bulk_insert(
    index: &mut IvfIndex,
    payload: &VectorIvfBulkInsertV1,
    config: &IvfConfig,
) -> Result<(), VectorError> {
    payload.validate()?;
    for row in &payload.rows {
        validate_vector(row.node_id, &row.vector, config)?;
        if index.contains_node(row.node_id) {
            return Err(VectorError::DuplicateNodeId {
                node_id: row.node_id,
            });
        }
    }

    for row in &payload.rows {
        insert_validated(index, row.node_id, Arc::from(row.vector.as_slice()), config)?;
    }
    Ok(())
}

pub(crate) fn apply_bulk_delete(
    index: &mut IvfIndex,
    payload: &VectorIvfBulkDeleteV1,
) -> Result<(), VectorError> {
    payload.validate()?;
    for node_id in &payload.node_ids {
        apply_delete(index, *node_id);
    }
    Ok(())
}

fn insert_validated(
    index: &mut IvfIndex,
    node_id: NodeId,
    vector: Arc<[f32]>,
    config: &IvfConfig,
) -> Result<(), VectorError> {
    if index.is_trained() {
        let coarse = index
            .coarse_quantizer
            .as_deref()
            .expect("is_trained checked coarse");
        let codebook = index
            .residual_codebook
            .as_deref()
            .expect("is_trained checked codebook");
        append_encoded(
            &mut index.posting_lists,
            coarse,
            codebook,
            node_id,
            &vector,
            config,
        )?;
    } else {
        index.unassigned_buffer.push(RawVector { node_id, vector });
    }
    index.vector_count += 1;
    Ok(())
}

pub(super) fn apply_delete(index: &mut IvfIndex, node_id: NodeId) {
    let before_deferred = index.unassigned_buffer.len();
    index.unassigned_buffer.retain(|row| row.node_id != node_id);
    let removed_deferred = before_deferred - index.unassigned_buffer.len();

    let mut removed_posted = 0_usize;
    for list in &mut index.posting_lists {
        let before = list.entries.len();
        list.entries.retain(|entry| entry.node_id != node_id);
        removed_posted += before - list.entries.len();
    }

    let removed = removed_deferred + removed_posted;
    if removed > 0 {
        index.vector_count = index.vector_count.saturating_sub(removed);
    }
}

fn invalid_payload(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidPayload {
        reason: reason.into(),
    }
}
