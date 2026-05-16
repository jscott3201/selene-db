//! Wire format for selene-vector mutation events.

use std::collections::HashSet;
use std::str;

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

/// Leading byte for BRIEF-109 named-index WAL payloads.
pub const NAMED_PAYLOAD_VERSION: u8 = 0x01;

/// Magic prefix for vector index lifecycle create events.
pub const PAYLOAD_MAGIC_CREATE_INDEX: [u8; 4] = *b"VECC";

/// Magic prefix for vector index lifecycle drop events.
pub const PAYLOAD_MAGIC_DROP_INDEX: [u8; 4] = *b"VECX";

const LEGACY_PAYLOAD_PREFIX: u8 = b'V';

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

/// Named-index payload split after BRIEF-109 prefix decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedVectorPayload {
    /// Target vector index name. Legacy unprefixed payloads use `"default"`.
    pub index_name: String,
    /// Inner legacy-compatible vector payload bytes, beginning with a `V...`
    /// four-byte magic.
    pub body: Vec<u8>,
}

/// Version-1 vector lifecycle create-index payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorCreateIndexV1 {
    /// Canonical index kind (`"hnsw"` or `"ivf"`).
    pub kind: String,
    /// Serialized normalized provider config for `kind`.
    pub config: Vec<u8>,
}

/// Version-1 vector lifecycle drop-index payload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VectorDropIndexV1;

/// Decoded lifecycle event body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleEventKind {
    /// Create the named index using the embedded kind/config payload.
    Create(VectorCreateIndexV1),
    /// Drop the named index. The target name is carried by the named payload
    /// prefix.
    Drop(VectorDropIndexV1),
}

/// Encode `body` under the BRIEF-109 named-index WAL prefix.
///
/// # Errors
///
/// Returns [`VectorError::InvalidPayload`] when the index name is empty or too
/// long for the wire field.
pub fn encode_named_payload(index_name: &str, body: Vec<u8>) -> Result<Vec<u8>, VectorError> {
    if index_name.is_empty() {
        return Err(invalid_payload(
            "named payload index name must be non-empty",
        ));
    }
    let name_len = u16::try_from(index_name.len())
        .map_err(|_| invalid_payload("named payload index name exceeds u16 length"))?;
    let mut out = Vec::with_capacity(1 + 2 + index_name.len() + body.len());
    out.push(NAMED_PAYLOAD_VERSION);
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(index_name.as_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode the BRIEF-109 named-index WAL prefix, falling back to v1.0
/// unprefixed payloads as `"default"`.
///
/// # Errors
///
/// Returns [`VectorError::InvalidPayload`] when the prefix, name, or inner
/// payload shape is malformed.
pub fn split_named_payload(bytes: &[u8]) -> Result<NamedVectorPayload, VectorError> {
    let Some(first) = bytes.first().copied() else {
        return Err(invalid_payload("vector event payload is empty"));
    };
    match first {
        NAMED_PAYLOAD_VERSION => split_v1_named_payload(bytes),
        LEGACY_PAYLOAD_PREFIX => Ok(NamedVectorPayload {
            index_name: "default".to_owned(),
            body: bytes.to_vec(),
        }),
        other => Err(invalid_payload(format!(
            "unknown vector event prefix 0x{other:02X}"
        ))),
    }
}

fn split_v1_named_payload(bytes: &[u8]) -> Result<NamedVectorPayload, VectorError> {
    let Some(name_len_bytes) = bytes.get(1..3) else {
        return Err(invalid_payload(
            "named payload truncated before name length",
        ));
    };
    let name_len = u16::from_le_bytes(
        name_len_bytes
            .try_into()
            .expect("slice length checked for u16"),
    ) as usize;
    let name_start = 3_usize;
    let name_end = name_start
        .checked_add(name_len)
        .ok_or_else(|| invalid_payload("named payload name length overflow"))?;
    let Some(name_bytes) = bytes.get(name_start..name_end) else {
        return Err(invalid_payload("named payload truncated in name"));
    };
    if name_bytes.is_empty() {
        return Err(invalid_payload(
            "named payload index name must be non-empty",
        ));
    }
    let index_name = str::from_utf8(name_bytes)
        .map_err(|error| {
            invalid_payload(format!("named payload index name is not UTF-8: {error}"))
        })?
        .to_owned();
    let body = bytes
        .get(name_end..)
        .ok_or_else(|| invalid_payload("named payload truncated before body"))?;
    if body.len() < 4 {
        return Err(invalid_payload(
            "named payload body truncated: less than 4 magic bytes",
        ));
    }
    if body.first().copied() != Some(LEGACY_PAYLOAD_PREFIX) {
        return Err(invalid_payload(
            "named payload body does not start with vector magic",
        ));
    }
    Ok(NamedVectorPayload {
        index_name,
        body: body.to_vec(),
    })
}

impl VectorCreateIndexV1 {
    /// Encode this lifecycle payload to `VECC || kind || config`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] if the kind/config fields cannot
    /// be represented on the wire.
    pub fn encode(&self) -> Result<Vec<u8>, VectorError> {
        if self.kind.is_empty() {
            return Err(invalid_payload("create-index kind must be non-empty"));
        }
        if self.config.is_empty() {
            return Err(invalid_payload("create-index config must be non-empty"));
        }
        let kind_len = u16::try_from(self.kind.len())
            .map_err(|_| invalid_payload("create-index kind exceeds u16 length"))?;
        let config_len = u32::try_from(self.config.len())
            .map_err(|_| invalid_payload("create-index config exceeds u32 length"))?;
        let mut out = Vec::with_capacity(4 + 2 + self.kind.len() + 4 + self.config.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_CREATE_INDEX);
        out.extend_from_slice(&kind_len.to_le_bytes());
        out.extend_from_slice(self.kind.as_bytes());
        out.extend_from_slice(&config_len.to_le_bytes());
        out.extend_from_slice(&self.config);
        Ok(out)
    }

    /// Decode a `VECC` lifecycle payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the payload is malformed.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        let Some((magic, mut body)) = bytes.split_at_checked(4) else {
            return Err(invalid_payload("create-index payload shorter than magic"));
        };
        if magic != PAYLOAD_MAGIC_CREATE_INDEX {
            return Err(invalid_payload("payload magic is not VECC"));
        }
        let kind_len = read_u16(&mut body, "create-index kind length")? as usize;
        let kind = read_str(&mut body, kind_len, "create-index kind")?.to_owned();
        let config_len = read_u32(&mut body, "create-index config length")? as usize;
        let config = read_bytes(&mut body, config_len, "create-index config")?.to_vec();
        if !body.is_empty() {
            return Err(invalid_payload("create-index payload has trailing bytes"));
        }
        if config.is_empty() {
            return Err(invalid_payload("create-index config must be non-empty"));
        }
        Ok(Self { kind, config })
    }
}

impl VectorDropIndexV1 {
    /// Encode this lifecycle payload to `VECX`.
    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        PAYLOAD_MAGIC_DROP_INDEX.to_vec()
    }

    /// Decode a `VECX` lifecycle payload.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::InvalidPayload`] when the payload is malformed.
    pub fn decode(bytes: &[u8]) -> Result<Self, VectorError> {
        if bytes == PAYLOAD_MAGIC_DROP_INDEX {
            return Ok(Self);
        }
        Err(invalid_payload("payload magic is not VECX"))
    }
}

/// Decode a lifecycle event body beginning with `VECC` or `VECX`.
///
/// # Errors
///
/// Returns [`VectorError::InvalidPayload`] when the magic or body is invalid.
pub fn decode_lifecycle_event(bytes: &[u8]) -> Result<LifecycleEventKind, VectorError> {
    let Some((magic, _body)) = bytes.split_at_checked(4) else {
        return Err(invalid_payload(
            "lifecycle event truncated: less than 4 magic bytes",
        ));
    };
    let magic: [u8; 4] = magic
        .try_into()
        .expect("split_at_checked guarantees four magic bytes");
    match magic {
        PAYLOAD_MAGIC_CREATE_INDEX => {
            VectorCreateIndexV1::decode(bytes).map(LifecycleEventKind::Create)
        }
        PAYLOAD_MAGIC_DROP_INDEX => VectorDropIndexV1::decode(bytes).map(LifecycleEventKind::Drop),
        other => Err(invalid_payload(format!(
            "unknown lifecycle event magic {}",
            String::from_utf8_lossy(&other)
        ))),
    }
}

fn read_u16(bytes: &mut &[u8], label: &'static str) -> Result<u16, VectorError> {
    let Some((raw, rest)) = bytes.split_at_checked(2) else {
        return Err(invalid_payload(format!("{label} truncated")));
    };
    *bytes = rest;
    Ok(u16::from_le_bytes(
        raw.try_into().expect("slice length checked for u16"),
    ))
}

fn read_u32(bytes: &mut &[u8], label: &'static str) -> Result<u32, VectorError> {
    let Some((raw, rest)) = bytes.split_at_checked(4) else {
        return Err(invalid_payload(format!("{label} truncated")));
    };
    *bytes = rest;
    Ok(u32::from_le_bytes(
        raw.try_into().expect("slice length checked for u32"),
    ))
}

fn read_bytes<'a>(
    bytes: &mut &'a [u8],
    len: usize,
    label: &'static str,
) -> Result<&'a [u8], VectorError> {
    let Some((raw, rest)) = bytes.split_at_checked(len) else {
        return Err(invalid_payload(format!("{label} truncated")));
    };
    *bytes = rest;
    Ok(raw)
}

fn read_str<'a>(
    bytes: &mut &'a [u8],
    len: usize,
    label: &'static str,
) -> Result<&'a str, VectorError> {
    let raw = read_bytes(bytes, len, label)?;
    if raw.is_empty() {
        return Err(invalid_payload(format!("{label} must be non-empty")));
    }
    str::from_utf8(raw).map_err(|error| invalid_payload(format!("{label} is not UTF-8: {error}")))
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
mod named_tests;

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
