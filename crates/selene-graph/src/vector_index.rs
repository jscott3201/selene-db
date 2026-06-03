//! Built-in vector row-set indexes for node properties.
//!
//! The first vector index kind is intentionally small: a durable registration
//! plus an in-memory row bitmap for alive nodes whose `(label, property)` value
//! is a vector with the declared dimension. Registration and live maintenance
//! are strict so exact search can use the bitmap without hiding dimensionality
//! drift; recovery rebuild remains lenient for corrupted/legacy state and is
//! checked by the debug consistency net. Future ANN structures can hang off the
//! same catalog identity without changing WAL/snapshot DDL.

use std::collections::BTreeSet;

use roaring::RoaringBitmap;
use rustc_hash::FxHashMap;
use selene_core::{IStr, LabelSet, PropertyMap, Value};
use serde::{Deserialize, Serialize};

use crate::error::{GraphError, GraphResult};
use crate::graph::VectorIndexEntry;

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
}

/// Built-in vector index state for one `(label, property)` registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorIndex {
    kind: VectorIndexKind,
    dimension: u32,
    rows: RoaringBitmap,
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
        Ok(Self {
            kind,
            dimension,
            rows: RoaringBitmap::new(),
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

    pub(crate) fn insert_row(&mut self, row: u32) {
        self.rows.insert(row);
    }

    pub(crate) fn remove_row(&mut self, row: u32) {
        self.rows.remove(row);
    }

    pub(crate) fn rows_eq(&self, reference: &Self) -> bool {
        self.kind == reference.kind
            && self.dimension == reference.dimension
            && self.rows == reference.rows
    }
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
}

impl VectorIndexValueError {
    fn observed(&self) -> String {
        match self {
            Self::KindMismatch { observed } => (*observed).to_owned(),
            Self::DimensionMismatch { observed, .. } => format!("VECTOR<{observed}>"),
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
        match admit(value, dimension) {
            Ok(()) => index.insert_row(row),
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
        admit(value, entry.dimension())
            .map_err(|err| index_rejection(label, property, entry.dimension(), err))?;
        std::sync::Arc::make_mut(&mut entry.index).insert_row(row);
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
        admit(value, entry.dimension())
            .map_err(|err| index_rejection(label, property, entry.dimension(), err))?;
        std::sync::Arc::make_mut(&mut entry.index).remove_row(row);
    }
    Ok(())
}

fn admit(value: &Value, expected_dimension: u32) -> Result<(), VectorIndexValueError> {
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
    Ok(())
}

fn ensure_dimension(dimension: u32) -> GraphResult<()> {
    if dimension == 0 {
        Err(GraphError::VectorIndexInvalidDimension { dimension })
    } else {
        Ok(())
    }
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
