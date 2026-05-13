//! Codec for the `CQNT` IVF coarse-quantizer section.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;

use super::{CQNT, decode_failed, encode_failed};

/// Magic prefix for version-1 `CQNT` section bodies.
pub(crate) const PAYLOAD_MAGIC_CQNT: [u8; 4] = *b"VCQB";

/// Archived body for the `CQNT` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum CqntBodyV1 {
    /// No coarse quantizer has been trained.
    Empty,
    /// Trained coarse centroids.
    Trained {
        /// Number of coarse centroids.
        k_coarse: u32,
        /// Vector dimensionality.
        dim: u16,
        /// Centroids laid out centroid-major.
        centroids: Vec<f32>,
    },
}

pub(crate) fn encode_cqnt(body: &CqntBodyV1) -> Result<Vec<u8>, VectorError> {
    validate_cqnt(body)?;
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body)
        .map_err(|error| encode_failed(CQNT, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_CQNT.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_CQNT);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_cqnt(bytes: &[u8]) -> Result<CqntBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_CQNT.len()) else {
        return Err(decode_failed(CQNT, "CQNT magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_CQNT {
        return Err(decode_failed(CQNT, "CQNT magic mismatch"));
    }
    let decoded = rkyv::from_bytes::<CqntBodyV1, rkyv::rancor::Error>(body)
        .map_err(|error| decode_failed(CQNT, format!("rkyv decode failed: {error}")))?;
    validate_cqnt(&decoded)?;
    Ok(decoded)
}

fn validate_cqnt(body: &CqntBodyV1) -> Result<(), VectorError> {
    let CqntBodyV1::Trained {
        k_coarse,
        dim,
        centroids,
    } = body
    else {
        return Ok(());
    };
    if *k_coarse == 0 || *dim == 0 {
        return Err(decode_failed(
            CQNT,
            "CQNT k_coarse and dim must be greater than zero",
        ));
    }
    let expected = (*k_coarse as usize)
        .checked_mul(usize::from(*dim))
        .ok_or_else(|| decode_failed(CQNT, "CQNT centroid length overflow"))?;
    if centroids.len() != expected {
        return Err(decode_failed(
            CQNT,
            format!("CQNT centroid length {} != {expected}", centroids.len()),
        ));
    }
    for (index, value) in centroids.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                CQNT,
                format!("non-finite CQNT centroid component at index {index}: {value}"),
            ));
        }
    }
    Ok(())
}
