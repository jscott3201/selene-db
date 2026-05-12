//! Wire format for selene-vector mutation events.

use rkyv::{Archive, Deserialize, Serialize};
use selene_core::NodeId;

use crate::VectorError;

const MAX_LAYER: u8 = 32;

/// Magic prefix for every selene-vector mutation event payload.
pub const PAYLOAD_MAGIC: [u8; 4] = *b"VECU";

/// Vector mutation operation reserved in the BRIEF-59 wire format.
#[derive(Archive, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum VectorOp {
    /// Insert a fresh vector for a source graph node.
    Insert = 0,
    /// Replace an existing vector. Reserved; implemented after BRIEF-59.
    Update = 1,
    /// Delete an indexed vector. Reserved; implemented after BRIEF-59.
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
}
