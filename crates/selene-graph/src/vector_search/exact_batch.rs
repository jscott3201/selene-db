use roaring::RoaringBitmap;
use selene_core::{
    CancellationChecker, CoreError, DbString, NodeId, Value, VectorMetric, VectorMetricQuery,
    VectorTopK, VectorValue, vector_squared_norm,
};

use super::{
    VECTOR_SEARCH_CANCEL_STRIDE, VECTOR_SEARCH_PARALLEL_CHUNK_ROWS, VectorNodeSearchHit,
    VectorSearchError, should_parallelize_exact_scan, vector_node_hits,
};
use crate::error::GraphError;
use crate::graph::SeleneGraph;
use crate::parallel_scan::try_reduce_bitmap_chunks;
use crate::store::RowIndex;

impl SeleneGraph {
    /// Exhaustively rank vector-valued node properties for a batch of queries.
    ///
    /// The output position corresponds to the input query position. This keeps
    /// the exact single-query semantics but resolves the row set once and scans
    /// candidates once, which is useful for agent-memory workloads that probe
    /// several embeddings over the same `(label, property)` surface.
    pub fn exact_vector_search_nodes_batch_checked(
        &self,
        label: &DbString,
        property: &DbString,
        queries: &[VectorValue],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        checker.check()?;
        let Some(first_query) = queries.first() else {
            return Ok(Vec::new());
        };
        let first_dimension = first_query.dimension();
        for query in &queries[1..] {
            if query.dimension() != first_dimension {
                return Err(GraphError::from(CoreError::VectorDimensionMismatch {
                    lhs: first_dimension,
                    rhs: query.dimension(),
                })
                .into());
            }
        }
        if k == 0 {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        let Some(label_rows) = self.nodes_with_label(label) else {
            return Ok(vec![Vec::new(); queries.len()]);
        };

        let query_dimension = u32::try_from(first_dimension).ok();
        let vector_index = query_dimension.and_then(|dimension| {
            self.vector_index_for(label, property)
                .filter(|index| index.dimension() == dimension)
        });
        let rows = vector_index
            .as_ref()
            .map_or(label_rows, |index| index.rows());
        let scorers: Result<Vec<_>, GraphError> = queries
            .iter()
            .map(|query| metric.bind_query(query).map_err(GraphError::from))
            .collect();
        let scorers = scorers?;
        if should_parallelize_exact_scan(rows, k) {
            return self
                .exact_vector_search_batch_parallel(label, property, &scorers, k, rows, checker);
        }

        let top_ks =
            self.exact_vector_search_batch_serial(label, property, &scorers, k, rows, checker)?;
        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }

    fn exact_vector_search_batch_parallel(
        &self,
        label: &DbString,
        property: &DbString,
        scorers: &[VectorMetricQuery<'_>],
        k: usize,
        rows: &RoaringBitmap,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        let top_ks = try_reduce_bitmap_chunks(
            rows,
            VECTOR_SEARCH_PARALLEL_CHUNK_ROWS,
            checker,
            || new_batch_top_ks(scorers.len(), k),
            |chunk| self.exact_vector_search_batch_chunk(label, property, scorers, k, chunk),
            merge_batch_top_ks,
        )?;

        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }

    fn exact_vector_search_batch_serial(
        &self,
        label: &DbString,
        property: &DbString,
        scorers: &[VectorMetricQuery<'_>],
        k: usize,
        rows: &RoaringBitmap,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorTopK<NodeId>>, VectorSearchError> {
        let mut top_ks = new_batch_top_ks(scorers.len(), k);
        let use_candidate_norms = uses_cosine_metric(scorers);
        let mut rows_since_check = 0usize;
        for raw_row in rows.iter() {
            rows_since_check += 1;
            if rows_since_check >= VECTOR_SEARCH_CANCEL_STRIDE {
                checker.note_nodes_scanned(rows_since_check)?;
                rows_since_check = 0;
            }
            self.push_batch_row(
                label,
                property,
                scorers,
                &mut top_ks,
                raw_row,
                use_candidate_norms,
            )?;
        }
        if rows_since_check > 0 {
            checker.note_nodes_scanned(rows_since_check)?;
        }
        Ok(top_ks)
    }

    fn exact_vector_search_batch_chunk(
        &self,
        label: &DbString,
        property: &DbString,
        scorers: &[VectorMetricQuery<'_>],
        k: usize,
        rows: &[u32],
    ) -> Result<Vec<VectorTopK<NodeId>>, VectorSearchError> {
        let mut top_ks = new_batch_top_ks(scorers.len(), k);
        let use_candidate_norms = uses_cosine_metric(scorers);
        for &raw_row in rows {
            self.push_batch_row(
                label,
                property,
                scorers,
                &mut top_ks,
                raw_row,
                use_candidate_norms,
            )?;
        }
        Ok(top_ks)
    }

    fn push_batch_row(
        &self,
        label: &DbString,
        property: &DbString,
        scorers: &[VectorMetricQuery<'_>],
        top_ks: &mut [VectorTopK<NodeId>],
        raw_row: u32,
        use_candidate_norms: bool,
    ) -> Result<(), VectorSearchError> {
        if !self.node_store.is_alive(raw_row) {
            return Ok(());
        }
        let row = RowIndex::new(raw_row);
        let node_id = self
            .node_id_for_row(row)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!(
                    "vector search row {raw_row} for {} has no node id",
                    label.as_str()
                ),
            })?;
        let properties = self
            .node_store
            .properties
            .get(raw_row as usize)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!(
                    "vector search row {raw_row} for {} has no property row",
                    label.as_str()
                ),
            })?;
        let Some(Value::Vector(vector)) = properties.get(property) else {
            return Ok(());
        };
        if use_candidate_norms {
            let candidate_squared_norm = vector_squared_norm(vector);
            for (scorer, top_k) in scorers.iter().zip(top_ks) {
                let distance = scorer
                    .distance_with_candidate_squared_norm(vector, candidate_squared_norm)
                    .map_err(GraphError::from)?;
                top_k.push_distance(node_id, distance);
            }
        } else {
            for (scorer, top_k) in scorers.iter().zip(top_ks) {
                let distance = scorer.distance(vector).map_err(GraphError::from)?;
                top_k.push_distance(node_id, distance);
            }
        }
        Ok(())
    }
}

fn uses_cosine_metric(scorers: &[VectorMetricQuery<'_>]) -> bool {
    scorers
        .first()
        .is_some_and(|scorer| scorer.metric() == VectorMetric::Cosine)
}

fn new_batch_top_ks(query_count: usize, k: usize) -> Vec<VectorTopK<NodeId>> {
    (0..query_count).map(|_| VectorTopK::new(k)).collect()
}

fn merge_batch_top_ks(
    mut lhs: Vec<VectorTopK<NodeId>>,
    rhs: Vec<VectorTopK<NodeId>>,
) -> Result<Vec<VectorTopK<NodeId>>, VectorSearchError> {
    debug_assert_eq!(lhs.len(), rhs.len());
    for (lhs_top_k, rhs_top_k) in lhs.iter_mut().zip(rhs) {
        for hit in rhs_top_k.into_hits() {
            lhs_top_k.push_distance(hit.key, hit.distance);
        }
    }
    Ok(lhs)
}
