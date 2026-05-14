//! Additional bulk mutation payloads for selene-vector.

use std::collections::HashSet;

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

use crate::VectorError;

/// Magic prefix for every selene-vector bulk-delete payload.
pub const PAYLOAD_MAGIC_BULK_DELETE: [u8; 4] = *b"VECD";

/// Magic prefix for every selene-vector IVF-PQ bulk-insert payload.
pub const PAYLOAD_MAGIC_IVF_BULK_INSERT: [u8; 4] = *b"VIVB";

/// Magic prefix for every selene-vector IVF-PQ bulk-delete payload.
pub const PAYLOAD_MAGIC_IVF_BULK_DELETE: [u8; 4] = *b"VIVD";

/// Version-1 bulk-delete vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorBulkDeletePayloadV1 {
    /// Source graph node IDs to delete, in the order they must be applied.
    pub node_ids: Vec<NodeId>,
}

/// One row in a version-1 IVF-PQ bulk-insert mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IvfBulkInsertRow {
    /// Source graph node ID.
    pub node_id: NodeId,
    /// Dense f32 vector payload.
    pub vector: Vec<f32>,
}

/// Version-1 IVF-PQ bulk-insert mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorIvfBulkInsertV1 {
    /// Rows to insert, in the order they must be applied to the IVF index.
    pub rows: Vec<IvfBulkInsertRow>,
}

/// Version-1 IVF-PQ bulk-delete mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorIvfBulkDeleteV1 {
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

impl VectorIvfBulkInsertV1 {
    /// Encode this payload to `PAYLOAD_MAGIC_IVF_BULK_INSERT || rkyv_bytes`.
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
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IVF_BULK_INSERT.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IVF_BULK_INSERT);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC_IVF_BULK_INSERT || rkyv_bytes` into a payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or per-row payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_IVF_BULK_INSERT.len())
        else {
            return Err(invalid_payload(
                "IVF bulk-insert payload shorter than magic prefix",
            ));
        };
        if magic != PAYLOAD_MAGIC_IVF_BULK_INSERT {
            return Err(invalid_payload("payload magic is not VIVB"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    pub(crate) fn validate(&self) -> Result<(), VectorError> {
        if self.rows.is_empty() {
            return Err(invalid_payload(
                "IVF bulk-insert payload must contain at least one row",
            ));
        }
        let mut seen = HashSet::with_capacity(self.rows.len());
        for (row_index, row) in self.rows.iter().enumerate() {
            if row.vector.is_empty() {
                return Err(invalid_payload(format!(
                    "row {row_index}: vector must be non-empty"
                )));
            }
            validate_node_id(row.node_id, row_index)?;
            if !seen.insert(row.node_id) {
                return Err(VectorError::DuplicateNodeId {
                    node_id: row.node_id,
                });
            }
        }
        Ok(())
    }
}

impl VectorIvfBulkDeleteV1 {
    /// Encode this payload to `PAYLOAD_MAGIC_IVF_BULK_DELETE || rkyv_bytes`.
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
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IVF_BULK_DELETE.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IVF_BULK_DELETE);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC_IVF_BULK_DELETE || rkyv_bytes` into a payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or per-row payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_IVF_BULK_DELETE.len())
        else {
            return Err(invalid_payload(
                "IVF bulk-delete payload shorter than magic prefix",
            ));
        };
        if magic != PAYLOAD_MAGIC_IVF_BULK_DELETE {
            return Err(invalid_payload("payload magic is not VIVD"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    pub(crate) fn validate(&self) -> Result<(), VectorError> {
        validate_node_ids(&self.node_ids, "IVF bulk-delete")
    }
}

fn invalid_payload(reason: impl Into<String>) -> VectorError {
    VectorError::InvalidPayload {
        reason: reason.into(),
    }
}

fn validate_node_ids(node_ids: &[NodeId], label: &str) -> Result<(), VectorError> {
    if node_ids.is_empty() {
        return Err(invalid_payload(format!(
            "{label} payload must contain at least one node id"
        )));
    }
    let mut seen = HashSet::with_capacity(node_ids.len());
    for (row_index, node_id) in node_ids.iter().copied().enumerate() {
        validate_node_id(node_id, row_index)?;
        if !seen.insert(node_id) {
            return Err(VectorError::DuplicateNodeId { node_id });
        }
    }
    Ok(())
}

fn validate_node_id(node_id: NodeId, row_index: usize) -> Result<(), VectorError> {
    if node_id == NodeId::TOMBSTONE {
        return Err(VectorError::InvalidNodeId {
            node_id,
            reason: format!("row {row_index}: TOMBSTONE not allowed"),
        });
    }
    Ok(())
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

    fn raw_ivf_bulk_insert_encode(payload: &VectorIvfBulkInsertV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(payload).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IVF_BULK_INSERT.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IVF_BULK_INSERT);
        out.extend_from_slice(&archived);
        out
    }

    fn raw_ivf_bulk_delete_encode(payload: &VectorIvfBulkDeleteV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(payload).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IVF_BULK_DELETE.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IVF_BULK_DELETE);
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

    #[test]
    fn ivf_bulk_insert_roundtrip_preserves_rows() {
        let payload = VectorIvfBulkInsertV1 {
            rows: vec![
                IvfBulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![1.0, 0.0],
                },
                IvfBulkInsertRow {
                    node_id: NodeId::new(2),
                    vector: vec![0.0, 1.0],
                },
            ],
        };

        let decoded = VectorIvfBulkInsertV1::decode(&payload.encode().unwrap()).unwrap();

        assert_eq!(decoded, payload);
    }

    #[test]
    fn ivf_bulk_insert_decode_rejects_empty_archived_batch() {
        let bytes = raw_ivf_bulk_insert_encode(&VectorIvfBulkInsertV1 { rows: Vec::new() });

        let err = VectorIvfBulkInsertV1::decode(&bytes).expect_err("empty batch rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("at least one row")
        ));
    }

    #[test]
    fn ivf_bulk_insert_encode_rejects_empty_vector() {
        let err = VectorIvfBulkInsertV1 {
            rows: vec![IvfBulkInsertRow {
                node_id: NodeId::new(1),
                vector: Vec::new(),
            }],
        }
        .encode()
        .expect_err("empty vector rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("non-empty")
        ));
    }

    #[test]
    fn ivf_bulk_insert_encode_rejects_duplicate_node_id() {
        let err = VectorIvfBulkInsertV1 {
            rows: vec![
                IvfBulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![1.0],
                },
                IvfBulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![2.0],
                },
            ],
        }
        .encode()
        .expect_err("duplicate rejected");

        assert!(matches!(
            err,
            VectorError::DuplicateNodeId { node_id } if node_id == NodeId::new(1)
        ));
    }

    #[test]
    fn ivf_bulk_delete_roundtrip_preserves_node_ids() {
        let payload = VectorIvfBulkDeleteV1 {
            node_ids: vec![NodeId::new(1), NodeId::new(2)],
        };

        let decoded = VectorIvfBulkDeleteV1::decode(&payload.encode().unwrap()).unwrap();

        assert_eq!(decoded, payload);
    }

    #[test]
    fn ivf_bulk_delete_decode_rejects_empty_archived_batch() {
        let bytes = raw_ivf_bulk_delete_encode(&VectorIvfBulkDeleteV1 {
            node_ids: Vec::new(),
        });

        let err = VectorIvfBulkDeleteV1::decode(&bytes).expect_err("empty batch rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("at least one node id")
        ));
    }

    #[test]
    fn ivf_bulk_delete_encode_rejects_tombstone_node_id() {
        let err = VectorIvfBulkDeleteV1 {
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
