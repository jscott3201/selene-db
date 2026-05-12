//! Codec for the `VECS` raw-vector snapshot section.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::hnsw::HnswGraph;

use super::{VECS, decode_failed, encode_failed, expected_component_len};

/// Magic prefix for version-1 `VECS` section bodies.
pub(crate) const PAYLOAD_MAGIC_VECS: [u8; 4] = *b"VVEC";

/// Archived header for the `VECS` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct VecsHeaderV1 {
    /// Vector dimensions locked at graph construction.
    pub(crate) dimensions: u16,
    /// Total node count.
    pub(crate) node_count: u32,
}

/// Archived vector body in InternalIndex order.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct VecsBodyV1 {
    /// Structural metadata mirrored against GRPH.
    pub(crate) header: VecsHeaderV1,
    /// Flat `node_count * dimensions` component buffer.
    pub(crate) components: Vec<f32>,
}

pub(crate) fn encode_vecs(graph: &HnswGraph) -> Result<Vec<u8>, VectorError> {
    let node_count = u32::try_from(graph.len())
        .map_err(|_| encode_failed(VECS, "graph node count exceeds VECS header range"))?;
    let dimensions = u16::try_from(graph.dimensions())
        .map_err(|_| encode_failed(VECS, "graph dimensions exceed VECS header range"))?;
    let expected_len = expected_component_len(node_count, dimensions, VECS)
        .map_err(|err| encode_failed(VECS, err.to_string()))?;
    let mut components = Vec::with_capacity(expected_len);
    for node in graph.iter_nodes() {
        components.extend_from_slice(&node.vector);
    }
    let body = VecsBodyV1 {
        header: VecsHeaderV1 {
            dimensions,
            node_count,
        },
        components,
    };
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&body)
        .map_err(|error| encode_failed(VECS, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_VECS.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_VECS);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_vecs(bytes: &[u8]) -> Result<VecsBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_VECS.len()) else {
        return Err(decode_failed(VECS, "VECS magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_VECS {
        return Err(decode_failed(VECS, "VECS magic mismatch"));
    }
    let decoded = rkyv::from_bytes::<VecsBodyV1, rkyv::rancor::Error>(body)
        .map_err(|error| decode_failed(VECS, format!("rkyv decode failed: {error}")))?;
    validate_vecs(&decoded)?;
    Ok(decoded)
}

fn validate_vecs(body: &VecsBodyV1) -> Result<(), VectorError> {
    if body.header.dimensions == 0 {
        return Err(decode_failed(
            VECS,
            "VECS dimensions must be greater than zero",
        ));
    }
    let expected_len =
        expected_component_len(body.header.node_count, body.header.dimensions, VECS)?;
    if body.components.len() != expected_len {
        return Err(decode_failed(
            VECS,
            format!(
                "VECS components length {} disagrees with expected {expected_len}",
                body.components.len()
            ),
        ));
    }
    for (index, value) in body.components.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                VECS,
                format!("non-finite vector component at flat index {index}: {value}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(node_count: u32, dimensions: u16, components: Vec<f32>) -> VecsBodyV1 {
        VecsBodyV1 {
            header: VecsHeaderV1 {
                dimensions,
                node_count,
            },
            components,
        }
    }

    fn raw_encode(body: &VecsBodyV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_VECS.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_VECS);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn decode_rejects_wrong_magic() {
        let err = decode_vecs(b"XXXX").expect_err("bad magic rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("magic"))
        );
    }

    #[test]
    fn decode_rejects_zero_dimensions() {
        let err = decode_vecs(&raw_encode(&body(0, 0, vec![]))).expect_err("zero dim rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("dimensions"))
        );
    }

    #[test]
    fn decode_rejects_component_length_mismatch() {
        let err = decode_vecs(&raw_encode(&body(2, 2, vec![1.0, 2.0, 3.0])))
            .expect_err("length mismatch rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("components length"))
        );
    }

    #[test]
    fn decode_rejects_non_finite_component() {
        let err = decode_vecs(&raw_encode(&body(1, 2, vec![1.0, f32::INFINITY])))
            .expect_err("non-finite rejected");
        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("non-finite"))
        );
    }
}
