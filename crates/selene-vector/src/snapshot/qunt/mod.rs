//! Codec for the `QUNT` quantized snapshot overlay section.

use rkyv::{Archive, Deserialize, Serialize};

mod v1_legacy;
mod v2_legacy;
mod validate;

use crate::hnsw::HnswGraph;
use crate::quantize::{QuantMethod, QuantizedStore};
use crate::{HnswConfig, VectorError};

use super::{QUNT, decode_failed, encode_failed};

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
    /// Quantized store matching `method`.
    pub(crate) store: QuantizedStore,
}

pub(crate) fn encode_qunt(
    graph: &HnswGraph,
    config: &HnswConfig,
) -> Result<(Vec<u8>, Option<QuantizedStore>), VectorError> {
    let Some(store) = QuantizedStore::build(
        graph.len(),
        graph.dimensions(),
        config.metric,
        config.quantization,
        graph.iter_nodes().map(|node| node.vector.as_ref()),
    )?
    else {
        return Ok((Vec::new(), None));
    };
    let bytes = encode_qunt_store(graph, store.clone())?;
    Ok((bytes, Some(store)))
}

pub(crate) fn encode_qunt_store(
    graph: &HnswGraph,
    store: QuantizedStore,
) -> Result<Vec<u8>, VectorError> {
    let node_count = u32::try_from(graph.len())
        .map_err(|_| encode_failed(QUNT, "graph node count exceeds QUNT header range"))?;
    let dimensions = u16::try_from(graph.dimensions())
        .map_err(|_| encode_failed(QUNT, "graph dimensions exceed QUNT header range"))?;
    let body = QuntBodyV1 {
        method: store.method().to_wire(),
        dimensions,
        node_count,
        store,
    };
    // Encode-tier cascade: try v1 first (BRIEF-66 byte parity when
    // rotation=None AND polysemous_trained=false), then v2 (BRIEF-68 byte
    // parity when polysemous_trained=false), then the v3 flag-bearing
    // archive when polysemous_trained=true.
    if let Some(bytes) = v1_legacy::encode_if_legacy_compatible(&body)? {
        return Ok(bytes);
    }
    if let Some(bytes) = v2_legacy::encode_if_legacy_compatible(&body)? {
        return Ok(bytes);
    }
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
    // Decode-tier cascade: try the v3 flag-bearing archive (current shape),
    // then v2_legacy (rotation but no polysemous flag), then v1_legacy
    // (no rotation, no flag). Each successive try consumes the prior
    // rkyv error to keep diagnostics surfaceable when all three fail.
    let decoded = match rkyv::from_bytes::<QuntBodyV1, rkyv::rancor::Error>(body) {
        Ok(decoded) => decoded,
        Err(v3_error) => match v2_legacy::decode(body, &v3_error) {
            Ok(decoded) => decoded,
            Err(v2_error) => v1_legacy::decode(body, v2_error)?,
        },
    };
    validate::validate_qunt_body(&decoded)?;
    Ok(decoded)
}

pub(crate) fn validate_qunt_for_graph(
    body: &QuntBodyV1,
    graph: &HnswGraph,
    config: &HnswConfig,
) -> Result<QuantMethod, VectorError> {
    validate::validate_qunt_for_graph(body, graph, config)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::NodeId;

    use crate::DistanceMetric;
    use crate::hnsw::{HnswGraph, HnswNode};
    use crate::quantize::{PerCoordRange, QuantizedStore, QuantizedStoreSq8};

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

    fn pq_graph(dim: usize) -> HnswGraph {
        let mut graph = HnswGraph::empty(dim as u16);
        for raw in 1..=256 {
            let vector = (0..dim)
                .map(|coord| ((raw as f32 * 0.031) + (coord as f32 * 0.17)).sin())
                .collect::<Vec<_>>();
            graph
                .nodes
                .push(HnswNode::new(NodeId::new(raw), Arc::from(vector), 0).unwrap());
        }
        graph
    }

    fn config() -> HnswConfig {
        HnswConfig::new(2)
            .unwrap()
            .with_quantization(crate::QuantizationConfig {
                enabled: true,
                ..Default::default()
            })
            .unwrap()
    }

    fn pq_config(metric: DistanceMetric) -> HnswConfig {
        HnswConfig::with_params(2, 16, 200, 50, metric)
            .unwrap()
            .with_quantization(crate::QuantizationConfig {
                enabled: true,
                method: QuantMethod::Pq,
                pq: Some(crate::PqParams {
                    m_subspaces: 1,
                    k_centroids: 256,
                    train_min_vectors: 256,
                    use_opq: false,
                    use_polysemous: false,
                    hamming_threshold_ratio: 0.5,
                }),
                ..Default::default()
            })
            .unwrap()
    }

    fn body(node_count: u32, dimensions: u16) -> QuntBodyV1 {
        let graph = graph();
        let (bytes, _) = encode_qunt(&graph, &config()).unwrap();
        let mut encoded = decode_qunt(&bytes).unwrap();
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

    fn sq8_mut(body: &mut QuntBodyV1) -> &mut QuantizedStoreSq8 {
        match &mut body.store {
            QuantizedStore::Sq8(store) => store,
            QuantizedStore::Pq(_) => panic!("test body should be SQ8"),
        }
    }

    fn pq_mut(body: &mut QuntBodyV1) -> &mut crate::quantize::QuantizedStorePq {
        match &mut body.store {
            QuantizedStore::Pq(store) => store,
            QuantizedStore::Sq8(_) => panic!("test body should be PQ"),
        }
    }

    #[test]
    fn qunt_roundtrip_preserves_bytes() {
        let graph = graph();
        let (bytes, _) = encode_qunt(&graph, &config()).unwrap();
        let decoded = decode_qunt(&bytes).unwrap();
        let reencoded = encode_qunt_store(&graph, decoded.store).unwrap();

        assert_eq!(bytes, reencoded);
    }

    #[test]
    fn qunt_codec_preserves_vqnt_magic_and_archived_method_field() {
        let graph = graph();
        let (bytes, _) = encode_qunt(&graph, &config()).unwrap();

        assert!(bytes.starts_with(&PAYLOAD_MAGIC_QUNT));
        let archived = &bytes[PAYLOAD_MAGIC_QUNT.len()..];
        let decoded = decode_qunt(&bytes).expect("method lives inside archived body");
        assert_eq!(decoded.method, QuantMethod::Sq8.to_wire());
        assert!(!archived.is_empty());
    }

    #[test]
    fn qunt_codec_method_byte_one_decodes_as_pq() {
        let graph = pq_graph(2);
        let (bytes, _) = encode_qunt(&graph, &pq_config(DistanceMetric::L2)).unwrap();
        let decoded = decode_qunt(&bytes).unwrap();

        assert_eq!(decoded.method, QuantMethod::Pq.to_wire());
        assert!(matches!(decoded.store, QuantizedStore::Pq(_)));
    }

    #[test]
    fn qunt_decode_rejects_bad_pq_rotation_shape() {
        let graph = pq_graph(2);
        let (bytes, _) = encode_qunt(&graph, &pq_config(DistanceMetric::L2)).unwrap();
        let mut decoded = decode_qunt(&bytes).unwrap();
        pq_mut(&mut decoded).codebook.rotation = Some(vec![1.0, 0.0, 0.0]);

        let err = decode_qunt(&raw_encode(&decoded)).expect_err("bad rotation rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("rotation length"))
        );
    }

    #[test]
    fn qunt_decode_rejects_non_orthonormal_pq_rotation() {
        let graph = pq_graph(2);
        let (bytes, _) = encode_qunt(&graph, &pq_config(DistanceMetric::L2)).unwrap();
        let mut decoded = decode_qunt(&bytes).unwrap();
        pq_mut(&mut decoded).codebook.rotation = Some(vec![2.0, 0.0, 0.0, 1.0]);

        let err = decode_qunt(&raw_encode(&decoded)).expect_err("bad rotation rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("not orthonormal"))
        );
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
    fn pq_cosine_decode_rejects_missing_approx_norms() {
        let graph = pq_graph(2);
        let (bytes, _) = encode_qunt(&graph, &pq_config(DistanceMetric::L2)).unwrap();
        let decoded = decode_qunt(&bytes).unwrap();

        let err = validate_qunt_for_graph(&decoded, &graph, &pq_config(DistanceMetric::Cosine))
            .expect_err("PQ cosine requires approximate norms");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("approx_norms"))
        );
    }

    #[test]
    fn qunt_decode_rejects_codes_length_mismatch() {
        let mut body = body(2, 2);
        sq8_mut(&mut body).codes.pop();

        let err = decode_qunt(&raw_encode(&body)).expect_err("bad length rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("codes length"))
        );
    }

    #[test]
    fn qunt_decode_rejects_norms_length_mismatch() {
        let mut body = body(2, 2);
        sq8_mut(&mut body).approx_norms.pop();

        let err = decode_qunt(&raw_encode(&body)).expect_err("bad norm length rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("approx_norms"))
        );
    }

    #[test]
    fn qunt_decode_rejects_non_finite_range_or_norm() {
        let mut range_body = body(2, 2);
        sq8_mut(&mut range_body).range.min[0] = f32::NAN;
        let range_err =
            decode_qunt(&raw_encode(&range_body)).expect_err("non-finite range rejected");
        assert!(
            matches!(range_err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("non-finite"))
        );

        let mut norm_body = body(2, 2);
        sq8_mut(&mut norm_body).approx_norms[0] = f32::INFINITY;
        let norm_err = decode_qunt(&raw_encode(&norm_body)).expect_err("non-finite norm rejected");
        assert!(
            matches!(norm_err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("approx_norm"))
        );
    }

    #[test]
    fn qunt_decode_rejects_negative_approx_norm() {
        let mut body = body(2, 2);
        sq8_mut(&mut body).approx_norms[0] = -1.0;

        let err = decode_qunt(&raw_encode(&body)).expect_err("negative norm rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("negative") && reason.contains("approx_norm"))
        );
    }

    #[test]
    fn qunt_decode_rejects_inverted_range() {
        let mut body = body(2, 2);
        sq8_mut(&mut body).range = PerCoordRange {
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

        let err =
            validate_qunt_for_graph(&body, &graph(), &config()).expect_err("dimension mismatch");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("dimension mismatch"))
        );
    }

    #[test]
    fn qunt_decode_rejects_node_count_above_graph_len() {
        let body = body(3, 2);

        let err =
            validate_qunt_for_graph(&body, &graph(), &config()).expect_err("node count mismatch");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("exceeds graph"))
        );
    }

    #[test]
    fn qunt_decode_allows_node_count_below_graph_len() {
        let mut body = body(2, 2);
        body.node_count = 1;
        sq8_mut(&mut body).codes.truncate(2);
        sq8_mut(&mut body).approx_norms.truncate(1);
        let decoded = decode_qunt(&raw_encode(&body)).unwrap();

        validate_qunt_for_graph(&decoded, &graph(), &config()).unwrap();
    }
}
