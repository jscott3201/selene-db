//! Rank-fusion helpers for retrieval candidate lists.
//!
//! Reciprocal Rank Fusion combines independently ranked candidate lists without
//! requiring their original score scales to be comparable. A candidate receives
//! `weight / (rank_constant + rank)` for each ranking that contains it, where
//! `rank` is one-based.

use std::collections::{BTreeMap, BTreeSet};

use selene_core::NodeId;
use thiserror::Error;

/// Default Reciprocal Rank Fusion smoothing constant.
pub const DEFAULT_RRF_RANK_CONSTANT: f64 = 60.0;

/// One fused Reciprocal Rank Fusion result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReciprocalRankFusionHit {
    /// Candidate node id.
    pub node_id: NodeId,
    /// Higher-is-better fused RRF score.
    pub score: f64,
}

/// Caller errors for Reciprocal Rank Fusion.
#[derive(Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReciprocalRankFusionError {
    /// The rank constant is not positive and finite.
    #[error("rank_constant must be positive and finite")]
    InvalidRankConstant,

    /// The supplied weight list length differs from the rankings length.
    #[error("weights length {weights} must match rankings length {rankings}")]
    WeightCountMismatch {
        /// Number of ranking lists.
        rankings: usize,
        /// Number of weights.
        weights: usize,
    },

    /// A weight is negative or non-finite.
    #[error("weights[{index}] must be non-negative and finite")]
    InvalidWeight {
        /// Offending weight index.
        index: usize,
    },
}

/// Fuse ranked node lists with Reciprocal Rank Fusion.
///
/// Rankings are consumed in caller order and candidates are ranked by first
/// occurrence within each input list. Duplicate nodes in one ranking contribute
/// only their first occurrence. The output is sorted by fused score descending,
/// then by `NodeId` ascending for deterministic ties.
pub fn reciprocal_rank_fusion(
    rankings: &[Vec<NodeId>],
    weights: Option<&[f64]>,
    rank_constant: f64,
    k: usize,
) -> Result<Vec<ReciprocalRankFusionHit>, ReciprocalRankFusionError> {
    if !rank_constant.is_finite() || rank_constant <= 0.0 {
        return Err(ReciprocalRankFusionError::InvalidRankConstant);
    }
    if k == 0 || rankings.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(weights) = weights {
        validate_weights(weights, rankings.len())?;
    }

    let mut scores = BTreeMap::<NodeId, f64>::new();
    for (ranking_index, ranking) in rankings.iter().enumerate() {
        let weight = weights.map_or(1.0, |weights| weights[ranking_index]);
        if weight == 0.0 {
            continue;
        }
        let mut seen = BTreeSet::new();
        for (rank_index, node_id) in ranking.iter().enumerate() {
            if !seen.insert(*node_id) {
                continue;
            }
            let rank = rank_index as f64 + 1.0;
            *scores.entry(*node_id).or_insert(0.0) += weight / (rank_constant + rank);
        }
    }

    let mut hits: Vec<_> = scores
        .into_iter()
        .map(|(node_id, score)| ReciprocalRankFusionHit { node_id, score })
        .collect();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then(left.node_id.cmp(&right.node_id))
    });
    hits.truncate(k);
    Ok(hits)
}

fn validate_weights(weights: &[f64], rankings_len: usize) -> Result<(), ReciprocalRankFusionError> {
    if weights.len() != rankings_len {
        return Err(ReciprocalRankFusionError::WeightCountMismatch {
            rankings: rankings_len,
            weights: weights.len(),
        });
    }
    for (index, weight) in weights.iter().copied().enumerate() {
        if !weight.is_finite() || weight < 0.0 {
            return Err(ReciprocalRankFusionError::InvalidWeight { index });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(raw: u64) -> NodeId {
        NodeId::new(raw)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "actual {actual} differs from expected {expected}"
        );
    }

    #[test]
    fn overlapping_candidates_gain_rank_fusion_support() {
        let hits = reciprocal_rank_fusion(
            &[vec![node(1), node(2)], vec![node(2), node(3)]],
            None,
            DEFAULT_RRF_RANK_CONSTANT,
            3,
        )
        .unwrap();

        assert_eq!(
            hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
            [node(2), node(1), node(3)]
        );
        assert_close(
            hits[0].score,
            1.0 / (DEFAULT_RRF_RANK_CONSTANT + 2.0) + 1.0 / (DEFAULT_RRF_RANK_CONSTANT + 1.0),
        );
    }

    #[test]
    fn duplicates_within_one_ranking_contribute_first_occurrence_only() {
        let hits = reciprocal_rank_fusion(
            &[vec![node(1), node(1), node(2)]],
            None,
            DEFAULT_RRF_RANK_CONSTANT,
            2,
        )
        .unwrap();

        assert_eq!(
            hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
            [node(1), node(2)]
        );
        assert_close(hits[0].score, 1.0 / (DEFAULT_RRF_RANK_CONSTANT + 1.0));
        assert_close(hits[1].score, 1.0 / (DEFAULT_RRF_RANK_CONSTANT + 3.0));
    }

    #[test]
    fn weights_scale_or_elide_rankings() {
        let hits = reciprocal_rank_fusion(
            &[vec![node(1), node(2)], vec![node(2), node(3)]],
            Some(&[0.0, 2.0]),
            DEFAULT_RRF_RANK_CONSTANT,
            3,
        )
        .unwrap();

        assert_eq!(
            hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
            [node(2), node(3)]
        );
        assert_close(hits[0].score, 2.0 / (DEFAULT_RRF_RANK_CONSTANT + 1.0));
    }

    #[test]
    fn ties_break_by_node_id() {
        let hits = reciprocal_rank_fusion(
            &[vec![node(7)], vec![node(3)]],
            None,
            DEFAULT_RRF_RANK_CONSTANT,
            2,
        )
        .unwrap();

        assert_eq!(
            hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>(),
            [node(3), node(7)]
        );
    }

    #[test]
    fn rejects_invalid_parameters() {
        assert_eq!(
            reciprocal_rank_fusion(&[vec![node(1)]], None, 0.0, 1).unwrap_err(),
            ReciprocalRankFusionError::InvalidRankConstant
        );
        assert_eq!(
            reciprocal_rank_fusion(&[vec![node(1)]], Some(&[1.0, 2.0]), 60.0, 1).unwrap_err(),
            ReciprocalRankFusionError::WeightCountMismatch {
                rankings: 1,
                weights: 2
            }
        );
        assert_eq!(
            reciprocal_rank_fusion(&[vec![node(1)]], Some(&[-1.0]), 60.0, 1).unwrap_err(),
            ReciprocalRankFusionError::InvalidWeight { index: 0 }
        );
    }
}
