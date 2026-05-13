//! Codec for the `IPQB` IVF residual-PQ codebook section.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::quantize::PqCodebook;

use super::{IPQB, decode_failed, encode_failed};

/// Magic prefix for version-1 `IPQB` section bodies.
pub(crate) const PAYLOAD_MAGIC_IPQB: [u8; 4] = *b"VIPB";

/// Archived body for the `IPQB` section.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) enum IpqbBodyV1 {
    /// No residual codebook has been trained.
    Empty,
    /// Trained residual PQ codebook.
    Trained {
        /// Residual PQ codebook.
        codebook: PqCodebook,
    },
}

pub(crate) fn encode_ipqb(body: &IpqbBodyV1) -> Result<Vec<u8>, VectorError> {
    validate_ipqb(body)?;
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body)
        .map_err(|error| encode_failed(IPQB, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
    out.extend_from_slice(&archived);
    Ok(out)
}

pub(crate) fn decode_ipqb(bytes: &[u8]) -> Result<IpqbBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_IPQB.len()) else {
        return Err(decode_failed(IPQB, "IPQB magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_IPQB {
        return Err(decode_failed(IPQB, "IPQB magic mismatch"));
    }
    let decoded = rkyv::from_bytes::<IpqbBodyV1, rkyv::rancor::Error>(body)
        .map_err(|error| decode_failed(IPQB, format!("rkyv decode failed: {error}")))?;
    validate_ipqb(&decoded)?;
    Ok(decoded)
}

fn validate_ipqb(body: &IpqbBodyV1) -> Result<(), VectorError> {
    let IpqbBodyV1::Trained { codebook } = body else {
        return Ok(());
    };
    let m = codebook.m_subspaces as usize;
    let k = codebook.k_centroids as usize;
    let subdim = codebook.subspace_dim as usize;
    if m == 0 || k == 0 || subdim == 0 {
        return Err(decode_failed(
            IPQB,
            "IPQB m_subspaces, k_centroids, and subspace_dim must be greater than zero",
        ));
    }
    let expected = m
        .checked_mul(k)
        .and_then(|value| value.checked_mul(subdim))
        .ok_or_else(|| decode_failed(IPQB, "IPQB codebook length overflow"))?;
    if codebook.centroids.len() != expected {
        return Err(decode_failed(
            IPQB,
            format!(
                "IPQB codebook length {} != {expected}",
                codebook.centroids.len()
            ),
        ));
    }
    for (index, value) in codebook.centroids.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                IPQB,
                format!("non-finite IPQB component at index {index}: {value}"),
            ));
        }
    }
    Ok(())
}
