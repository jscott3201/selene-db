//! Exact native vector search over graph node properties.

use std::{cmp::Ordering, time::Duration};

use rayon::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{
    CancellationCause, CancellationChecker, CoreError, IStr, NodeId, Value, VectorMetric,
    VectorMetricQuery, VectorTopK, VectorValue,
};

use crate::error::{GraphError, GraphResult};
use crate::graph::SeleneGraph;
use crate::shared::SharedGraph;
use crate::store::RowIndex;
use crate::vector_index::{HnswSearchScratch, VectorIndex, VectorIndexSearchHit};

const VECTOR_SEARCH_CANCEL_STRIDE: usize = 1024;
const VECTOR_SEARCH_PARALLEL_CHUNK_ROWS: usize = 2048;

#[cfg(not(test))]
const VECTOR_SEARCH_PARALLEL_MIN_ROWS: u64 = 16_384;
#[cfg(test)]
const VECTOR_SEARCH_PARALLEL_MIN_ROWS: u64 = 8;

/// Exact vector-search result for a graph node.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorNodeSearchHit {
    /// Stable external node identifier.
    pub node_id: NodeId,
    /// Lower-is-better score under the requested [`VectorMetric`].
    pub distance: f64,
}

/// Tunable options for approximate vector search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApproximateVectorSearchOptions {
    /// Distance metric requested by the caller.
    pub metric: VectorMetric,
    /// Maximum result count.
    pub k: usize,
    /// ANN search-width hint.
    pub ef_search: usize,
}

impl ApproximateVectorSearchOptions {
    /// Construct approximate vector-search options.
    #[must_use]
    pub const fn new(metric: VectorMetric, k: usize, ef_search: usize) -> Self {
        Self {
            metric,
            k,
            ef_search,
        }
    }
}

/// Error returned by cancellation-aware vector search.
#[derive(Debug, thiserror::Error)]
pub enum VectorSearchError {
    /// Graph storage or metric error.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Caller-requested cancellation was observed.
    #[error("vector search cancelled")]
    Cancelled,
    /// Statement deadline elapsed while vector search was scanning.
    #[error("vector search deadline exceeded after {elapsed:?}")]
    Timeout {
        /// Duration since the deadline elapsed.
        elapsed: Duration,
    },
    /// Approximate search was requested without a matching ANN index.
    #[error("matching ANN vector index not found")]
    ApproximateIndexMissing,
    /// Approximate search metric does not match the ANN index metric.
    #[error("ANN vector index was built for {indexed:?}, not requested {requested:?}")]
    ApproximateMetricMismatch {
        /// Metric stored in the ANN index.
        indexed: VectorMetric,
        /// Metric requested by the caller.
        requested: VectorMetric,
    },
}

impl VectorSearchError {
    fn into_graph_error(self) -> GraphError {
        match self {
            Self::Graph(error) => error,
            Self::Cancelled | Self::Timeout { .. } => GraphError::Cancelled,
            Self::ApproximateIndexMissing | Self::ApproximateMetricMismatch { .. } => {
                GraphError::Inconsistent {
                    reason: self.to_string(),
                }
            }
        }
    }
}

impl From<CancellationCause> for VectorSearchError {
    fn from(cause: CancellationCause) -> Self {
        match cause {
            CancellationCause::Cancelled => Self::Cancelled,
            CancellationCause::Timeout { elapsed } => Self::Timeout { elapsed },
        }
    }
}

impl SeleneGraph {
    /// Exhaustively rank vector-valued node properties for one label.
    ///
    /// This is the correctness oracle and small-corpus path for future ANN
    /// indexes: it scans the row bitmap for `label`, skips nodes where
    /// `property` is absent or not a vector, and returns the exact best `k`
    /// matches. Graph structural inconsistencies are reported as
    /// [`GraphError::Inconsistent`]; vector metric errors such as dimension
    /// mismatch propagate through [`GraphError::Core`].
    pub fn exact_vector_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.exact_vector_search_nodes_checked(
            label,
            property,
            query,
            metric,
            k,
            CancellationChecker::disabled(),
        )
        .map_err(VectorSearchError::into_graph_error)
    }

    /// Exhaustively rank vector-valued node properties with cancellation checks.
    ///
    /// This preserves the exact ordering and filtering contract of
    /// [`Self::exact_vector_search_nodes`] while checking `checker` before the
    /// scan and every 1024 candidate rows thereafter. It is the preferred path
    /// for GQL procedure execution because a large exact scan should remain
    /// cooperatively cancellable until ANN indexes take over this surface.
    pub fn exact_vector_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(label_rows) = self.nodes_with_label(label) else {
            return Ok(Vec::new());
        };
        let query_dimension = u32::try_from(query.dimension()).ok();
        let vector_index = query_dimension.and_then(|dimension| {
            self.vector_index_for(label, property)
                .filter(|index| index.dimension() == dimension)
        });
        let rows = vector_index
            .as_ref()
            .map_or(label_rows, |index| index.rows());
        let scorer = metric.bind_query(query).map_err(GraphError::from)?;
        if should_parallelize_exact_scan(vector_index.as_deref(), rows, k, checker) {
            return self.exact_vector_search_indexed_parallel(label, property, scorer, k, rows);
        }

        let mut top_k = VectorTopK::new(k);
        let mut rows_since_check = 0usize;
        for raw_row in rows.iter() {
            rows_since_check += 1;
            if rows_since_check >= VECTOR_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                rows_since_check = 0;
            }
            if !self.node_store.is_alive(raw_row) {
                continue;
            }
            let row = RowIndex::new(raw_row);
            let node_id = self
                .node_id_for_row(row)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "label index row {raw_row} for {} has no node id",
                        label.as_str()
                    ),
                })?;
            let properties = self
                .node_store
                .properties
                .get(raw_row as usize)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "label index row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            let distance = scorer.distance(vector).map_err(GraphError::from)?;
            top_k.push_distance(node_id, distance);
        }

        Ok(top_k
            .into_hits()
            .into_iter()
            .map(|hit| VectorNodeSearchHit {
                node_id: hit.key,
                distance: hit.distance,
            })
            .collect())
    }

    /// Exhaustively rank vector-valued node properties for a batch of queries.
    ///
    /// The output position corresponds to the input query position. This keeps
    /// the exact single-query semantics but resolves the row set once and scans
    /// candidates once, which is useful for agent-memory workloads that probe
    /// several embeddings over the same `(label, property)` surface.
    pub fn exact_vector_search_nodes_batch_checked(
        &self,
        label: &IStr,
        property: &IStr,
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
        let mut top_ks: Vec<_> = queries.iter().map(|_| VectorTopK::new(k)).collect();

        let mut rows_since_check = 0usize;
        for raw_row in rows.iter() {
            rows_since_check += 1;
            if rows_since_check >= VECTOR_SEARCH_CANCEL_STRIDE {
                checker.check()?;
                rows_since_check = 0;
            }
            if !self.node_store.is_alive(raw_row) {
                continue;
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
                continue;
            };
            for (scorer, top_k) in scorers.iter().zip(&mut top_ks) {
                let distance = scorer.distance(vector).map_err(GraphError::from)?;
                top_k.push_distance(node_id, distance);
            }
        }

        Ok(top_ks.into_iter().map(vector_node_hits).collect())
    }

    fn exact_vector_search_indexed_parallel(
        &self,
        label: &IStr,
        property: &IStr,
        scorer: VectorMetricQuery<'_>,
        k: usize,
        rows: &RoaringBitmap,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        let raw_rows: Vec<u32> = rows.iter().collect();
        let top_k = raw_rows
            .par_chunks(VECTOR_SEARCH_PARALLEL_CHUNK_ROWS)
            .map(|chunk| self.exact_vector_search_indexed_chunk(label, property, scorer, k, chunk))
            .try_reduce(|| VectorTopK::new(k), merge_top_k)?;

        Ok(vector_node_hits(top_k))
    }

    fn exact_vector_search_indexed_chunk(
        &self,
        label: &IStr,
        property: &IStr,
        scorer: VectorMetricQuery<'_>,
        k: usize,
        rows: &[u32],
    ) -> Result<VectorTopK<NodeId>, VectorSearchError> {
        let mut top_k = VectorTopK::new(k);
        for &raw_row in rows {
            if !self.node_store.is_alive(raw_row) {
                continue;
            }
            let row = RowIndex::new(raw_row);
            let node_id = self
                .node_id_for_row(row)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "vector index row {raw_row} for {} has no node id",
                        label.as_str()
                    ),
                })?;
            let properties = self
                .node_store
                .properties
                .get(raw_row as usize)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!(
                        "vector index row {raw_row} for {} has no property row",
                        label.as_str()
                    ),
                })?;
            let Some(Value::Vector(vector)) = properties.get(property) else {
                continue;
            };
            let distance = scorer.distance(vector).map_err(GraphError::from)?;
            top_k.push_distance(node_id, distance);
        }
        Ok(top_k)
    }

    /// Approximately rank vector-valued node properties through an ANN index.
    ///
    /// This is intentionally separate from [`Self::exact_vector_search_nodes`]:
    /// it requires a registered ANN vector index whose dimension and metric
    /// match the query, then returns the approximate result set produced by that
    /// derived index. Distances are exact for the candidates the ANN index
    /// returns, but recall is governed by `ef_search`.
    pub fn approximate_vector_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        checker.check()?;
        let query_dimension = u32::try_from(query.dimension())
            .map_err(|_| VectorSearchError::ApproximateIndexMissing)?;
        let Some(index) = self
            .vector_index_for(label, property)
            .filter(|index| index.dimension() == query_dimension)
        else {
            return Err(VectorSearchError::ApproximateIndexMissing);
        };
        let Some(indexed_metric) = index.ann_metric() else {
            return Err(VectorSearchError::ApproximateIndexMissing);
        };
        if indexed_metric != options.metric {
            return Err(VectorSearchError::ApproximateMetricMismatch {
                indexed: indexed_metric,
                requested: options.metric,
            });
        }
        let row_hits = index
            .ann_search(query, options.k, options.ef_search)
            .ok_or(VectorSearchError::ApproximateIndexMissing)?
            .map_err(GraphError::from)?;

        ann_row_hits_to_node_hits(self, label, row_hits, &checker)
    }

    /// Run approximate ANN vector search for a batch of queries.
    ///
    /// The result at each output position corresponds to the query at the same
    /// input position and follows the same ordering, visibility, and error
    /// contract as [`Self::approximate_vector_search_nodes_checked`]. The batch
    /// path resolves the ANN index once and reuses scratch buffers where the
    /// algorithm supports them, making it the preferred native API when a caller has several
    /// independent embedding lookups over the same `(label, property)` index.
    pub fn approximate_vector_search_nodes_batch_checked(
        &self,
        label: &IStr,
        property: &IStr,
        queries: &[VectorValue],
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        checker.check()?;
        let Some(first_query) = queries.first() else {
            return Ok(Vec::new());
        };
        let query_dimension = u32::try_from(first_query.dimension())
            .map_err(|_| VectorSearchError::ApproximateIndexMissing)?;
        let Some(index) = self
            .vector_index_for(label, property)
            .filter(|index| index.dimension() == query_dimension)
        else {
            return Err(VectorSearchError::ApproximateIndexMissing);
        };
        let Some(indexed_metric) = index.ann_metric() else {
            return Err(VectorSearchError::ApproximateIndexMissing);
        };
        if indexed_metric != options.metric {
            return Err(VectorSearchError::ApproximateMetricMismatch {
                indexed: indexed_metric,
                requested: options.metric,
            });
        }

        let mut scratch = HnswSearchScratch::default();
        let mut batch_hits = Vec::with_capacity(queries.len());
        for query in queries {
            checker.check()?;
            let dimension = u32::try_from(query.dimension())
                .map_err(|_| VectorSearchError::ApproximateIndexMissing)?;
            if dimension != query_dimension {
                return Err(VectorSearchError::ApproximateIndexMissing);
            }
            let row_hits = index
                .ann_search_with_scratch(query, options.k, options.ef_search, &mut scratch)
                .ok_or(VectorSearchError::ApproximateIndexMissing)?
                .map_err(GraphError::from)?;

            batch_hits.push(ann_row_hits_to_node_hits(self, label, row_hits, &checker)?);
        }

        Ok(batch_hits)
    }
}

fn should_parallelize_exact_scan(
    vector_index: Option<&VectorIndex>,
    rows: &RoaringBitmap,
    k: usize,
    checker: CancellationChecker<'_>,
) -> bool {
    vector_index.is_some()
        && checker.is_disabled()
        && k != 0
        && rows.len() >= VECTOR_SEARCH_PARALLEL_MIN_ROWS
}

fn merge_top_k(
    mut lhs: VectorTopK<NodeId>,
    rhs: VectorTopK<NodeId>,
) -> Result<VectorTopK<NodeId>, VectorSearchError> {
    for hit in rhs.into_hits() {
        lhs.push_distance(hit.key, hit.distance);
    }
    Ok(lhs)
}

fn vector_node_hits(top_k: VectorTopK<NodeId>) -> Vec<VectorNodeSearchHit> {
    top_k
        .into_hits()
        .into_iter()
        .map(|hit| VectorNodeSearchHit {
            node_id: hit.key,
            distance: hit.distance,
        })
        .collect()
}

fn ann_row_hits_to_node_hits(
    graph: &SeleneGraph,
    label: &IStr,
    row_hits: Vec<VectorIndexSearchHit>,
    checker: &CancellationChecker<'_>,
) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
    let mut hits = Vec::with_capacity(row_hits.len());
    let mut needs_sort = false;
    for hit in row_hits {
        checker.check()?;
        if !graph.node_store.is_alive(hit.row) {
            continue;
        }
        let row = RowIndex::new(hit.row);
        let node_id = graph
            .node_id_for_row(row)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!(
                    "ANN vector index row {} for {} has no node id",
                    hit.row,
                    label.as_str()
                ),
            })?;
        let node_hit = VectorNodeSearchHit {
            node_id,
            distance: hit.distance,
        };
        needs_sort |= hits
            .last()
            .is_some_and(|previous| compare_node_search_hit(previous, &node_hit).is_gt());
        hits.push(node_hit);
    }
    if needs_sort {
        hits.sort_by(compare_node_search_hit);
    }
    Ok(hits)
}

fn compare_node_search_hit(lhs: &VectorNodeSearchHit, rhs: &VectorNodeSearchHit) -> Ordering {
    lhs.distance
        .total_cmp(&rhs.distance)
        .then_with(|| lhs.node_id.cmp(&rhs.node_id))
}

impl SharedGraph {
    /// Exhaustively rank vector-valued node properties in the current snapshot.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::exact_vector_search_nodes`], so the result is lock-free
    /// with respect to concurrent writers once the snapshot pointer is read.
    pub fn exact_vector_search_nodes(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
    ) -> GraphResult<Vec<VectorNodeSearchHit>> {
        self.read()
            .exact_vector_search_nodes(label, property, query, metric, k)
    }

    /// Exhaustively rank vector-valued node properties with cancellation checks.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::exact_vector_search_nodes_checked`].
    pub fn exact_vector_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .exact_vector_search_nodes_checked(label, property, query, metric, k, checker)
    }

    /// Lock-free read snapshot wrapper for
    /// [`SeleneGraph::exact_vector_search_nodes_batch_checked`].
    pub fn exact_vector_search_nodes_batch_checked(
        &self,
        label: &IStr,
        property: &IStr,
        queries: &[VectorValue],
        metric: VectorMetric,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read()
            .exact_vector_search_nodes_batch_checked(label, property, queries, metric, k, checker)
    }

    /// Approximately rank vector-valued node properties through an ANN index.
    ///
    /// This loads one immutable snapshot and delegates to
    /// [`SeleneGraph::approximate_vector_search_nodes_checked`].
    pub fn approximate_vector_search_nodes_checked(
        &self,
        label: &IStr,
        property: &IStr,
        query: &VectorValue,
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<VectorNodeSearchHit>, VectorSearchError> {
        self.read()
            .approximate_vector_search_nodes_checked(label, property, query, options, checker)
    }

    /// Lock-free read snapshot wrapper for
    /// [`SeleneGraph::approximate_vector_search_nodes_batch_checked`].
    pub fn approximate_vector_search_nodes_batch_checked(
        &self,
        label: &IStr,
        property: &IStr,
        queries: &[VectorValue],
        options: ApproximateVectorSearchOptions,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<Vec<VectorNodeSearchHit>>, VectorSearchError> {
        self.read().approximate_vector_search_nodes_batch_checked(
            label, property, queries, options, checker,
        )
    }
}

#[cfg(test)]
#[path = "vector_search/ann_conversion_tests.rs"]
mod ann_conversion_tests;
#[cfg(test)]
#[path = "vector_search/batch_tests.rs"]
mod batch_tests;
#[cfg(test)]
#[path = "vector_search/recall_tests.rs"]
mod recall_tests;
#[cfg(test)]
#[path = "vector_search/tests.rs"]
mod tests;
