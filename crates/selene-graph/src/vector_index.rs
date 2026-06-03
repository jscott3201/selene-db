//! Built-in vector row-set indexes for node properties.
//!
//! Vector indexes are durable registrations plus derived in-memory accelerators
//! over primary node values. Every kind keeps a row bitmap for alive nodes whose
//! `(label, property)` value is a vector with the declared dimension; HNSW kinds
//! also maintain an approximate neighbor graph. Registration and live
//! maintenance are strict so search cannot hide dimensionality or metric drift;
//! recovery rebuild remains lenient for corrupted/legacy state and is checked by
//! the debug consistency net.

use std::collections::BTreeSet;
use std::mem::size_of;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
#[path = "vector_index/hnsw.rs"]
mod hnsw;

use selene_core::{IStr, LabelSet, PropertyMap, Value, VectorMetric, VectorValue};
use serde::{Deserialize, Serialize};

use crate::error::{GraphError, GraphResult};
use crate::graph::VectorIndexEntry;
use hnsw::{HnswVectorHit, HnswVectorIndex};

type VectorIndexMap = FxHashMap<(IStr, IStr), VectorIndexEntry>;

struct VectorIndexRegistration {
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    name: Option<IStr>,
}

/// Vector index algorithm kind.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum VectorIndexKind {
    /// Exact row-set index over full-precision vectors stored on graph rows.
    Flat,
    /// Approximate HNSW index using squared Euclidean distance.
    HnswSquaredEuclidean,
    /// Approximate HNSW index using cosine distance.
    HnswCosine,
    /// Approximate HNSW index using negative inner product distance.
    HnswNegativeInnerProduct,
}

impl VectorIndexKind {
    /// Return the HNSW metric for approximate vector-index kinds.
    #[must_use]
    pub const fn hnsw_metric(self) -> Option<VectorMetric> {
        match self {
            Self::Flat => None,
            Self::HnswSquaredEuclidean => Some(VectorMetric::SquaredEuclidean),
            Self::HnswCosine => Some(VectorMetric::Cosine),
            Self::HnswNegativeInnerProduct => Some(VectorMetric::NegativeInnerProduct),
        }
    }
}

/// Estimated resident memory and cardinality details for one vector index.
///
/// This is intentionally an estimate rather than allocator-exact accounting.
/// `estimated_index_bytes` counts index-owned structures and excludes primary
/// graph vector component allocations that HNSW may share through `Arc` handles.
/// `estimated_reachable_bytes` adds the component bytes referenced by HNSW
/// entries as an upper-bound view; deleted HNSW entries can retain old component
/// storage until the derived index is rebuilt.
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
    /// Estimated bytes for index-owned structures, excluding referenced vector components.
    pub estimated_index_bytes: usize,
    /// Estimated upper-bound bytes reachable from the index including HNSW vector components.
    pub estimated_reachable_bytes: usize,
}

/// Built-in vector index state for one `(label, property)` registration.
#[derive(Clone, Debug)]
pub struct VectorIndex {
    kind: VectorIndexKind,
    dimension: u32,
    rows: RoaringBitmap,
    hnsw: Option<HnswVectorIndex>,
}

impl VectorIndex {
    /// Construct an empty vector index.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::VectorIndexInvalidDimension`] when `dimension` is
    /// zero.
    pub fn new(kind: VectorIndexKind, dimension: u32) -> GraphResult<Self> {
        ensure_dimension(dimension)?;
        let hnsw = match kind {
            VectorIndexKind::Flat => None,
            VectorIndexKind::HnswSquaredEuclidean => {
                Some(HnswVectorIndex::new(VectorMetric::SquaredEuclidean))
            }
            VectorIndexKind::HnswCosine => Some(HnswVectorIndex::new(VectorMetric::Cosine)),
            VectorIndexKind::HnswNegativeInnerProduct => {
                Some(HnswVectorIndex::new(VectorMetric::NegativeInnerProduct))
            }
        };
        Ok(Self {
            kind,
            dimension,
            rows: RoaringBitmap::new(),
            hnsw,
        })
    }

    /// Return the vector index algorithm kind.
    #[must_use]
    pub const fn kind(&self) -> VectorIndexKind {
        self.kind
    }

    /// Return the required vector dimensionality.
    #[must_use]
    pub const fn dimension(&self) -> u32 {
        self.dimension
    }

    /// Return the number of indexed rows.
    #[must_use]
    pub fn cardinality(&self) -> u64 {
        self.rows.len()
    }

    /// Borrow the indexed row bitmap.
    #[must_use]
    pub const fn rows(&self) -> &RoaringBitmap {
        &self.rows
    }

    /// Return true when this index has an ANN graph.
    #[must_use]
    pub const fn is_hnsw(&self) -> bool {
        self.kind.hnsw_metric().is_some()
    }

    /// Return the HNSW metric, if this is an HNSW index.
    #[must_use]
    pub const fn hnsw_metric(&self) -> Option<VectorMetric> {
        self.kind.hnsw_metric()
    }

    /// Return an estimated memory usage snapshot for this index.
    #[must_use]
    pub fn memory_usage(&self) -> VectorIndexMemoryUsage {
        let row_bitmap_bytes = roaring_heap_bytes(&self.rows);
        let row_bitmap_serialized_bytes = self.rows.serialized_size();
        let hnsw = self
            .hnsw
            .as_ref()
            .map(HnswVectorIndex::memory_usage)
            .unwrap_or_default();
        let estimated_index_bytes = size_of::<Self>()
            .saturating_add(row_bitmap_bytes)
            .saturating_add(hnsw.estimated_heap_bytes);
        VectorIndexMemoryUsage {
            indexed_rows: self.cardinality(),
            row_bitmap_bytes,
            row_bitmap_serialized_bytes,
            hnsw_index_bytes: hnsw.estimated_heap_bytes,
            hnsw_referenced_vector_bytes: hnsw.referenced_vector_bytes,
            hnsw_entries: hnsw.entries,
            hnsw_live_entries: hnsw.live_entries,
            hnsw_deleted_entries: hnsw.deleted_entries,
            hnsw_link_count: hnsw.link_count,
            estimated_index_bytes,
            estimated_reachable_bytes: estimated_index_bytes
                .saturating_add(hnsw.referenced_vector_bytes),
        }
    }

    pub(crate) fn insert_value(&mut self, row: u32, vector: &VectorValue) -> GraphResult<()> {
        self.rows.insert(row);
        if let Some(hnsw) = &mut self.hnsw {
            hnsw.insert(row, vector.clone())?;
        }
        Ok(())
    }

    pub(crate) fn remove_row(&mut self, row: u32) {
        self.rows.remove(row);
        if let Some(hnsw) = &mut self.hnsw {
            hnsw.remove(row);
        }
    }

    pub(crate) fn rows_eq(&self, reference: &Self) -> bool {
        self.kind == reference.kind
            && self.dimension == reference.dimension
            && self.rows == reference.rows
    }

    pub(crate) fn hnsw_search(
        &self,
        query: &VectorValue,
        k: usize,
        ef_search: usize,
    ) -> Option<selene_core::CoreResult<Vec<HnswVectorHit>>> {
        self.hnsw
            .as_ref()
            .map(|hnsw| hnsw.search(query, k, ef_search))
    }
}

fn roaring_heap_bytes(rows: &RoaringBitmap) -> usize {
    let statistics = rows.statistics();
    u64_to_usize_saturating(
        statistics
            .n_bytes_array_containers
            .saturating_add(statistics.n_bytes_run_containers)
            .saturating_add(statistics.n_bytes_bitset_containers),
    )
}

/// Error returned when a value cannot be admitted to a vector index.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VectorIndexValueError {
    /// Value is not a vector.
    #[error("kind mismatch: observed {observed}")]
    KindMismatch {
        /// Observed value kind.
        observed: &'static str,
    },
    /// Vector dimensionality differs from the registration.
    #[error("dimension mismatch: expected {expected}, observed {observed}")]
    DimensionMismatch {
        /// Expected vector dimensionality.
        expected: u32,
        /// Observed vector dimensionality.
        observed: usize,
    },
    /// Vector is structurally valid but invalid for the index metric.
    #[error("metric rejection: {observed}")]
    MetricRejected {
        /// Observed metric rejection reason.
        observed: String,
    },
}

impl VectorIndexValueError {
    fn observed(&self) -> String {
        match self {
            Self::KindMismatch { observed } => (*observed).to_owned(),
            Self::DimensionMismatch { observed, .. } => format!("VECTOR<{observed}>"),
            Self::MetricRejected { observed } => observed.clone(),
        }
    }
}

pub(crate) fn apply_node_create(
    indexes: &mut VectorIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            insert_commit(indexes, label.clone(), property.clone(), value, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_delete(
    indexes: &mut VectorIndexMap,
    labels: &LabelSet,
    props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    for label in labels.iter() {
        for (property, value) in props.iter() {
            if is_null(value) {
                continue;
            }
            remove_commit(indexes, label.clone(), property.clone(), value, row)?;
        }
    }
    Ok(())
}

pub(crate) fn apply_node_update(
    indexes: &mut VectorIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
    row: u32,
) -> GraphResult<()> {
    let candidates = candidate_keys(indexes, old_labels, old_props, new_labels, new_props);
    for (label, property) in candidates {
        if let Some(value) = indexable_value(old_labels, old_props, &label, &property) {
            remove_commit(indexes, label.clone(), property.clone(), value, row)?;
        }
        if let Some(value) = indexable_value(new_labels, new_props, &label, &property) {
            insert_commit(indexes, label.clone(), property.clone(), value, row)?;
        }
    }
    Ok(())
}

/// Build a vector index strictly at registration time.
pub(crate) fn build_vector_index(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
) -> GraphResult<VectorIndex> {
    build_vector_index_inner(graph, label, property, kind, dimension, BuildPolicy::Strict)
}

/// Build a vector index leniently for recovery/snapshot rebuild.
pub(crate) fn build_vector_index_lenient(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
) -> GraphResult<VectorIndex> {
    build_vector_index_inner(
        graph,
        label,
        property,
        kind,
        dimension,
        BuildPolicy::Lenient,
    )
}

/// Rebuild every registered vector index from node columns.
pub(crate) fn rebuild_vector_indexes(graph: &mut crate::SeleneGraph) -> GraphResult<()> {
    let registrations: Vec<VectorIndexRegistration> = graph
        .vector_index
        .iter()
        .map(|((label, property), entry)| VectorIndexRegistration {
            label: label.clone(),
            property: property.clone(),
            kind: entry.kind(),
            dimension: entry.dimension(),
            name: entry.name.clone(),
        })
        .collect();
    graph.vector_index.clear();
    for registration in registrations {
        let index = build_vector_index_lenient(
            graph,
            registration.label.clone(),
            registration.property.clone(),
            registration.kind,
            registration.dimension,
        )?;
        graph.vector_index.insert(
            (registration.label, registration.property),
            VectorIndexEntry::new(index, registration.name),
        );
    }
    Ok(())
}

fn build_vector_index_inner(
    graph: &crate::SeleneGraph,
    label: IStr,
    property: IStr,
    kind: VectorIndexKind,
    dimension: u32,
    policy: BuildPolicy,
) -> GraphResult<VectorIndex> {
    let mut index = VectorIndex::new(kind, dimension)?;
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
                if let Err(err) = index.insert_value(row, vector) {
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

fn candidate_keys(
    indexes: &VectorIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
) -> BTreeSet<(IStr, IStr)> {
    if indexes.is_empty() {
        return BTreeSet::new();
    }
    let mut labels: BTreeSet<IStr> = BTreeSet::new();
    labels.extend(old_labels.iter().cloned());
    labels.extend(new_labels.iter().cloned());

    let mut properties: BTreeSet<IStr> = BTreeSet::new();
    properties.extend(old_props.keys().cloned());
    properties.extend(new_props.keys().cloned());

    let mut candidates = BTreeSet::new();
    for label in &labels {
        for property in &properties {
            let key = (label.clone(), property.clone());
            if indexes.contains_key(&key) {
                candidates.insert(key);
            }
        }
    }
    candidates
}

fn indexable_value<'a>(
    labels: &LabelSet,
    props: &'a PropertyMap,
    label: &IStr,
    property: &IStr,
) -> Option<&'a Value> {
    if !labels.contains(label) {
        return None;
    }
    props.get(property).filter(|value| !is_null(value))
}

fn insert_commit(
    indexes: &mut VectorIndexMap,
    label: IStr,
    property: IStr,
    value: &Value,
    row: u32,
) -> GraphResult<()> {
    if let Some(entry) = indexes.get_mut(&(label.clone(), property.clone())) {
        let vector = admit(value, entry.kind(), entry.dimension())
            .map_err(|err| index_rejection(label, property, entry.dimension(), err))?;
        std::sync::Arc::make_mut(&mut entry.index).insert_value(row, vector)?;
    }
    Ok(())
}

fn remove_commit(
    indexes: &mut VectorIndexMap,
    label: IStr,
    property: IStr,
    value: &Value,
    row: u32,
) -> GraphResult<()> {
    if let Some(entry) = indexes.get_mut(&(label.clone(), property.clone())) {
        admit(value, entry.kind(), entry.dimension())
            .map_err(|err| index_rejection(label, property, entry.dimension(), err))?;
        std::sync::Arc::make_mut(&mut entry.index).remove_row(row);
    }
    Ok(())
}

fn admit(
    value: &Value,
    kind: VectorIndexKind,
    expected_dimension: u32,
) -> Result<&VectorValue, VectorIndexValueError> {
    let Value::Vector(vector) = value else {
        return Err(VectorIndexValueError::KindMismatch {
            observed: value_kind_name(value),
        });
    };
    if vector.dimension() != expected_dimension as usize {
        return Err(VectorIndexValueError::DimensionMismatch {
            expected: expected_dimension,
            observed: vector.dimension(),
        });
    }
    if let Some(metric) = kind.hnsw_metric() {
        metric
            .distance(vector, vector)
            .map_err(|err| VectorIndexValueError::MetricRejected {
                observed: err.to_string(),
            })?;
    }
    Ok(vector)
}

fn ensure_dimension(dimension: u32) -> GraphResult<()> {
    if dimension == 0 {
        Err(GraphError::VectorIndexInvalidDimension { dimension })
    } else {
        Ok(())
    }
}

fn u64_to_usize_saturating(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn index_rejection(
    label: IStr,
    property: IStr,
    expected_dimension: u32,
    err: VectorIndexValueError,
) -> GraphError {
    GraphError::VectorIndexValueRejected {
        label,
        property,
        expected_dimension,
        observed: err.observed(),
    }
}

fn warn_rejected(
    op: &'static str,
    label: IStr,
    property: IStr,
    row: u32,
    err: &VectorIndexValueError,
) {
    tracing::warn!(
        op,
        %label,
        %property,
        row,
        error = %err,
        "skipped vector-index update for value that does not match the registered vector index"
    );
}

const fn is_null(value: &Value) -> bool {
    matches!(value, Value::Null)
}

const fn value_kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Uint(_) => "Uint",
        Value::Int128(_) => "Int128",
        Value::Uint128(_) => "Uint128",
        Value::Float(_) => "Float",
        Value::Float32(_) => "Float32",
        Value::Decimal(_) => "Decimal",
        Value::String(_) => "String",
        Value::Bytes(_) => "Bytes",
        Value::List(_) => "List",
        Value::Record(_) => "Record",
        Value::RecordTyped(_) => "RecordTyped",
        Value::Path(_) => "Path",
        Value::NodeRef(_) => "NodeRef",
        Value::EdgeRef(_) => "EdgeRef",
        Value::GraphRef(_) => "GraphRef",
        Value::TableRef(_) => "TableRef",
        Value::ZonedDateTime(_) => "ZonedDateTime",
        Value::LocalDateTime(_) => "LocalDateTime",
        Value::Date(_) => "Date",
        Value::ZonedTime(_) => "ZonedTime",
        Value::LocalTime(_) => "LocalTime",
        Value::Duration(_) => "Duration",
        Value::Extended { .. } => "Extended",
        Value::Uuid(_) => "Uuid",
        Value::Vector(_) => "Vector",
        _ => "Unknown",
    }
}

#[cfg(test)]
#[path = "vector_index/tests.rs"]
mod tests;
