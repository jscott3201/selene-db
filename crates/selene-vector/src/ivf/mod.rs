//! IVF-PQ vector index provider.

use std::sync::Arc;

use selene_core::NodeId;

use crate::quantize::PqCodebook;

pub mod coarse;
pub mod posting;
mod provider;
pub mod search;
pub mod train;

use coarse::CoarseQuantizer;
use posting::PostingList;
pub use provider::IvfProvider;

/// Provider name used in `Change::IndexExtensionEvent` for IVF-PQ.
pub(crate) const IVF_PROVIDER_NAME: &str = "selene-vector-ivf";

#[derive(Clone, Debug)]
pub(crate) struct RawVector {
    pub(crate) node_id: NodeId,
    pub(crate) vector: Arc<[f32]>,
}

#[derive(Clone, Debug)]
pub(crate) struct TrainedIvf {
    pub(crate) coarse: Arc<CoarseQuantizer>,
    pub(crate) codebook: Arc<PqCodebook>,
    pub(crate) posting_lists: Vec<PostingList>,
}

/// Immutable IVF-PQ index snapshot.
#[derive(Clone, Debug)]
pub struct IvfIndex {
    pub(crate) coarse_quantizer: Option<Arc<CoarseQuantizer>>,
    pub(crate) posting_lists: Vec<PostingList>,
    pub(crate) residual_codebook: Option<Arc<PqCodebook>>,
    pub(crate) unassigned_buffer: Vec<RawVector>,
    pub(crate) dimensions: u16,
    pub(crate) vector_count: usize,
}

impl IvfIndex {
    /// Construct an empty IVF index.
    #[must_use]
    pub fn empty(dimensions: u16) -> Self {
        Self {
            coarse_quantizer: None,
            posting_lists: Vec::new(),
            residual_codebook: None,
            unassigned_buffer: Vec::new(),
            dimensions,
            vector_count: 0,
        }
    }

    pub(crate) fn trained(dimensions: u16, trained: TrainedIvf, vector_count: usize) -> Self {
        Self {
            coarse_quantizer: Some(trained.coarse),
            posting_lists: trained.posting_lists,
            residual_codebook: Some(trained.codebook),
            unassigned_buffer: Vec::new(),
            dimensions,
            vector_count,
        }
    }

    pub(crate) fn deferred(dimensions: u16, rows: Vec<RawVector>) -> Self {
        Self {
            coarse_quantizer: None,
            posting_lists: Vec::new(),
            residual_codebook: None,
            vector_count: rows.len(),
            unassigned_buffer: rows,
            dimensions,
        }
    }

    /// Return the configured vector dimensionality.
    #[must_use]
    pub fn dimensions(&self) -> usize {
        usize::from(self.dimensions)
    }

    /// Return the number of indexed or buffered vectors.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vector_count
    }

    /// Return true when no vectors are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vector_count == 0
    }

    /// Return true when the IVF index has trained centroids and posting lists.
    #[must_use]
    pub fn is_trained(&self) -> bool {
        self.coarse_quantizer.is_some() && self.residual_codebook.is_some()
    }

    pub(crate) fn contains_node(&self, node_id: NodeId) -> bool {
        self.unassigned_buffer
            .iter()
            .any(|row| row.node_id == node_id)
            || self
                .posting_lists
                .iter()
                .flat_map(|list| &list.entries)
                .any(|entry| entry.node_id == node_id)
    }
}

/// Lightweight statistics for an IVF-PQ index.
#[derive(Clone, Debug, PartialEq)]
pub enum IvfStats {
    /// Trained IVF-PQ state with posting lists.
    Trained {
        /// Number of coarse centroids.
        k_coarse: u32,
        /// Default probe count.
        n_probe_default: u32,
        /// Per-list entry counts.
        posting_list_lengths: Vec<u32>,
        /// Raw rows accumulated after training.
        unassigned_count: usize,
        /// Bytes used by coarse centroids.
        bytes_coarse_centroids: usize,
        /// Bytes used by residual PQ codebook.
        bytes_residual_codebook: usize,
        /// Bytes used by posting-list PQ codes.
        bytes_posting_lists: usize,
        /// Bytes used by reconstructed norm cache.
        bytes_reconstructed_norms: usize,
        /// Approximate `f32_bytes / compressed_bytes` ratio.
        compression_ratio: f32,
    },
    /// Deferred IVF state below the training threshold.
    Deferred {
        /// Observed buffered vector count.
        observed_vectors: usize,
        /// Required vector count before training may run.
        required: usize,
    },
}
