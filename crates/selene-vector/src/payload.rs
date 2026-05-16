//! Wire format for selene-vector mutation events.

use std::collections::HashSet;

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

use crate::VectorError;

mod bulk;
pub use bulk::{
    IvfBulkInsertRow, PAYLOAD_MAGIC_BULK_DELETE, PAYLOAD_MAGIC_IVF_BULK_DELETE,
    PAYLOAD_MAGIC_IVF_BULK_INSERT, VectorBulkDeletePayloadV1, VectorIvfBulkDeleteV1,
    VectorIvfBulkInsertV1,
};

const MAX_LAYER: u8 = 32;

/// Magic prefix for every selene-vector mutation event payload.
pub const PAYLOAD_MAGIC: [u8; 4] = *b"VECU";

/// Magic prefix for every selene-vector bulk-insert payload.
pub const PAYLOAD_MAGIC_BULK: [u8; 4] = *b"VECB";

/// Magic prefix for every selene-vector IVF-PQ insert payload.
pub const PAYLOAD_MAGIC_IVF: [u8; 4] = *b"VIVF";

/// Vector mutation operation stored in version-1 vector mutation payloads.
#[derive(Archive, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum VectorOp {
    /// Insert a fresh vector for a source graph node.
    Insert = 0,
    /// Replace an existing vector. Reserved for a future mutation API.
    Update = 1,
    /// Delete an indexed vector.
    Delete = 2,
}

/// Version-1 vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorUpsertPayloadV1 {
    /// Requested vector mutation operation.
    pub op: VectorOp,
    /// Source graph node ID.
    pub node_id: NodeId,
    /// Dense f32 vector payload. Empty only for [`VectorOp::Delete`].
    pub vector: Vec<f32>,
    /// Persisted insertion layer, sampled by the writer before WAL emission.
    pub max_layer: u8,
}

impl VectorUpsertPayloadV1 {
    /// Encode this payload to `PAYLOAD_MAGIC || rkyv_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the payload fails wire
    /// invariant checks (max_layer cap, operation/vector shape). Returns
    /// [`VectorError::EncodeFailed`] when rkyv serialization fails. Validation
    /// runs BEFORE serialization so producers cannot emit syntactically valid
    /// but semantically invalid WAL bytes that the decoder would later reject.
    pub fn encode(&self) -> Result<Vec<u8>, VectorError> {
        self.validate()?;
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(|error| {
            VectorError::EncodeFailed {
                reason: error.to_string(),
            }
        })?;
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC || rkyv_bytes` into a typed payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or operation-specific payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC.len()) else {
            return Err(invalid_payload("payload shorter than magic prefix"));
        };
        if magic != PAYLOAD_MAGIC {
            return Err(invalid_payload("payload magic is not VECU"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    fn validate(&self) -> Result<(), VectorError> {
        if self.max_layer > MAX_LAYER {
            return Err(invalid_payload(format!(
                "max_layer {} exceeds cap {MAX_LAYER}",
                self.max_layer
            )));
        }
        match self.op {
            VectorOp::Insert | VectorOp::Update if self.vector.is_empty() => Err(invalid_payload(
                format!("{:?} requires a non-empty vector", self.op),
            )),
            VectorOp::Delete if !self.vector.is_empty() => {
                Err(invalid_payload("Delete payload must not include a vector"))
            }
            _ => Ok(()),
        }
    }
}

/// One row in a version-1 bulk-insert vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BulkInsertRow {
    /// Source graph node ID.
    pub node_id: NodeId,
    /// Dense f32 vector payload.
    pub vector: Vec<f32>,
    /// Persisted insertion layer, sampled by the writer before WAL emission.
    pub max_layer: u8,
}

/// Version-1 bulk-insert vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorBulkInsertPayloadV1 {
    /// Rows to insert, in the order they must be applied to the HNSW graph.
    pub rows: Vec<BulkInsertRow>,
}

/// Version-1 IVF-PQ vector mutation payload.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VectorIvfUpsertV1 {
    /// Requested vector mutation operation.
    pub op: VectorOp,
    /// Source graph node ID.
    pub node_id: NodeId,
    /// Dense f32 vector payload.
    pub vector: Vec<f32>,
}

impl VectorIvfUpsertV1 {
    /// Encode this payload to `PAYLOAD_MAGIC_IVF || rkyv_bytes`.
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
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IVF.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IVF);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC_IVF || rkyv_bytes` into a typed payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or operation-specific payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_IVF.len()) else {
            return Err(invalid_payload("IVF payload shorter than magic prefix"));
        };
        if magic != PAYLOAD_MAGIC_IVF {
            return Err(invalid_payload("payload magic is not VIVF"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    pub(crate) fn validate(&self) -> Result<(), VectorError> {
        match self.op {
            VectorOp::Insert | VectorOp::Update if self.vector.is_empty() => Err(invalid_payload(
                format!("{:?} requires a non-empty vector", self.op),
            )),
            VectorOp::Delete if !self.vector.is_empty() => {
                Err(invalid_payload("Delete payload must not include a vector"))
            }
            _ => Ok(()),
        }
    }
}

impl VectorBulkInsertPayloadV1 {
    /// Encode this payload to `PAYLOAD_MAGIC_BULK || rkyv_bytes`.
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
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_BULK.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_BULK);
        out.extend_from_slice(&archived);
        Ok(out)
    }

    /// Decode `PAYLOAD_MAGIC_BULK || rkyv_bytes` into a typed payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the magic, archive bytes,
    /// or per-row payload shape is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_BULK.len()) else {
            return Err(invalid_payload("bulk payload shorter than magic prefix"));
        };
        if magic != PAYLOAD_MAGIC_BULK {
            return Err(invalid_payload("payload magic is not VECB"));
        }
        let payload = rkyv::from_bytes::<Self, rkyv::rancor::Error>(body)
            .map_err(|error| invalid_payload(format!("rkyv decode failed: {error}")))?;
        payload.validate()?;
        Ok(payload)
    }

    pub(crate) fn validate(&self) -> Result<(), VectorError> {
        if self.rows.is_empty() {
            return Err(invalid_payload(
                "bulk-insert payload must contain at least one row",
            ));
        }
        let mut seen = HashSet::with_capacity(self.rows.len());
        for (row_index, row) in self.rows.iter().enumerate() {
            if row.vector.is_empty() {
                return Err(invalid_payload(format!(
                    "row {row_index}: vector must be non-empty"
                )));
            }
            if row.node_id == NodeId::TOMBSTONE {
                return Err(VectorError::InvalidNodeId {
                    node_id: row.node_id,
                    reason: format!("row {row_index}: TOMBSTONE not allowed"),
                });
            }
            if row.max_layer > MAX_LAYER {
                return Err(VectorError::MaxLayerExceedsCap {
                    observed: row.max_layer,
                    cap: MAX_LAYER,
                });
            }
            for (index, value) in row.vector.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err(VectorError::NonFiniteVectorComponent {
                        node_id: row.node_id,
                        index,
                        value,
                    });
                }
            }
            if !seen.insert(row.node_id) {
                return Err(VectorError::DuplicateNodeId {
                    node_id: row.node_id,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum EventKind {
    Upsert(VectorUpsertPayloadV1),
    Bulk(VectorBulkInsertPayloadV1),
    BulkDelete(VectorBulkDeletePayloadV1),
    IvfBulkInsert,
    IvfBulkDelete,
}

pub(crate) fn decode_event(bytes: &[u8]) -> Result<EventKind, VectorError> {
    let Some((magic, _body)) = bytes.split_at_checked(4) else {
        return Err(invalid_payload(
            "vector event truncated: less than 4 magic bytes",
        ));
    };
    let magic: [u8; 4] = magic
        .try_into()
        .expect("split_at_checked guarantees four magic bytes");
    match magic {
        PAYLOAD_MAGIC => VectorUpsertPayloadV1::decode(bytes).map(EventKind::Upsert),
        PAYLOAD_MAGIC_BULK => VectorBulkInsertPayloadV1::decode(bytes).map(EventKind::Bulk),
        PAYLOAD_MAGIC_BULK_DELETE => {
            VectorBulkDeletePayloadV1::decode(bytes).map(EventKind::BulkDelete)
        }
        PAYLOAD_MAGIC_IVF_BULK_INSERT => {
            VectorIvfBulkInsertV1::decode(bytes).map(|_| EventKind::IvfBulkInsert)
        }
        PAYLOAD_MAGIC_IVF_BULK_DELETE => {
            VectorIvfBulkDeleteV1::decode(bytes).map(|_| EventKind::IvfBulkDelete)
        }
        other => Err(invalid_payload(format!(
            "unknown vector event magic {}",
            String::from_utf8_lossy(&other)
        ))),
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
    use selene_core::NodeId;

    /// Encode without running the producer-side validator. Lets unit tests pin
    /// the decode-side validator independently of the encode pre-check.
    fn raw_encode(payload: &VectorUpsertPayloadV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(payload).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC);
        out.extend_from_slice(&archived);
        out
    }

    fn raw_bulk_encode(payload: &VectorBulkInsertPayloadV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(payload).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_BULK.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_BULK);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn decode_rejects_max_layer_above_32() {
        let bytes = raw_encode(&VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new(1),
            vector: vec![1.0],
            max_layer: 33,
        });

        let err = VectorUpsertPayloadV1::decode(&bytes).expect_err("oversized layer rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("max_layer")
        ));
    }

    #[test]
    fn decode_rejects_insert_with_empty_vector() {
        let bytes = raw_encode(&VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new(1),
            vector: Vec::new(),
            max_layer: 0,
        });

        let err = VectorUpsertPayloadV1::decode(&bytes).expect_err("empty Insert rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("Insert")
        ));
    }

    #[test]
    fn decode_rejects_delete_with_vector() {
        let bytes = raw_encode(&VectorUpsertPayloadV1 {
            op: VectorOp::Delete,
            node_id: NodeId::new(1),
            vector: vec![1.0],
            max_layer: 0,
        });

        let err = VectorUpsertPayloadV1::decode(&bytes).expect_err("Delete with vector rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("Delete")
        ));
    }

    #[test]
    fn bulk_roundtrip_preserves_rows() {
        let payload = VectorBulkInsertPayloadV1 {
            rows: vec![
                BulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![1.0, 0.0],
                    max_layer: 0,
                },
                BulkInsertRow {
                    node_id: NodeId::new(2),
                    vector: vec![0.0, 1.0],
                    max_layer: 1,
                },
            ],
        };

        let decoded = VectorBulkInsertPayloadV1::decode(&payload.encode().unwrap()).unwrap();

        assert_eq!(decoded, payload);
    }

    #[test]
    fn bulk_decode_rejects_empty_archived_batch() {
        let bytes = raw_bulk_encode(&VectorBulkInsertPayloadV1 { rows: Vec::new() });
        let err = VectorBulkInsertPayloadV1::decode(&bytes).expect_err("empty batch rejected");

        assert!(matches!(
            err,
            VectorError::InvalidPayload { reason } if reason.contains("at least one row")
        ));
    }

    #[test]
    fn bulk_encode_rejects_empty_vector() {
        let err = VectorBulkInsertPayloadV1 {
            rows: vec![BulkInsertRow {
                node_id: NodeId::new(1),
                vector: Vec::new(),
                max_layer: 0,
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
    fn bulk_encode_rejects_tombstone_row() {
        let err = VectorBulkInsertPayloadV1 {
            rows: vec![BulkInsertRow {
                node_id: NodeId::TOMBSTONE,
                vector: vec![1.0],
                max_layer: 0,
            }],
        }
        .encode()
        .expect_err("tombstone rejected");

        assert!(matches!(
            err,
            VectorError::InvalidNodeId { node_id, .. } if node_id == NodeId::TOMBSTONE
        ));
    }

    #[test]
    fn bulk_encode_rejects_duplicate_node_id() {
        let err = VectorBulkInsertPayloadV1 {
            rows: vec![
                BulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![1.0],
                    max_layer: 0,
                },
                BulkInsertRow {
                    node_id: NodeId::new(1),
                    vector: vec![2.0],
                    max_layer: 0,
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
    fn bulk_encode_rejects_max_layer_above_cap() {
        let err = VectorBulkInsertPayloadV1 {
            rows: vec![BulkInsertRow {
                node_id: NodeId::new(1),
                vector: vec![1.0],
                max_layer: 33,
            }],
        }
        .encode()
        .expect_err("max layer rejected");

        assert!(matches!(
            err,
            VectorError::MaxLayerExceedsCap {
                observed: 33,
                cap: 32
            }
        ));
    }

    #[test]
    fn bulk_encode_rejects_non_finite_component() {
        let err = VectorBulkInsertPayloadV1 {
            rows: vec![BulkInsertRow {
                node_id: NodeId::new(1),
                vector: vec![1.0, f32::NAN],
                max_layer: 0,
            }],
        }
        .encode()
        .expect_err("non-finite rejected");

        assert!(matches!(
            err,
            VectorError::NonFiniteVectorComponent { index: 1, value, .. } if value.is_nan()
        ));
    }

    #[test]
    fn decode_event_dispatches_upsert_and_bulk() {
        let upsert = VectorUpsertPayloadV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new(1),
            vector: vec![1.0],
            max_layer: 0,
        };
        let bulk = VectorBulkInsertPayloadV1 {
            rows: vec![BulkInsertRow {
                node_id: NodeId::new(2),
                vector: vec![2.0],
                max_layer: 0,
            }],
        };
        let bulk_delete = VectorBulkDeletePayloadV1 {
            node_ids: vec![NodeId::new(3)],
        };
        let ivf_bulk_insert = VectorIvfBulkInsertV1 {
            rows: vec![IvfBulkInsertRow {
                node_id: NodeId::new(4),
                vector: vec![4.0],
            }],
        };
        let ivf_bulk_delete = VectorIvfBulkDeleteV1 {
            node_ids: vec![NodeId::new(5)],
        };

        assert!(matches!(
            decode_event(&upsert.encode().unwrap()).unwrap(),
            EventKind::Upsert(decoded) if decoded == upsert
        ));
        assert!(matches!(
            decode_event(&bulk.encode().unwrap()).unwrap(),
            EventKind::Bulk(decoded) if decoded == bulk
        ));
        assert!(matches!(
            decode_event(&bulk_delete.encode().unwrap()).unwrap(),
            EventKind::BulkDelete(decoded) if decoded == bulk_delete
        ));
        assert!(matches!(
            decode_event(&ivf_bulk_insert.encode().unwrap()).unwrap(),
            EventKind::IvfBulkInsert
        ));
        assert!(matches!(
            decode_event(&ivf_bulk_delete.encode().unwrap()).unwrap(),
            EventKind::IvfBulkDelete
        ));
    }

    #[test]
    fn decode_event_rejects_truncated_and_unknown_magic() {
        let truncated = decode_event(b"VEC").expect_err("truncated rejected");
        assert!(matches!(
            truncated,
            VectorError::InvalidPayload { reason } if reason.contains("truncated")
        ));

        let unknown = decode_event(b"VECXpayload").expect_err("unknown rejected");
        assert!(matches!(
            unknown,
            VectorError::InvalidPayload { reason } if reason.contains("unknown vector event magic")
        ));
    }
}
