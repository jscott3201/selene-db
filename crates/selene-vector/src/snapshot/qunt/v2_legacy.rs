//! Decode/encode-compatible QUNT archive shape from after OPQ but before
//! polysemous. Used to preserve byte goldens when `polysemous_trained=false`.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::quantize::{
    PqCodebook, PqCodebookV2Legacy, QuantizedStore, QuantizedStorePq, QuantizedStoreSq8,
};
use crate::snapshot::{QUNT, encode_failed};

use super::{PAYLOAD_MAGIC_QUNT, QuntBodyV1};

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
struct QuntBodyV2Legacy {
    method: u8,
    dimensions: u16,
    node_count: u32,
    store: QuantizedStoreV2Legacy,
}

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
enum QuantizedStoreV2Legacy {
    Sq8(QuantizedStoreSq8),
    Pq(QuantizedStorePqV2Legacy),
}

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
struct QuantizedStorePqV2Legacy {
    codebook: PqCodebookV2Legacy,
    codes: Vec<u8>,
    approx_norms: Option<Vec<f32>>,
}

/// Emit the body in the post-OPQ archive shape when `polysemous_trained` is
/// false; otherwise return `None` and let the caller emit the flag-bearing V3
/// archive. Sq8 stores always render through V2 because SQ8 has no polysemous
/// concept.
pub(super) fn encode_if_legacy_compatible(
    body: &QuntBodyV1,
) -> Result<Option<Vec<u8>>, VectorError> {
    let store = match &body.store {
        QuantizedStore::Sq8(store) => QuantizedStoreV2Legacy::Sq8(store.clone()),
        QuantizedStore::Pq(store) => {
            let Some(codebook) = store.codebook.as_v2_legacy() else {
                return Ok(None);
            };
            QuantizedStoreV2Legacy::Pq(QuantizedStorePqV2Legacy {
                codebook,
                codes: store.codes.clone(),
                approx_norms: store.approx_norms.clone(),
            })
        }
    };
    let legacy = QuntBodyV2Legacy {
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
    _v3_error: &rkyv::rancor::Error,
) -> Result<QuntBodyV1, rkyv::rancor::Error> {
    let legacy = rkyv::from_bytes::<QuntBodyV2Legacy, rkyv::rancor::Error>(body)?;
    Ok(QuntBodyV1 {
        method: legacy.method,
        dimensions: legacy.dimensions,
        node_count: legacy.node_count,
        store: legacy.store.into(),
    })
}

impl From<QuantizedStoreV2Legacy> for QuantizedStore {
    fn from(value: QuantizedStoreV2Legacy) -> Self {
        match value {
            QuantizedStoreV2Legacy::Sq8(store) => Self::Sq8(store),
            QuantizedStoreV2Legacy::Pq(store) => Self::Pq(store.into()),
        }
    }
}

impl From<QuantizedStorePqV2Legacy> for QuantizedStorePq {
    fn from(value: QuantizedStorePqV2Legacy) -> Self {
        Self {
            codebook: PqCodebook::from(value.codebook),
            codes: value.codes,
            approx_norms: value.approx_norms,
        }
    }
}
