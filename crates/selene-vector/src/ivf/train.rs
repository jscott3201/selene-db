//! IVF-PQ training and residual encoding.

use std::sync::Arc;

use selene_core::NodeId;

use crate::clustering::kmeans_train;
use crate::hnsw::distance::dot_product;
use crate::quantize::PqCodebook;
use crate::{DistanceMetric, IvfConfig, VectorError};

use super::coarse::CoarseQuantizer;
use super::posting::{PostingEntry, PostingList, empty_posting_lists};
use super::{RawVector, TrainedIvf};

/// Seed used for deterministic coarse k-means training.
pub(crate) const IVF_COARSE_SEED: u64 = 0xB67E_0001_u64;

/// Seed used for deterministic residual PQ training.
pub(crate) const IVF_RESIDUAL_SEED: u64 = 0xB67E_0002_u64;

pub(crate) fn train(rows: &[RawVector], config: &IvfConfig) -> Result<TrainedIvf, VectorError> {
    if rows.len() < config.training_min_vectors {
        return Err(VectorError::IvfTrainingDeferred {
            observed_vectors: rows.len(),
            required: config.training_min_vectors,
        });
    }
    let refs = rows
        .iter()
        .map(|row| row.vector.as_ref())
        .collect::<Vec<_>>();
    let centroids = kmeans_train(
        &refs,
        config.dim,
        config.k_coarse as usize,
        IVF_COARSE_SEED,
        "coarse_quantizer",
    )?;
    let coarse = Arc::new(CoarseQuantizer::new(centroids, config)?);
    let residuals = rows
        .iter()
        .map(|row| residual_for(&coarse, row.vector.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    let residual_refs = residuals.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let codebook = Arc::new(PqCodebook::train(
        config.dim,
        config.pq,
        &residual_refs,
        IVF_RESIDUAL_SEED,
        "ivf_residual_pq",
    )?);
    let posting_lists = encode_rows(rows, &coarse, &codebook, config)?;
    Ok(TrainedIvf {
        coarse,
        codebook,
        posting_lists,
    })
}

pub(crate) fn encode_rows(
    rows: &[RawVector],
    coarse: &CoarseQuantizer,
    codebook: &PqCodebook,
    config: &IvfConfig,
) -> Result<Vec<PostingList>, VectorError> {
    let mut posting_lists = empty_posting_lists(coarse.k_coarse);
    for row in rows {
        append_encoded(
            &mut posting_lists,
            coarse,
            codebook,
            row.node_id,
            &row.vector,
            config,
        )?;
    }
    Ok(posting_lists)
}

pub(crate) fn append_encoded(
    posting_lists: &mut [PostingList],
    coarse: &CoarseQuantizer,
    codebook: &PqCodebook,
    node_id: NodeId,
    vector: &[f32],
    config: &IvfConfig,
) -> Result<(), VectorError> {
    let centroid_id = coarse.assign(vector)?;
    let centroid = coarse
        .centroid(centroid_id)
        .ok_or_else(|| VectorError::IvfTrainingFailed {
            context: "posting_assignment",
            reason: format!("missing centroid {centroid_id}"),
        })?;
    let residual = subtract(vector, centroid);
    let mut codes = Vec::with_capacity(codebook.m_subspaces as usize);
    codebook.encode_row(&residual, &mut codes);
    let reconstructed_norm = if config.metric == DistanceMetric::Cosine {
        let mut decoded = vec![0.0; config.dim];
        codebook.decode_codes(&codes, &mut decoded).ok_or_else(|| {
            VectorError::IvfTrainingFailed {
                context: "ivf_residual_decode",
                reason: "code length does not match codebook".into(),
            }
        })?;
        for (slot, center) in decoded.iter_mut().zip(centroid) {
            *slot += *center;
        }
        Some(dot_product(&decoded, &decoded).sqrt())
    } else {
        None
    };
    let entry = PostingEntry {
        node_id,
        codes: codes.into_boxed_slice(),
        reconstructed_norm,
    };
    let list = posting_lists.get_mut(centroid_id as usize).ok_or_else(|| {
        VectorError::IvfTrainingFailed {
            context: "posting_assignment",
            reason: format!("centroid id {centroid_id} exceeds posting list count"),
        }
    })?;
    list.push(entry);
    Ok(())
}

pub(crate) fn residual_for(
    coarse: &CoarseQuantizer,
    vector: &[f32],
) -> Result<Vec<f32>, VectorError> {
    let centroid_id = coarse.assign(vector)?;
    let centroid = coarse
        .centroid(centroid_id)
        .ok_or_else(|| VectorError::IvfTrainingFailed {
            context: "coarse_quantizer",
            reason: format!("missing centroid {centroid_id}"),
        })?;
    Ok(subtract(vector, centroid))
}

fn subtract(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).map(|(a, b)| a - b).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::NodeId;

    use crate::{DistanceMetric, IvfConfig, PqParams};

    use super::*;

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
            },
            256,
        )
        .unwrap()
    }

    fn rows() -> Vec<RawVector> {
        (0..256)
            .map(|idx| RawVector {
                node_id: NodeId::new(idx + 1),
                vector: Arc::from([idx as f32, (idx % 7) as f32]),
            })
            .collect()
    }

    #[test]
    fn ivf_train_seeds_pinned() {
        assert_eq!(IVF_COARSE_SEED, 0xB67E_0001_u64);
        assert_eq!(IVF_RESIDUAL_SEED, 0xB67E_0002_u64);
    }

    #[test]
    fn residual_pq_training_is_deterministic_given_seed() {
        let config = config();
        let rows = rows();

        let left = train(&rows, &config).unwrap();
        let right = train(&rows, &config).unwrap();

        assert_eq!(left.coarse.centroids, right.coarse.centroids);
        assert_eq!(left.codebook.centroids, right.codebook.centroids);
        assert_eq!(left.posting_lists, right.posting_lists);
    }
}
