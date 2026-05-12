//! Codec for the `QUNT` SQ8 snapshot overlay section.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::hnsw::HnswGraph;
use crate::quantize::{QuantMethod, QuantizedStoreSq8};

use super::{QUNT, decode_failed, encode_failed, expected_component_len, node_count_usize};

/// Magic prefix for version-1 `QUNT` section bodies.
pub(crate) const PAYLOAD_MAGIC_QUNT: [u8; 4] = *b"VQNT";

/// Archived body for the `QUNT` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct QuntBodyV1 {
    /// Raw method byte; decode validates through `QuantMethod::from_wire`.
    pub(crate) method: u8,
    /// Vector dimensions locked at graph construction.
    pub(crate) dimensions: u16,
    /// Number of dense graph rows covered by this quantized prefix.
    pub(crate) node_count: u32,
    /// SQ8 quantized store.
    pub(crate) store: QuantizedStoreSq8,
}

pub(crate) fn encode_qunt(graph: &HnswGraph, method: QuantMethod) -> Result<Vec<u8>, VectorError> {
    let node_count = u32::try_from(graph.len())
        .map_err(|_| encode_failed(QUNT, "graph node count exceeds QUNT header range"))?;
    let dimensions = u16::try_from(graph.dimensions())
        .map_err(|_| encode_failed(QUNT, "graph dimensions exceed QUNT header range"))?;
    let store = QuantizedStoreSq8::build(
        graph.len(),
        graph.dimensions(),
        graph.iter_nodes().map(|node| node.vector.as_ref()),
    )?;
    let body = QuntBodyV1 {
        method: method.to_wire(),
        dimensions,
        node_count,
        store,
    };
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&body)
        .map_err(|error| encode_failed(QUNT, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_QUNT.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_QUNT);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_qunt(bytes: &[u8]) -> Result<QuntBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_QUNT.len()) else {
        return Err(decode_failed(QUNT, "QUNT magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_QUNT {
        return Err(decode_failed(QUNT, "QUNT magic mismatch"));
    }
    let decoded = rkyv::from_bytes::<QuntBodyV1, rkyv::rancor::Error>(body)
        .map_err(|error| decode_failed(QUNT, format!("rkyv decode failed: {error}")))?;
    validate_qunt_body(&decoded)?;
    Ok(decoded)
}

pub(crate) fn validate_qunt_for_graph(
    body: &QuntBodyV1,
    graph: &HnswGraph,
) -> Result<QuantMethod, VectorError> {
    let method = QuantMethod::from_wire(body.method)?;
    if usize::from(body.dimensions) != graph.dimensions() {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT dimension mismatch: {} != {}",
                body.dimensions,
                graph.dimensions()
            ),
        ));
    }
    let node_count = node_count_usize(body.node_count, QUNT)?;
    if node_count > graph.len() {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT node_count {} exceeds graph node count {}",
                body.node_count,
                graph.len()
            ),
        ));
    }
    Ok(method)
}

fn validate_qunt_body(body: &QuntBodyV1) -> Result<(), VectorError> {
    QuantMethod::from_wire(body.method)?;
    if body.dimensions == 0 {
        return Err(decode_failed(
            QUNT,
            "QUNT dimensions must be greater than zero",
        ));
    }
    let dim = usize::from(body.dimensions);
    let expected_len = expected_component_len(body.node_count, body.dimensions, QUNT)?;
    if body.store.range.min.len() != dim || body.store.range.max.len() != dim {
        return Err(decode_failed(
            QUNT,
            format!("QUNT range length disagrees with dimensions {dim}"),
        ));
    }
    if body.store.codes.len() != expected_len {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT codes length {} disagrees with expected {expected_len}",
                body.store.codes.len()
            ),
        ));
    }
    let node_count = node_count_usize(body.node_count, QUNT)?;
    if body.store.approx_norms.len() != node_count {
        return Err(decode_failed(
            QUNT,
            format!(
                "QUNT approx_norms length {} disagrees with node_count {node_count}",
                body.store.approx_norms.len()
            ),
        ));
    }
    validate_ranges(&body.store.range.min, &body.store.range.max)?;
    for (index, norm) in body.store.approx_norms.iter().copied().enumerate() {
        // Codex review fix (P2): approximate vector magnitudes must be
        // non-negative; a negative norm survives finiteness checks but
        // silently inverts cosine asymmetric scores.
        if !norm.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT approx_norm at index {index}: {norm}"),
            ));
        }
        if norm < 0.0 {
            return Err(decode_failed(
                QUNT,
                format!("negative QUNT approx_norm at index {index}: {norm}"),
            ));
        }
    }
    Ok(())
}

fn validate_ranges(min: &[f32], max: &[f32]) -> Result<(), VectorError> {
    for (index, (min, max)) in min.iter().copied().zip(max.iter().copied()).enumerate() {
        if !min.is_finite() || !max.is_finite() {
            return Err(decode_failed(
                QUNT,
                format!("non-finite QUNT range at coordinate {index}: {min}, {max}"),
            ));
        }
        if min > max {
            return Err(decode_failed(
                QUNT,
                format!("inverted QUNT range at coordinate {index}: {min} > {max}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::NodeId;

    use crate::hnsw::{HnswGraph, HnswNode};
    use crate::quantize::PerCoordRange;

    use super::*;

    fn graph() -> HnswGraph {
        let mut graph = HnswGraph::empty(2);
        graph
            .nodes
            .push(HnswNode::new(NodeId::new(1), Arc::from([0.0, 1.0]), 0).unwrap());
        graph
            .nodes
            .push(HnswNode::new(NodeId::new(2), Arc::from([1.0, 2.0]), 0).unwrap());
        graph
    }

    fn body(node_count: u32, dimensions: u16) -> QuntBodyV1 {
        let graph = graph();
        let mut encoded = decode_qunt(&encode_qunt(&graph, QuantMethod::Sq8).unwrap()).unwrap();
        encoded.node_count = node_count;
        encoded.dimensions = dimensions;
        encoded
    }

    fn raw_encode(body: &QuntBodyV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_QUNT.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_QUNT);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn qunt_roundtrip_preserves_bytes() {
        let graph = graph();
        let bytes = encode_qunt(&graph, QuantMethod::Sq8).unwrap();
        let decoded = decode_qunt(&bytes).unwrap();

        assert_eq!(bytes, raw_encode(&decoded));
    }

    #[test]
    fn qunt_decode_rejects_unknown_method() {
        let mut body = body(2, 2);
        body.method = 0xFF;

        let err = decode_qunt(&raw_encode(&body)).expect_err("unknown method rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("unknown QUNT method"))
        );
    }

    #[test]
    fn qunt_decode_rejects_codes_length_mismatch() {
        let mut body = body(2, 2);
        body.store.codes.pop();

        let err = decode_qunt(&raw_encode(&body)).expect_err("bad length rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("codes length"))
        );
    }

    #[test]
    fn qunt_decode_rejects_norms_length_mismatch() {
        let mut body = body(2, 2);
        body.store.approx_norms.pop();

        let err = decode_qunt(&raw_encode(&body)).expect_err("bad norm length rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("approx_norms"))
        );
    }

    #[test]
    fn qunt_decode_rejects_non_finite_range_or_norm() {
        let mut range_body = body(2, 2);
        range_body.store.range.min[0] = f32::NAN;
        let range_err =
            decode_qunt(&raw_encode(&range_body)).expect_err("non-finite range rejected");
        assert!(
            matches!(range_err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("non-finite"))
        );

        let mut norm_body = body(2, 2);
        norm_body.store.approx_norms[0] = f32::INFINITY;
        let norm_err = decode_qunt(&raw_encode(&norm_body)).expect_err("non-finite norm rejected");
        assert!(
            matches!(norm_err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("approx_norm"))
        );
    }

    #[test]
    fn qunt_decode_rejects_negative_approx_norm() {
        let mut body = body(2, 2);
        body.store.approx_norms[0] = -1.0;

        let err = decode_qunt(&raw_encode(&body)).expect_err("negative norm rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("negative") && reason.contains("approx_norm"))
        );
    }

    #[test]
    fn qunt_decode_rejects_inverted_range() {
        let mut body = body(2, 2);
        body.store.range = PerCoordRange {
            min: vec![1.0, 0.0],
            max: vec![0.0, 2.0],
        };

        let err = decode_qunt(&raw_encode(&body)).expect_err("inverted range rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("inverted"))
        );
    }

    #[test]
    fn qunt_decode_rejects_dimension_mismatch_against_graph() {
        let body = body(2, 3);

        let err = validate_qunt_for_graph(&body, &graph()).expect_err("dimension mismatch");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("dimension mismatch"))
        );
    }

    #[test]
    fn qunt_decode_rejects_node_count_above_graph_len() {
        let body = body(3, 2);

        let err = validate_qunt_for_graph(&body, &graph()).expect_err("node count mismatch");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("exceeds graph"))
        );
    }

    #[test]
    fn qunt_decode_allows_node_count_below_graph_len() {
        let mut body = body(2, 2);
        body.node_count = 1;
        body.store.codes.truncate(2);
        body.store.approx_norms.truncate(1);
        let decoded = decode_qunt(&raw_encode(&body)).unwrap();

        validate_qunt_for_graph(&decoded, &graph()).unwrap();
    }
}
