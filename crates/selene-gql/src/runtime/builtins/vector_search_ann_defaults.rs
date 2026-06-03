//! Shared ANN vector-search defaults for native vector built-ins.

use selene_core::{IStr, Value, VectorMetric};
use selene_graph::SeleneGraph;

use crate::procedure_registry::ProcedureError;

/// Default HNSW search width for omitted ANN procedure arguments.
pub(super) const DEFAULT_HNSW_SEARCH_WIDTH: usize = 64;
/// Default IVF probe/list width for omitted ANN procedure arguments.
pub(super) const DEFAULT_IVF_SEARCH_WIDTH: usize = 2;
/// Planner-visible documentation for the executable `NULL` default.
pub(super) const SEARCH_WIDTH_DEFAULT_DOC: &str = "NULL (HNSW 64, IVF 2)";

/// Parse an optional ANN search-width value.
pub(super) fn optional_search_width_arg(
    proc_name: &str,
    value: &Value,
) -> Result<Option<usize>, ProcedureError> {
    match value {
        Value::Null => Ok(None),
        Value::Int(value) if *value >= 0 => usize::try_from(*value)
            .map(Some)
            .map_err(|_| search_width_too_large(proc_name)),
        Value::Uint(value) => usize::try_from(*value)
            .map(Some)
            .map_err(|_| search_width_too_large(proc_name)),
        _ => Err(ProcedureError::InvalidArgument {
            detail: format!("{proc_name} ef_search must be NULL or a non-negative INTEGER"),
        }),
    }
}

/// Resolve the omitted ANN search-width default from the registered index kind.
pub(super) fn default_search_width(
    graph: &SeleneGraph,
    label: &IStr,
    property: &IStr,
    query_dimension: usize,
    metric: VectorMetric,
) -> usize {
    let Ok(query_dimension) = u32::try_from(query_dimension) else {
        return DEFAULT_HNSW_SEARCH_WIDTH;
    };
    let Some(index) = graph
        .vector_index_for(label, property)
        .filter(|index| index.dimension() == query_dimension)
    else {
        return DEFAULT_HNSW_SEARCH_WIDTH;
    };
    if index.ann_metric() == Some(metric) && index.is_ivf() {
        DEFAULT_IVF_SEARCH_WIDTH
    } else {
        DEFAULT_HNSW_SEARCH_WIDTH
    }
}

fn search_width_too_large(proc_name: &str) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: format!("{proc_name} ef_search is too large for this platform"),
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{GraphId, VectorMetric, intern};
    use selene_graph::{SharedGraph, VectorIndexKind};

    use super::{DEFAULT_HNSW_SEARCH_WIDTH, DEFAULT_IVF_SEARCH_WIDTH, default_search_width};

    fn graph_with_index(kind: VectorIndexKind) -> SharedGraph {
        let graph = SharedGraph::new(GraphId::new(431_001));
        let label = intern("VectorDoc").expect("label interns");
        let property = intern("embedding").expect("property interns");
        let mut txn = graph.begin_write();
        txn.mutator()
            .create_vector_index(label, property, kind, 2)
            .expect("vector index creates");
        txn.commit().expect("index creation commits");
        graph
    }

    #[test]
    fn default_search_width_selects_ivf_width_for_matching_ivf_index() {
        let graph = graph_with_index(VectorIndexKind::IvfSquaredEuclidean);
        let label = intern("VectorDoc").expect("label interns");
        let property = intern("embedding").expect("property interns");
        let snapshot = graph.read();

        assert_eq!(
            default_search_width(
                &snapshot,
                &label,
                &property,
                2,
                VectorMetric::SquaredEuclidean
            ),
            DEFAULT_IVF_SEARCH_WIDTH
        );
    }

    #[test]
    fn default_search_width_keeps_hnsw_width_for_matching_hnsw_index() {
        let graph = graph_with_index(VectorIndexKind::HnswSquaredEuclidean);
        let label = intern("VectorDoc").expect("label interns");
        let property = intern("embedding").expect("property interns");
        let snapshot = graph.read();

        assert_eq!(
            default_search_width(
                &snapshot,
                &label,
                &property,
                2,
                VectorMetric::SquaredEuclidean
            ),
            DEFAULT_HNSW_SEARCH_WIDTH
        );
    }

    #[test]
    fn default_search_width_keeps_hnsw_width_without_matching_ivf_index() {
        let graph = graph_with_index(VectorIndexKind::IvfCosine);
        let label = intern("VectorDoc").expect("label interns");
        let property = intern("embedding").expect("property interns");
        let snapshot = graph.read();

        assert_eq!(
            default_search_width(&snapshot, &label, &property, 3, VectorMetric::Cosine),
            DEFAULT_HNSW_SEARCH_WIDTH
        );
        assert_eq!(
            default_search_width(
                &snapshot,
                &label,
                &property,
                2,
                VectorMetric::SquaredEuclidean
            ),
            DEFAULT_HNSW_SEARCH_WIDTH
        );
    }
}
