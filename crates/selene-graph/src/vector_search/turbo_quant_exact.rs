//! Exact fallbacks for TurboQuant searches that cover every candidate row.

use roaring::RoaringBitmap;

use super::{ApproximateVectorSearchOptions, VectorIndexSearchHit};

pub(super) fn covers_rows(rows: &RoaringBitmap, options: ApproximateVectorSearchOptions) -> bool {
    let candidate_limit = options.ef_search.max(options.k);
    !rows.is_empty() && u64::try_from(candidate_limit).unwrap_or(u64::MAX) >= rows.len()
}

pub(super) fn row_hits(rows: &RoaringBitmap) -> Vec<VectorIndexSearchHit> {
    rows.iter()
        .map(|row| VectorIndexSearchHit { row, distance: 0.0 })
        .collect()
}
