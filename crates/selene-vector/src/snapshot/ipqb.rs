//! Codec for the `IPQB` IVF residual-PQ codebook section.

use rkyv::{Archive, Deserialize, Serialize};

use crate::VectorError;
use crate::quantize::linalg;
use crate::quantize::{PqCodebook, PqCodebookV1Legacy, PqCodebookV2Legacy};

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

#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
enum IpqbBodyV1Legacy {
    Empty,
    Trained { codebook: PqCodebookV1Legacy },
}

/// Decode/encode shape from the post-OPQ, pre-polysemous archive. Used so
/// IVF-PQ snapshots produced before the polysemous flag stay byte-identical
/// when `polysemous_trained = false`.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
enum IpqbBodyV2Legacy {
    Empty,
    Trained { codebook: PqCodebookV2Legacy },
}

impl From<IpqbBodyV1Legacy> for IpqbBodyV1 {
    fn from(value: IpqbBodyV1Legacy) -> Self {
        match value {
            IpqbBodyV1Legacy::Empty => Self::Empty,
            IpqbBodyV1Legacy::Trained { codebook } => Self::Trained {
                codebook: codebook.into(),
            },
        }
    }
}

impl From<IpqbBodyV2Legacy> for IpqbBodyV1 {
    fn from(value: IpqbBodyV2Legacy) -> Self {
        match value {
            IpqbBodyV2Legacy::Empty => Self::Empty,
            IpqbBodyV2Legacy::Trained { codebook } => Self::Trained {
                codebook: codebook.into(),
            },
        }
    }
}

pub(crate) fn encode_ipqb(body: &IpqbBodyV1) -> Result<Vec<u8>, VectorError> {
    validate_ipqb(body)?;
    // Why: these historical labels pin the exact byte-parity fixtures this
    // encoder must keep producing for legacy archives.
    // Encode cascade: v1 legacy (BRIEF-66 byte parity) → v2 legacy
    // (BRIEF-68 byte parity) → v3 flag-bearing archive.
    if let Some(bytes) = encode_ipqb_v1_legacy_if_compatible(body)? {
        return Ok(bytes);
    }
    if let Some(bytes) = encode_ipqb_v2_legacy_if_compatible(body)? {
        return Ok(bytes);
    }
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body)
        .map_err(|error| encode_failed(IPQB, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
    out.extend_from_slice(&archived);
    Ok(out)
}

fn encode_ipqb_v1_legacy_if_compatible(body: &IpqbBodyV1) -> Result<Option<Vec<u8>>, VectorError> {
    let legacy = match body {
        IpqbBodyV1::Empty => IpqbBodyV1Legacy::Empty,
        IpqbBodyV1::Trained { codebook } => {
            if codebook.rotation.is_some() || codebook.polysemous_trained {
                return Ok(None);
            }
            IpqbBodyV1Legacy::Trained {
                codebook: PqCodebookV1Legacy {
                    m_subspaces: codebook.m_subspaces,
                    k_centroids: codebook.k_centroids,
                    subspace_dim: codebook.subspace_dim,
                    centroids: codebook.centroids.clone(),
                },
            }
        }
    };
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&legacy)
        .map_err(|error| encode_failed(IPQB, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
    out.extend_from_slice(&archived);
    Ok(Some(out))
}

fn encode_ipqb_v2_legacy_if_compatible(body: &IpqbBodyV1) -> Result<Option<Vec<u8>>, VectorError> {
    let legacy = match body {
        IpqbBodyV1::Empty => IpqbBodyV2Legacy::Empty,
        IpqbBodyV1::Trained { codebook } => {
            let Some(v2) = codebook.as_v2_legacy() else {
                return Ok(None);
            };
            IpqbBodyV2Legacy::Trained { codebook: v2 }
        }
    };
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&legacy)
        .map_err(|error| encode_failed(IPQB, error.to_string()))?;
    let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
    out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
    out.extend_from_slice(&archived);
    Ok(Some(out))
}

pub(crate) fn decode_ipqb(bytes: &[u8]) -> Result<IpqbBodyV1, VectorError> {
    let Some((magic, body)) = bytes.split_at_checked(PAYLOAD_MAGIC_IPQB.len()) else {
        return Err(decode_failed(IPQB, "IPQB magic mismatch"));
    };
    if magic != PAYLOAD_MAGIC_IPQB {
        return Err(decode_failed(IPQB, "IPQB magic mismatch"));
    }
    // Each successive shape decodes from the same `body` bytes; the
    // ambiguity (every encoded `Empty` payload also decodes as `Empty`
    // under wider shapes) is handled by preferring the *widest* decoder
    // that surfaces a `Trained` body. Conversely, a successful `Trained`
    // decode in any shape wins over an `Empty` decode in a wider shape
    // Why: these historical labels identify the fixture generations that must
    // continue to decode from stored bytes.
    // (BRIEF-66/67 byte goldens stay loadable).
    let v3 = rkyv::from_bytes::<IpqbBodyV1, rkyv::rancor::Error>(body);
    let v2 = rkyv::from_bytes::<IpqbBodyV2Legacy, rkyv::rancor::Error>(body).ok();
    let v1 = rkyv::from_bytes::<IpqbBodyV1Legacy, rkyv::rancor::Error>(body).ok();
    let decoded = match (v3, v2, v1) {
        // Pre-polysemous archive resolving as Empty in v3 but Trained in v2:
        // honor the v2 Trained body (carries rotation but not polysemous).
        (Ok(IpqbBodyV1::Empty), Some(IpqbBodyV2Legacy::Trained { codebook }), _) => {
            let decoded: IpqbBodyV1 = IpqbBodyV2Legacy::Trained { codebook }.into();
            validate_ipqb(&decoded)?;
            decoded
        }
        // Pre-OPQ archive resolving as Empty in v3/v2 but Trained in v1
        // legacy: honor the v1 Trained body (no rotation, no flag).
        (Ok(IpqbBodyV1::Empty), _, Some(IpqbBodyV1Legacy::Trained { codebook })) => {
            let decoded: IpqbBodyV1 = IpqbBodyV1Legacy::Trained { codebook }.into();
            validate_ipqb(&decoded)?;
            decoded
        }
        (Ok(decoded), _, _) => {
            validate_ipqb(&decoded)?;
            decoded
        }
        (Err(v3_error), Some(v2_body), _) => {
            let decoded: IpqbBodyV1 = v2_body.into();
            validate_ipqb(&decoded).map_err(|legacy_error| {
                decode_failed(
                    IPQB,
                    format!("rkyv decode failed: v3={v3_error}; v2={legacy_error}"),
                )
            })?;
            decoded
        }
        (Err(v3_error), None, Some(v1_body)) => {
            let decoded: IpqbBodyV1 = v1_body.into();
            validate_ipqb(&decoded).map_err(|legacy_error| {
                decode_failed(
                    IPQB,
                    format!("rkyv decode failed: v3={v3_error}; v1={legacy_error}"),
                )
            })?;
            decoded
        }
        (Err(v3_error), None, None) => {
            return Err(decode_failed(
                IPQB,
                format!("rkyv decode failed: {v3_error}"),
            ));
        }
    };
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
    validate_rotation(codebook.rotation.as_deref(), codebook.dim())?;
    Ok(())
}

fn validate_rotation(rotation: Option<&[f32]>, dim: usize) -> Result<(), VectorError> {
    let Some(rotation) = rotation else {
        return Ok(());
    };
    let expected = dim
        .checked_mul(dim)
        .ok_or_else(|| decode_failed(IPQB, "IPQB OPQ rotation length overflow"))?;
    if rotation.len() != expected {
        return Err(decode_failed(
            IPQB,
            format!(
                "IPQB OPQ rotation length {} disagrees with dim^2 {expected}",
                rotation.len()
            ),
        ));
    }
    for (index, value) in rotation.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(decode_failed(
                IPQB,
                format!("non-finite IPQB OPQ rotation component at index {index}: {value}"),
            ));
        }
    }
    if !linalg::is_orthonormal(rotation, dim, 1.0e-5) {
        return Err(decode_failed(IPQB, "IPQB OPQ rotation is not orthonormal"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_encode(body: &IpqbBodyV1) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
        out.extend_from_slice(&archived);
        out
    }

    fn raw_encode_legacy(body: &IpqbBodyV1Legacy) -> Vec<u8> {
        let archived = rkyv::to_bytes::<rkyv::rancor::Error>(body).expect("raw encode");
        let mut out = Vec::with_capacity(PAYLOAD_MAGIC_IPQB.len() + archived.len());
        out.extend_from_slice(&PAYLOAD_MAGIC_IPQB);
        out.extend_from_slice(&archived);
        out
    }

    #[test]
    fn legacy_ipqb_decodes_to_none_rotation() {
        let legacy = IpqbBodyV1Legacy::Trained {
            codebook: PqCodebookV1Legacy {
                m_subspaces: 1,
                k_centroids: 256,
                subspace_dim: 2,
                centroids: vec![0.0; 512],
            },
        };

        let decoded = decode_ipqb(&raw_encode_legacy(&legacy)).unwrap();

        let IpqbBodyV1::Trained { codebook } = decoded else {
            panic!("legacy trained body should stay trained");
        };
        assert!(codebook.rotation.is_none());
    }

    #[test]
    fn ipqb_decode_rejects_bad_rotation_shape() {
        let body = IpqbBodyV1::Trained {
            codebook: PqCodebook {
                m_subspaces: 1,
                k_centroids: 256,
                subspace_dim: 2,
                centroids: vec![0.0; 512],
                rotation: Some(vec![1.0, 0.0, 0.0]),
                polysemous_trained: false,
            },
        };

        let err = decode_ipqb(&raw_encode(&body)).expect_err("bad rotation rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("rotation length"))
        );
    }

    #[test]
    fn ipqb_decode_rejects_non_orthonormal_rotation() {
        let body = IpqbBodyV1::Trained {
            codebook: PqCodebook {
                m_subspaces: 1,
                k_centroids: 256,
                subspace_dim: 2,
                centroids: vec![0.0; 512],
                rotation: Some(vec![2.0, 0.0, 0.0, 1.0]),
                polysemous_trained: false,
            },
        };

        let err = decode_ipqb(&raw_encode(&body)).expect_err("bad rotation rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("not orthonormal"))
        );
    }

    #[test]
    fn corrupted_v2_ipqb_rotation_does_not_silently_fall_back_to_legacy() {
        let body = IpqbBodyV1::Trained {
            codebook: PqCodebook {
                m_subspaces: 1,
                k_centroids: 256,
                subspace_dim: 2,
                centroids: vec![0.0; 512],
                rotation: Some(vec![2.0, 0.0, 0.0, 1.0]),
                polysemous_trained: false,
            },
        };
        let encoded = raw_encode(&body);

        let err = decode_ipqb(&encoded).expect_err("bad v2 rotation rejected");

        assert!(
            matches!(err, VectorError::SectionDecodeFailed { reason, .. } if reason.contains("not orthonormal"))
        );
    }
}
