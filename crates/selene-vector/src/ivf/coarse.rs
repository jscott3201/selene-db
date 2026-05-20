//! Coarse quantizer for IVF-PQ.

use rkyv::{Archive, Deserialize, Serialize};

use crate::clustering::nearest_centroid;
#[cfg(test)]
use crate::clustering::squared_l2;
use crate::{IvfConfig, VectorError};

/// Coarse IVF centroid table.
#[derive(Archive, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CoarseQuantizer {
    /// Centroids laid out centroid-major.
    pub centroids: Vec<f32>,
    /// Number of coarse centroids.
    pub k_coarse: u32,
    /// Vector dimensionality.
    pub dim: u16,
}

impl CoarseQuantizer {
    pub(crate) fn new(centroids: Vec<f32>, config: &IvfConfig) -> Result<Self, VectorError> {
        let expected = (config.k_coarse as usize)
            .checked_mul(config.dim)
            .ok_or_else(|| VectorError::IvfTrainingFailed {
                context: "coarse_quantizer",
                reason: "k_coarse * dim overflow".into(),
            })?;
        if centroids.len() != expected {
            return Err(VectorError::IvfTrainingFailed {
                context: "coarse_quantizer",
                reason: format!("centroid length {} != {expected}", centroids.len()),
            });
        }
        Ok(Self {
            centroids,
            k_coarse: config.k_coarse,
            dim: u16::try_from(config.dim).map_err(|_| VectorError::IvfDimensionMismatch {
                expected: u16::MAX as usize,
                observed: config.dim,
            })?,
        })
    }

    /// Return the nearest coarse centroid id for `vector`.
    ///
    /// # Errors
    ///
    /// Returns [`VectorError::IvfDimensionMismatch`] when `vector` has the
    /// wrong dimensionality.
    pub fn assign(&self, vector: &[f32]) -> Result<u32, VectorError> {
        if vector.len() != usize::from(self.dim) {
            return Err(VectorError::IvfDimensionMismatch {
                expected: usize::from(self.dim),
                observed: vector.len(),
            });
        }
        let assigned = nearest_centroid(
            vector,
            &self.centroids,
            self.k_coarse as usize,
            usize::from(self.dim),
        );
        u32::try_from(assigned).map_err(|_| VectorError::IvfTrainingFailed {
            context: "coarse_quantizer",
            reason: "assigned centroid id overflow".into(),
        })
    }

    pub(crate) fn centroid(&self, centroid_id: u32) -> Option<&[f32]> {
        let dim = usize::from(self.dim);
        let id = usize::try_from(centroid_id).ok()?;
        if id >= self.k_coarse as usize {
            return None;
        }
        let start = id.checked_mul(dim)?;
        self.centroids.get(start..start.checked_add(dim)?)
    }

    /// Return coarse centroids ordered by the L2 assignment frame.
    ///
    /// Posting lists are populated by [`Self::assign`], which uses the L2
    /// coarse partition. Probe traversal must use that same frame for every
    /// final scoring metric; switching Dot/Cosine probe ranking without also
    /// changing assignment can skip the only list containing the best-scoring
    /// candidates.
    #[cfg(test)]
    pub(crate) fn nearest_probes(&self, query: &[f32], n_probe: u32) -> Vec<u32> {
        let dim = usize::from(self.dim);
        let mut scored = (0..self.k_coarse)
            .filter_map(|centroid_id| {
                let centroid = self.centroid(centroid_id)?;
                Some((centroid_id, squared_l2(query, &centroid[..dim])))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        scored
            .into_iter()
            .take(n_probe as usize)
            .map(|(centroid_id, _)| centroid_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DistanceMetric, PqParams};

    fn config() -> IvfConfig {
        IvfConfig::with_params(
            2,
            2,
            1,
            DistanceMetric::L2,
            PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: false,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            },
            256,
        )
        .unwrap()
    }

    #[test]
    fn coarse_assignment_picks_nearest_centroid() {
        let quantizer = CoarseQuantizer::new(vec![0.0, 0.0, 10.0, 0.0], &config()).unwrap();

        assert_eq!(quantizer.assign(&[9.0, 0.0]).unwrap(), 1);
        assert_eq!(quantizer.assign(&[1.0, 0.0]).unwrap(), 0);
    }

    #[test]
    fn nearest_probes_stays_aligned_with_l2_assignment_frame() {
        let cfg = IvfConfig::with_params(
            2,
            3,
            1,
            DistanceMetric::Dot,
            PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: false,
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
            },
            256,
        )
        .unwrap();
        let quantizer = CoarseQuantizer::new(vec![0.9, 0.0, 100.0, 0.0, 0.0, 1.0], &cfg).unwrap();

        let probes = quantizer.nearest_probes(&[1.0, 0.0], 1);

        assert_eq!(probes, vec![0]);
    }
}
