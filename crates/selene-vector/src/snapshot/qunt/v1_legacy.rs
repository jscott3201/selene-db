//! Decode-only QUNT archive shapes from before OPQ rotation.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::quantize::{
    PqCodebook, PqCodebookV1Legacy, QuantizedStore, QuantizedStorePq, QuantizedStoreSq8,
};
use crate::snapshot::{QUNT, decode_failed, encode_failed};

use super::{PAYLOAD_MAGIC_QUNT, QuntBodyV1};

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
struct QuntBodyV1Legacy {
    method: u8,
    dimensions: u16,
    node_count: u32,
    store: QuantizedStoreV1Legacy,
}

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
enum QuantizedStoreV1Legacy {
    Sq8(QuantizedStoreSq8),
    Pq(QuantizedStorePqV1Legacy),
}

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
struct QuantizedStorePqV1Legacy {
    m_subspaces: u32,
    k_centroids: u32,
    subspace_dim: u32,
    codebook: Vec<f32>,
    codes: Vec<u8>,
    approx_norms: Option<Vec<f32>>,
}

pub(super) fn encode_if_legacy_compatible(
    body: &QuntBodyV1,
) -> Result<Option<Vec<u8>>, VectorError> {
    let store = match &body.store {
        QuantizedStore::Sq8(store) => QuantizedStoreV1Legacy::Sq8(store.clone()),
        QuantizedStore::Pq(store) => {
            if store.codebook.rotation.is_some() {
                return Ok(None);
            }
            QuantizedStoreV1Legacy::Pq(QuantizedStorePqV1Legacy {
                m_subspaces: store.codebook.m_subspaces,
                k_centroids: store.codebook.k_centroids,
                subspace_dim: store.codebook.subspace_dim,
                codebook: store.codebook.centroids.clone(),
                codes: store.codes.clone(),
                approx_norms: store.approx_norms.clone(),
            })
        }
    };
    let legacy = QuntBodyV1Legacy {
        method: body.method,
        dimensions: body.dimensions,
        node_count: body.node_count,
        store,
    };
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&legacy)
        .map_err(|error| encode_failed(QUNT, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_QUNT.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_QUNT);
    out.extend_from_slice(&archived);
    Ok(Some(out))
}

pub(super) fn decode(
    body: &[u8],
    v2_error: rkyv::rancor::Error,
) -> Result<QuntBodyV1, VectorError> {
    let legacy = rkyv::from_bytes::<QuntBodyV1Legacy, rkyv::rancor::Error>(body).map_err(
        |legacy_error| {
            decode_failed(
                QUNT,
                format!("rkyv decode failed: v2={v2_error}; legacy={legacy_error}"),
            )
        },
    )?;
    Ok(QuntBodyV1 {
        method: legacy.method,
        dimensions: legacy.dimensions,
        node_count: legacy.node_count,
        store: legacy.store.into(),
    })
}

impl From<QuantizedStoreV1Legacy> for QuantizedStore {
    fn from(value: QuantizedStoreV1Legacy) -> Self {
        match value {
            QuantizedStoreV1Legacy::Sq8(store) => Self::Sq8(store),
            QuantizedStoreV1Legacy::Pq(store) => Self::Pq(store.into()),
        }
    }
}

impl From<QuantizedStorePqV1Legacy> for QuantizedStorePq {
    fn from(value: QuantizedStorePqV1Legacy) -> Self {
        Self {
            codebook: PqCodebook::from(PqCodebookV1Legacy {
                m_subspaces: value.m_subspaces,
                k_centroids: value.k_centroids,
                subspace_dim: value.subspace_dim,
                centroids: value.codebook,
            }),
            codes: value.codes,
            approx_norms: value.approx_norms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantize::QuantMethod;
    use crate::snapshot::qunt::{PAYLOAD_MAGIC_QUNT, decode_qunt};

    fn raw_encode_legacy(body: &QuntBodyV1Legacy) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_QUNT.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_QUNT);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn legacy_qunt_pq_decodes_to_none_rotation() {
        let body = QuntBodyV1Legacy {
            method: QuantMethod::Pq.to_wire(),
            dimensions: 2,
            node_count: 1,
            store: QuantizedStoreV1Legacy::Pq(QuantizedStorePqV1Legacy {
                m_subspaces: 1,
                k_centroids: 256,
                subspace_dim: 2,
                codebook: vec![0.0; 512],
                codes: vec![0],
                approx_norms: None,
            }),
        };

        let decoded = decode_qunt(&raw_encode_legacy(&body)).unwrap();

        let QuantizedStore::Pq(store) = decoded.store else {
            panic!("legacy PQ store should decode as PQ");
        };
        assert!(store.codebook.rotation.is_none());
    }
}
