use roaring::RoaringBitmap;
use selene_core::{CoreResult, VectorValue};

use super::search_hit::turbo_quant_hits;
use super::{VectorIndex, VectorIndexSearchHit};

impl VectorIndex {
    pub(crate) fn turbo_quant_prefers_fused_batch(&self, query_count: usize) -> bool {
        self.turbo_quant
            .as_ref()
            .is_some_and(|turbo_quant| turbo_quant.should_fuse_batch_scan(query_count))
    }

    pub(crate) fn turbo_quant_candidates(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
    ) -> Option<CoreResult<Vec<VectorIndexSearchHit>>> {
        self.turbo_quant.as_ref().map(|turbo_quant| {
            turbo_quant
                .candidates(query, k, search_width)
                .map(turbo_quant_hits)
        })
    }

    pub(crate) fn turbo_quant_candidates_in_rows(
        &self,
        query: &VectorValue,
        k: usize,
        search_width: usize,
        allowed_rows: &RoaringBitmap,
    ) -> Option<CoreResult<Vec<VectorIndexSearchHit>>> {
        self.turbo_quant.as_ref().map(|turbo_quant| {
            turbo_quant
                .candidates_in_rows(query, k, search_width, allowed_rows)
                .map(turbo_quant_hits)
        })
    }

    pub(crate) fn turbo_quant_candidates_batch(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
    ) -> Option<CoreResult<Vec<Vec<VectorIndexSearchHit>>>> {
        self.turbo_quant.as_ref().map(|turbo_quant| {
            turbo_quant
                .candidates_batch(queries, k, search_width)
                .map(|batches| batches.into_iter().map(turbo_quant_hits).collect())
        })
    }

    pub(crate) fn turbo_quant_candidates_batch_in_rows(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
        allowed_rows: &[RoaringBitmap],
    ) -> Option<CoreResult<Vec<Vec<VectorIndexSearchHit>>>> {
        self.turbo_quant.as_ref().map(|turbo_quant| {
            turbo_quant
                .candidates_batch_in_rows(queries, k, search_width, allowed_rows)
                .map(|batches| batches.into_iter().map(turbo_quant_hits).collect())
        })
    }
}
