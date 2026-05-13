//! Provider-config-aware IVF snapshot validation.

use std::collections::HashSet;
use std::sync::Arc;

use selene_core::NodeId;

use super::RawVector;
use super::posting::PostingList;
use crate::quantize::PqCodebook;
use crate::{DistanceMetric, IvfConfig, VectorError};

pub(crate) fn deferred_rows_to_raw(
    unassigned_rows: Vec<(NodeId, Vec<f32>)>,
    config: &IvfConfig,
) -> Result<Vec<RawVector>, VectorError> {
    let mut seen = HashSet::with_capacity(unassigned_rows.len());
    let mut rows = Vec::with_capacity(unassigned_rows.len());
    for (node_id, vector) in unassigned_rows {
        validate_vector(node_id, &vector, config)?;
        if !seen.insert(node_id) {
            return Err(VectorError::DuplicateNodeId { node_id });
        }
        rows.push(RawVector {
            node_id,
            vector: Arc::from(vector),
        });
    }
    Ok(rows)
}

pub(crate) fn validate_trained_codebook(
    codebook: &PqCodebook,
    config: &IvfConfig,
) -> Result<(), VectorError> {
    if codebook.m_subspaces as usize != config.pq.m_subspaces {
        return Err(inconsistent(
            "IPQB m_subspaces disagrees with provider config",
        ));
    }
    if codebook.dim() != config.dim {
        return Err(inconsistent(
            "IPQB dimensions disagree with provider config",
        ));
    }
    if codebook.k_centroids != config.pq.k_centroids {
        return Err(inconsistent(
            "IPQB k_centroids disagrees with provider config",
        ));
    }
    Ok(())
}

pub(crate) fn validate_vector(
    node_id: NodeId,
    vector: &[f32],
    config: &IvfConfig,
) -> Result<(), VectorError> {
    if node_id == NodeId::TOMBSTONE {
        return Err(VectorError::InvalidNodeId {
            node_id,
            reason: "NodeId::TOMBSTONE cannot be added to an IVF index".into(),
        });
    }
    if vector.len() != config.dim {
        return Err(VectorError::DimensionsLocked {
            expected: config.dim,
            observed: vector.len(),
        });
    }
    for (index, value) in vector.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(VectorError::NonFiniteVectorComponent {
                node_id,
                index,
                value,
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_posting_lists(
    posting_lists: &[PostingList],
    node_count: u32,
    config: &IvfConfig,
    codebook: &PqCodebook,
) -> Result<(), VectorError> {
    if posting_lists.len() != config.k_coarse as usize {
        return Err(inconsistent(
            "POST posting list count disagrees with k_coarse",
        ));
    }
    let mut observed = 0_u32;
    let mut seen = HashSet::new();
    let codebook_m = codebook.m_subspaces as usize;
    let codebook_k = codebook.k_centroids as usize;
    for (expected_id, list) in posting_lists.iter().enumerate() {
        if list.centroid_id as usize != expected_id {
            return Err(inconsistent("POST centroid ids must be dense and ordered"));
        }
        for entry in &list.entries {
            observed = observed
                .checked_add(1)
                .ok_or_else(|| inconsistent("POST node_count overflow"))?;
            if !seen.insert(entry.node_id) {
                return Err(VectorError::DuplicateNodeId {
                    node_id: entry.node_id,
                });
            }
            if entry.codes.len() != codebook_m {
                return Err(inconsistent(
                    "POST code length disagrees with IPQB m_subspaces",
                ));
            }
            if entry
                .codes
                .iter()
                .copied()
                .any(|code| usize::from(code) >= codebook_k)
            {
                return Err(inconsistent(
                    "POST code byte exceeds IPQB k_centroids range",
                ));
            }
            let needs_norm = config.metric == DistanceMetric::Cosine;
            if entry.reconstructed_norm.is_some() != needs_norm {
                return Err(inconsistent(
                    "POST reconstructed_norm presence disagrees with metric",
                ));
            }
            if let Some(norm) = entry.reconstructed_norm
                && (!norm.is_finite() || norm < 0.0)
            {
                return Err(inconsistent(
                    "POST reconstructed_norm must be finite and nonnegative",
                ));
            }
        }
    }
    if observed != node_count {
        return Err(inconsistent(
            "POST node_count disagrees with posting entries",
        ));
    }
    Ok(())
}

pub(crate) fn inconsistent(reason: impl Into<String>) -> VectorError {
    VectorError::IvfSectionInconsistent {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::posting::{PostingEntry, PostingList};
    use super::*;
    use crate::{DistanceMetric, PqParams};

    fn config() -> IvfConfig {
        IvfConfig::with_params(
            2,
            1,
            1,
            DistanceMetric::L2,
            PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors: 256,
            },
            256,
        )
        .unwrap()
    }

    #[test]
    fn codebook_m_subspaces_must_match_provider_config() {
        let codebook = PqCodebook::from_parts(2, 256, 1, vec![0.0; 512]);

        let err = validate_trained_codebook(&codebook, &config()).unwrap_err();

        assert!(matches!(err, VectorError::IvfSectionInconsistent { .. }));
    }

    #[test]
    fn posting_codes_must_be_less_than_codebook_centroid_count() {
        let codebook = PqCodebook::from_parts(1, 64, 2, vec![0.0; 128]);
        let posting_lists = vec![PostingList {
            centroid_id: 0,
            entries: vec![PostingEntry {
                node_id: NodeId::new(7),
                codes: vec![200].into_boxed_slice(),
                reconstructed_norm: None,
            }],
        }];

        let err = validate_posting_lists(&posting_lists, 1, &config(), &codebook).unwrap_err();

        assert!(matches!(err, VectorError::IvfSectionInconsistent { .. }));
    }

    #[test]
    fn deferred_rows_reject_duplicate_node_ids() {
        let rows = vec![
            (NodeId::new(9), vec![1.0, 2.0]),
            (NodeId::new(9), vec![3.0, 4.0]),
        ];

        let err = deferred_rows_to_raw(rows, &config()).unwrap_err();

        assert!(matches!(err, VectorError::DuplicateNodeId { .. }));
    }
}
