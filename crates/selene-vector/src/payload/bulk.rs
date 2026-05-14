//! Additional bulk mutation payloads for selene-vector.

use std::collections::HashSet;

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

use crate::VectorError;

/// Magic prefix for every selene-vector bulk-delete payload.
pub const PAYLOAD_MAGIC_BULK_DELETE: [u8; 4] = *b"VECD";

/// Version-1 bulk-delete vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorBulkDeletePayloadV1 {
    /// Source graph node IDs to delete, in the order they must be applied.
    pub node_ids: Vec<NodeId>,
}

impl VectorBulkDeletePayloadV1 {
    /// Encode this payload to `PAYLOAD_MAGIC_BULK_DELETE || rkyv_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the payload fails wire
    /// invariant checks. Returns [`VectorError::EncodeFailed`] when rkyv
    /// serialization fails.
    pub fn encode(&self) -> Result<Vec<u8>, VectorError> {
        self.validate()?;
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(|error| {
            VectorError::EncodeFailed {
                reason: error.to_string(),
            }
        })?;
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_BULK_DELETE.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_BULK_DELETE);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC_BULK_DELETE || rkyv_bytes` into a typed payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or per-row payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_BULK_DELETE.len()) else {
            return Err(invalid_payload(
                "bulk-delete payload shorter than magic prefix",
            ));
        };
        if magic != PAYLOAD_MAGIC_BULK_DELETE {
            return Err(invalid_payload("payload magic is not VECD"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    pub(crate) fn validate(&self) -> Result<(), VectorError> {
        if self.node_ids.is_empty() {
            return Err(invalid_payload(
                "bulk-delete payload must contain at least one node id",
            ));
        }
        let mut seen = HashSet::with_capacity(self.node_ids.len());
        for (row_index, node_id) in self.node_ids.iter().copied().enumerate() {
            if node_id == NodeId::TOMBSTONE {
                return Err(VectorError::InvalidNodeId {
                    node_id,
                    reason: format!("row {row_index}: TOMBSTONE not allowed"),
                });
            }
            if !seen.insert(node_id) {
                return Err(VectorError::DuplicateNodeId { node_id });
            }
        }
        Ok(())
    }
}

fn invalid_payload(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidPayload {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_bulk_delete_encode(payload: &VectorBulkDeletePayloadV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(payload).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_BULK_DELETE.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_BULK_DELETE);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn bulk_delete_roundtrip_preserves_node_ids() {
        let payload = VectorBulkDeletePayloadV1 {
            node_ids: vec![NodeId::new(1), NodeId::new(2), NodeId::new(3)],
        };

        let decoded = VectorBulkDeletePayloadV1::decode(&payload.encode().unwrap()).unwrap();

        assert_eq!(decoded, payload);
    }

    #[test]
    fn bulk_delete_decode_rejects_empty_archived_batch() {
        let bytes = raw_bulk_delete_encode(&VectorBulkDeletePayloadV1 {
            node_ids: Vec::new(),
        });

        let err = VectorBulkDeletePayloadV1::decode(&bytes).expect_err("empty batch rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("at least one node id")
        ));
    }

    #[test]
    fn bulk_delete_encode_rejects_duplicate_node_id() {
        let err = VectorBulkDeletePayloadV1 {
            node_ids: vec![NodeId::new(1), NodeId::new(1)],
        }
        .encode()
        .expect_err("duplicate rejected");

        assert!(matches!(
            err,
            VectorError::DuplicateNodeId { node_id } if node_id == NodeId::new(1)
        ));
    }

    #[test]
    fn bulk_delete_encode_rejects_tombstone_node_id() {
        let err = VectorBulkDeletePayloadV1 {
            node_ids: vec![NodeId::TOMBSTONE],
        }
        .encode()
        .expect_err("tombstone rejected");

        assert!(matches!(
            err,
            VectorError::InvalidNodeId { node_id, .. } if node_id == NodeId::TOMBSTONE
        ));
    }
}
