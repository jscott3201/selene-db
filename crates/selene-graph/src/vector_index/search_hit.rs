//! Shared row-level search hits for derived vector indexes.

use super::hnsw::HnswVectorHit;
use super::ivf::IvfVectorHit;

/// A row-level hit returned by a derived ANN vector index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorIndexSearchHit {
    pub(crate) row: u32,
    pub(crate) distance: f64,
}

pub(crate) fn hnsw_hits(hits: Vec<HnswVectorHit>) -> Vec<VectorIndexSearchHit> {
    hits.into_iter()
        .map(|hit| VectorIndexSearchHit {
            row: hit.row,
            distance: hit.distance,
        })
        .collect()
}

pub(crate) fn ivf_hits(hits: Vec<IvfVectorHit>) -> Vec<VectorIndexSearchHit> {
    hits.into_iter()
        .map(|hit| VectorIndexSearchHit {
            row: hit.row,
            distance: hit.distance,
        })
        .collect()
}
