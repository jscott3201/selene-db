//! Graph vector-index adapter methods for IVF.

use selene_core::{CoreResult, VectorValue};

use super::search_hit::ivf_hits;
use super::{VectorIndex, VectorIndexSearchHit};

impl VectorIndex {
    pub(crate) fn ivf_candidates_batch(
        &self,
        queries: &[VectorValue],
        k: usize,
        search_width: usize,
    ) -> Option<CoreResult<Vec<Vec<VectorIndexSearchHit>>>> {
        self.ivf.as_ref().map(|ivf| {
            ivf.search_batch(queries, k, search_width)
                .map(|batches| batches.into_iter().map(ivf_hits).collect())
        })
    }
}
