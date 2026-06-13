//! Memory and structural diagnostics for vector indexes.

const BASIS_POINTS_DENOMINATOR: usize = 10_000;

/// Minimum pending IVF entries before a rebuild recommendation can fire.
///
/// The first retrain-policy bench showed that very small novel clusters can
/// be too little mass for retrained width-2 partitions even when the ratio is
/// non-zero. This floor keeps the diagnostic from recommending tiny retrains.
pub const IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES: usize = 100;

/// Minimum pending IVF retrain ratio, scaled by 10,000, for rebuild advice.
pub const IVF_REBUILD_PENDING_RETRAIN_BASIS_POINTS: usize = 100;

/// Estimated resident memory and cardinality details for one vector index.
///
/// This is intentionally an estimate rather than allocator-exact accounting.
/// `estimated_index_bytes` counts index-owned structures and excludes primary
/// graph vector component allocations that ANN indexes may share through `Arc`
/// handles. `estimated_reachable_bytes` adds the component bytes referenced by
/// derived entries and centroids as an upper-bound view; deleted ANN entries can
/// retain old component storage until the derived index is rebuilt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VectorIndexMemoryUsage {
    /// Number of live rows currently admitted to the index.
    pub indexed_rows: u64,
    /// Estimated heap bytes owned by the row bitmap.
    pub row_bitmap_bytes: usize,
    /// Roaring serialized size for the row bitmap.
    pub row_bitmap_serialized_bytes: usize,
    /// Estimated heap bytes owned by the HNSW derived index, excluding vector components.
    pub hnsw_index_bytes: usize,
    /// Component bytes reachable through HNSW vector handles.
    pub hnsw_referenced_vector_bytes: usize,
    /// Total HNSW entries, including stale deleted row versions.
    pub hnsw_entries: usize,
    /// Live HNSW entries reachable from row membership.
    pub hnsw_live_entries: usize,
    /// Stale HNSW entries retained for traversability after update/delete.
    pub hnsw_deleted_entries: usize,
    /// Stored directed HNSW links across all layers.
    pub hnsw_link_count: usize,
    /// Stored directed HNSW links in the level-0 layer.
    pub hnsw_level_zero_link_count: usize,
    /// Stored directed HNSW links above the level-0 layer.
    pub hnsw_upper_layer_link_count: usize,
    /// Maximum HNSW layer count attached to any indexed entry.
    pub hnsw_max_layer_count: usize,
    /// Maximum directed HNSW links stored in a single entry layer.
    pub hnsw_max_links_per_layer: usize,
    /// Average directed HNSW links per entry, scaled by 10,000.
    pub hnsw_average_links_per_entry_basis_points: usize,
    /// Estimated heap bytes owned by the IVF derived index, excluding vector components.
    pub ivf_index_bytes: usize,
    /// Component bytes reachable through IVF vector handles.
    pub ivf_referenced_vector_bytes: usize,
    /// Total IVF entries, including stale deleted row versions.
    pub ivf_entries: usize,
    /// Live IVF entries reachable from row membership.
    pub ivf_live_entries: usize,
    /// Stale IVF entries retained until the derived index is rebuilt.
    pub ivf_deleted_entries: usize,
    /// Number of trained IVF centroids.
    pub ivf_centroids: usize,
    /// Number of IVF inverted lists.
    pub ivf_list_count: usize,
    /// Number of IVF inverted lists with at least one assigned live entry.
    pub ivf_non_empty_list_count: usize,
    /// Maximum assigned live entries in one IVF inverted list.
    pub ivf_max_list_len: usize,
    /// Average assigned live entries per IVF inverted list, scaled by 10,000.
    pub ivf_average_list_len_basis_points: usize,
    /// Non-stale IVF entries assigned to inverted lists.
    pub ivf_assigned_entries: usize,
    /// Live IVF entries whose current vector was inserted or replaced after centroid training.
    pub ivf_pending_retrain_entries: usize,
    /// Estimated heap bytes owned by the TurboQuant derived index, excluding vector components.
    pub turbo_quant_index_bytes: usize,
    /// Component bytes reachable through TurboQuant-owned full-vector handles.
    pub turbo_quant_referenced_vector_bytes: usize,
    /// Total TurboQuant compressed entries.
    pub turbo_quant_entries: usize,
    /// Live TurboQuant row entries.
    pub turbo_quant_live_entries: usize,
    /// Stale TurboQuant entries retained by the derived index.
    ///
    /// TurboQuant compacts deletes and replacements immediately, so this should
    /// normally remain zero.
    pub turbo_quant_deleted_entries: usize,
    /// Packed TurboQuant coordinate-code bytes.
    pub turbo_quant_code_bytes: usize,
    /// TurboQuant scalar codebook bytes.
    pub turbo_quant_codebook_bytes: usize,
    /// TurboQuant per-dimension calibration bytes.
    pub turbo_quant_calibration_bytes: usize,
    /// Estimated bytes for index-owned structures, excluding referenced vector components.
    pub estimated_index_bytes: usize,
    /// Estimated upper-bound bytes reachable from the index including ANN vector components.
    pub estimated_reachable_bytes: usize,
}

impl VectorIndexMemoryUsage {
    /// Return pending IVF retrain entries divided by live IVF entries, scaled by 10,000.
    #[must_use]
    pub fn ivf_pending_retrain_basis_points(&self) -> usize {
        self.ivf_pending_retrain_entries
            .saturating_mul(BASIS_POINTS_DENOMINATOR)
            .checked_div(self.ivf_live_entries)
            .unwrap_or_default()
    }

    /// Return true when the IVF index should be rebuilt by maintenance soon.
    ///
    /// The recommendation is deliberately diagnostic only: reads never rebuild
    /// indexes, and callers still decide when to run `selene.rebuild_vector_indexes`.
    /// Deleted IVF entries are not part of this first trigger because delete
    /// maintenance already unlinks them from inverted lists; existing reclaimed
    /// counters expose that memory-only pressure separately.
    #[must_use]
    pub fn ivf_rebuild_recommended(&self) -> bool {
        self.ivf_pending_retrain_entries >= IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES
            && self.ivf_pending_retrain_basis_points() >= IVF_REBUILD_PENDING_RETRAIN_BASIS_POINTS
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES, IVF_REBUILD_PENDING_RETRAIN_BASIS_POINTS,
        VectorIndexMemoryUsage,
    };

    #[test]
    fn ivf_pending_retrain_ratio_uses_live_entries() {
        let usage = VectorIndexMemoryUsage {
            ivf_live_entries: 10_000,
            ivf_pending_retrain_entries: 100,
            ..VectorIndexMemoryUsage::default()
        };

        assert_eq!(
            usage.ivf_pending_retrain_basis_points(),
            IVF_REBUILD_PENDING_RETRAIN_BASIS_POINTS
        );
    }

    #[test]
    fn ivf_rebuild_recommendation_requires_ratio_and_floor() {
        let below_floor = VectorIndexMemoryUsage {
            ivf_live_entries: 1_000,
            ivf_pending_retrain_entries: IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES - 1,
            ..VectorIndexMemoryUsage::default()
        };
        let below_ratio = VectorIndexMemoryUsage {
            ivf_live_entries: 20_000,
            ivf_pending_retrain_entries: IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES,
            ..VectorIndexMemoryUsage::default()
        };
        let recommended = VectorIndexMemoryUsage {
            ivf_live_entries: 10_000,
            ivf_pending_retrain_entries: IVF_REBUILD_MIN_PENDING_RETRAIN_ENTRIES,
            ..VectorIndexMemoryUsage::default()
        };

        assert!(!below_floor.ivf_rebuild_recommended());
        assert!(!below_ratio.ivf_rebuild_recommended());
        assert!(recommended.ivf_rebuild_recommended());
    }
}
