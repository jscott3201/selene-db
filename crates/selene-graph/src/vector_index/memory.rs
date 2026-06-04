//! Memory and structural diagnostics for vector indexes.

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
    /// Estimated bytes for index-owned structures, excluding referenced vector components.
    pub estimated_index_bytes: usize,
    /// Estimated upper-bound bytes reachable from the index including ANN vector components.
    pub estimated_reachable_bytes: usize,
}
