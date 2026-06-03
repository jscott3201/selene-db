//! Vector-index build and rebuild helpers.

use selene_core::{HnswIndexConfig, IStr};

use crate::error::{GraphError, GraphResult};
use crate::graph::VectorIndexEntry;

use super::{
    VectorIndex, VectorIndexKind, VectorIndexMap, VectorIndexMemoryUsage, VectorIndexRebuildEntry,
    VectorIndexRebuildReport, admit, hnsw::HnswSearchScratch, index_rejection, is_null,
    warn_rejected,
};

struct VectorIndexRegistration {
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
    name: Option<IStr>,
    before: VectorIndexMemoryUsage,
}

/// Build a vector index strictly with optional HNSW construction config.
pub(crate) fn build_vector_index_with_hnsw_config(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
) -> GraphResult<VectorIndex> {
    build_vector_index_inner(
        graph,
        label,
        property,
        kind,
        dimension,
        hnsw_config,
        BuildPolicy::Strict,
    )
}

/// Build a vector index leniently with optional HNSW construction config.
pub(crate) fn build_vector_index_lenient_with_hnsw_config(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
) -> GraphResult<VectorIndex> {
    build_vector_index_inner(
        graph,
        label,
        property,
        kind,
        dimension,
        hnsw_config,
        BuildPolicy::Lenient,
    )
}

/// Rebuild every registered vector index from node columns.
pub(crate) fn rebuild_vector_indexes(graph: &mut crate::SeleneGraph) -> GraphResult<()> {
    rebuild_vector_indexes_inner(graph, BuildPolicy::Lenient).map(|_| ())
}

/// Strictly rebuild every registered vector index from node columns.
pub(crate) fn rebuild_vector_indexes_strict(
    graph: &mut crate::SeleneGraph,
) -> GraphResult<VectorIndexRebuildReport> {
    rebuild_vector_indexes_inner(graph, BuildPolicy::Strict)
}

fn rebuild_vector_indexes_inner(
    graph: &mut crate::SeleneGraph,
    policy: BuildPolicy,
) -> GraphResult<VectorIndexRebuildReport> {
    let registrations: Vec<VectorIndexRegistration> = graph
        .vector_index
        .iter()
        .map(|((label, property), entry)| VectorIndexRegistration {
            label: label.clone(),
            property: property.clone(),
            kind: entry.kind(),
            dimension: entry.dimension(),
            hnsw_config: entry.hnsw_config(),
            name: entry.name.clone(),
            before: entry.memory_usage(),
        })
        .collect();
    let mut rebuilt = VectorIndexMap::default();
    let mut entries = Vec::with_capacity(registrations.len());
    for registration in registrations {
        let index = build_vector_index_inner(
            graph,
            registration.label.clone(),
            registration.property.clone(),
            registration.kind,
            registration.dimension,
            registration.hnsw_config,
            policy,
        )?;
        let after = index.memory_usage();
        let key = (registration.label.clone(), registration.property.clone());
        rebuilt.insert(key, VectorIndexEntry::new(index, registration.name.clone()));
        entries.push(VectorIndexRebuildEntry {
            label: registration.label,
            property: registration.property,
            name: registration.name,
            kind: registration.kind,
            dimension: registration.dimension,
            hnsw_config: registration.hnsw_config,
            before: registration.before,
            after,
        });
    }
    graph.vector_index = rebuilt;
    Ok(VectorIndexRebuildReport::new(entries))
}

fn build_vector_index_inner(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    hnsw_config: Option<HnswIndexConfig>,
    policy: BuildPolicy,
) -> GraphResult<VectorIndex> {
    let mut index = VectorIndex::new_with_hnsw_config(kind, dimension, hnsw_config)?;
    let mut hnsw_scratch = HnswSearchScratch::default();
    for row_index in 0..graph.node_store.labels.len() {
        let row = u32::try_from(row_index).map_err(|_| GraphError::Inconsistent {
            reason: format!(
                "node store row index {row_index} exceeds u32::MAX; selene-graph caps rows at u32::MAX"
            ),
        })?;
        if !graph.node_store.is_alive(row) {
            continue;
        }
        let Some(labels) = graph.node_store.labels.get(row_index) else {
            continue;
        };
        if !labels.contains(&label) {
            continue;
        }
        let Some(props) = graph.node_store.properties.get(row_index) else {
            continue;
        };
        let Some(value) = props.get(&property) else {
            continue;
        };
        if is_null(value) {
            continue;
        }
        match admit(value, kind, dimension) {
            Ok(vector) => {
                if let Err(err) = index.insert_value_with_scratch(row, vector, &mut hnsw_scratch) {
                    match policy {
                        BuildPolicy::Strict => return Err(err),
                        BuildPolicy::Lenient => {
                            tracing::warn!(
                                row,
                                error = %err,
                                "skipped vector-index HNSW update during lenient rebuild"
                            );
                        }
                    }
                }
            }
            Err(err) => match policy {
                BuildPolicy::Strict => {
                    return Err(index_rejection(
                        label.clone(),
                        property.clone(),
                        dimension,
                        err,
                    ));
                }
                BuildPolicy::Lenient => {
                    warn_rejected("rebuild", label.clone(), property.clone(), row, &err);
                }
            },
        }
    }
    Ok(index)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildPolicy {
    Strict,
    Lenient,
}
