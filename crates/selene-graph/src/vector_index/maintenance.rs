use std::collections::BTreeSet;

use selene_core::{DbString, LabelSet, PropertyMap, Value, VectorValue};

use crate::error::{GraphError, GraphResult};

use super::{VectorIndexKind, VectorIndexMap};

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
        match (
            indexable_value(old_labels, old_props, &label, &property),
            indexable_value(new_labels, new_props, &label, &property),
        ) {
            (Some(old_value), Some(new_value)) => {
                replace_commit(
                    indexes,
                    label.clone(),
                    property.clone(),
                    old_value,
                    new_value,
                    row,
                )?;
            }
            (Some(value), None) => {
                remove_commit(indexes, label.clone(), property.clone(), value, row)?;
            }
            (None, Some(value)) => {
                insert_commit(indexes, label.clone(), property.clone(), value, row)?;
            }
            (None, None) => {}
        }
    }
    Ok(())
}

fn candidate_keys(
    indexes: &VectorIndexMap,
    old_labels: &LabelSet,
    old_props: &PropertyMap,
    new_labels: &LabelSet,
    new_props: &PropertyMap,
) -> BTreeSet<(DbString, DbString)> {
    if indexes.is_empty() {
        return BTreeSet::new();
    }
    let mut labels: BTreeSet<DbString> = BTreeSet::new();
    labels.extend(old_labels.iter().cloned());
    labels.extend(new_labels.iter().cloned());

    let mut properties: BTreeSet<DbString> = BTreeSet::new();
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
    label: &DbString,
    property: &DbString,
) -> Option<&'a Value> {
    if !labels.contains(label) {
        return None;
    }
    props.get(property).filter(|value| !is_null(value))
}

fn insert_commit(
    indexes: &mut VectorIndexMap,
    label: DbString,
    property: DbString,
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
    label: DbString,
    property: DbString,
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

fn replace_commit(
    indexes: &mut VectorIndexMap,
    label: DbString,
    property: DbString,
    old_value: &Value,
    new_value: &Value,
    row: u32,
) -> GraphResult<()> {
    if let Some(entry) = indexes.get_mut(&(label.clone(), property.clone())) {
        admit(old_value, entry.kind(), entry.dimension()).map_err(|err| {
            index_rejection(label.clone(), property.clone(), entry.dimension(), err)
        })?;
        let vector = admit(new_value, entry.kind(), entry.dimension())
            .map_err(|err| index_rejection(label, property, entry.dimension(), err))?;
        std::sync::Arc::make_mut(&mut entry.index).insert_value(row, vector)?;
    }
    Ok(())
}

pub(super) fn admit(
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
    if let Some(metric) = kind.ann_metric() {
        metric
            .distance(vector, vector)
            .map_err(|err| VectorIndexValueError::MetricRejected {
                observed: err.to_string(),
            })?;
    }
    Ok(vector)
}

pub(super) fn index_rejection(
    label: DbString,
    property: DbString,
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

pub(super) fn warn_rejected(
    op: &'static str,
    label: DbString,
    property: DbString,
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

pub(super) const fn is_null(value: &Value) -> bool {
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
        Value::Json(_) => "Json",
        _ => "Unknown",
    }
}
