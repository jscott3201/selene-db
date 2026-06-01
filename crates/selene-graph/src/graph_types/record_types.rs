//! Typed `RECORD` field-type descriptors for the closed-graph catalog.
//!
//! Split out of `graph_types.rs` (700-LOC cap). Holds the rkyv-persistable
//! RECORD field-type family — [`RecordFieldTypes`] / [`RecordFieldTypeDef`] /
//! [`RecordFieldType`] (the `CORE/GTYP` snapshot side; the serde/WAL counterpart
//! is `selene_core::schema::RecordFieldStructure`) — plus the catalog-time
//! shape validators these types share.
//!
//! Why: Per ISO 39075:2024 §18.9 `<record type>` / `<field types specification>`
//! (GV46 closed record types) and §18.10 (GV48 nested record types).

use std::collections::BTreeSet;

use selene_core::{IStr, PropertyValueType, Record, Value};
use serde::{Deserialize, Serialize};

use super::MAX_RECORD_TYPE_NESTING;
use crate::error::{GraphError, GraphResult};

/// Ordered field-type descriptor list for a closed/typed `RECORD` property declaration
/// (rkyv snapshot side; persisted in `CORE/GTYP`). The serde/WAL counterpart is
/// `selene_core::schema::RecordFieldStructure`; the two carry the same structure and must
/// round-trip into each other.
// Why: Per ISO 39075:2024 §18.9 <record type> / <field types specification>; GV46 (closed
// record types, §18.9 CR42).
//
// The bytecheck/serialize/deserialize bounds block is required on every type in the
// mutual-recursion cycle (RecordFieldTypes -> RecordFieldTypeDef -> RecordFieldType ->
// RecordFieldTypes) so the `omit_bounds` boundary's manual bounds propagate uniformly up
// to the `Arc<GraphTypeDef>` archive check in `GraphMeta`.
#[derive(
    Clone,
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
#[rkyv(
    bytecheck(bounds(__C: rkyv::validation::ArchiveContext)),
    deserialize_bounds(__D::Error: rkyv::rancor::Source),
    serialize_bounds(__S: rkyv::ser::Writer + rkyv::ser::Allocator, __S::Error: rkyv::rancor::Source)
)]
pub struct RecordFieldTypes(pub Vec<RecordFieldTypeDef>);

/// One declared field within a typed `RECORD`: name, its (possibly nested) type, and
/// whether the field is required (non-nullable).
#[derive(
    Clone,
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
#[rkyv(
    bytecheck(bounds(__C: rkyv::validation::ArchiveContext)),
    deserialize_bounds(__D::Error: rkyv::rancor::Source),
    serialize_bounds(__S: rkyv::ser::Writer + rkyv::ser::Allocator, __S::Error: rkyv::rancor::Source)
)]
pub struct RecordFieldTypeDef {
    /// Field name.
    pub name: IStr,
    /// Declared field type (recursively nestable).
    pub field_type: RecordFieldType,
    /// `true` when the field is required (NOT NULL).
    pub required: bool,
}

/// Persistable, recursively-nestable field-type descriptor for typed `RECORD` declarations.
///
/// Kept `#[non_exhaustive]` like [`super::PropertyElementType`]: its conversions and
/// `matches` live in this crate, so same-crate exhaustiveness checks still apply while
/// downstream crates remain forward-compatible.
// Why: Per ISO 39075:2024 §18.10 CR1 (GV48 nested record types) — a field type may itself
// contain a list or record type.
#[derive(
    Clone,
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
#[rkyv(
    bytecheck(bounds(__C: rkyv::validation::ArchiveContext)),
    deserialize_bounds(__D::Error: rkyv::rancor::Source),
    serialize_bounds(__S: rkyv::ser::Writer + rkyv::ser::Allocator, __S::Error: rkyv::rancor::Source)
)]
#[non_exhaustive]
pub enum RecordFieldType {
    /// Scalar field type.
    Scalar(PropertyValueType),
    /// `LIST` field type.
    List(#[rkyv(omit_bounds)] Box<RecordFieldType>),
    /// Nested `RECORD` field type.
    Record(#[rkyv(omit_bounds)] Box<RecordFieldTypes>),
}

impl RecordFieldType {
    /// Return true when `value` structurally conforms to this declared field type.
    #[must_use]
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Scalar(value_type) => value_type.matches(value),
            Self::List(inner) => match value {
                Value::List(values) => values.iter().all(|value| inner.matches(value)),
                _ => false,
            },
            Self::Record(inner) => inner.matches(value),
        }
    }
}

impl RecordFieldTypes {
    /// Return true when `value` structurally conforms to this closed-record descriptor.
    ///
    /// A `Value::Record(Record::Open)` is checked by field name with **set equality** —
    /// every declared field must be present (or optional) and match, and no undeclared
    /// extra field may appear. A `Value::RecordTyped` is checked positionally with the
    /// same cardinality, because it carries no inline names. Per ISO 39075:2024 §4.15.4 a
    /// closed record value must have the same field-name set as the descriptor.
    ///
    /// An explicit `Value::Null` for a field conforms iff that field is optional —
    /// consistent with how an absent (or positional `None`) optional field is treated, so
    /// the present-null and absent cases agree.
    #[must_use]
    pub fn matches(&self, value: &Value) -> bool {
        match value {
            Value::RecordTyped(record) => {
                record.values.len() == self.0.len()
                    && self
                        .0
                        .iter()
                        .zip(record.values.iter())
                        .all(|(field, slot)| match slot {
                            Some(Value::Null) | None => !field.required,
                            Some(value) => field.field_type.matches(value),
                        })
            }
            Value::Record(record) => match record.as_ref() {
                Record::Open(fields) => {
                    // Every declared field present-or-optional and type-matched ...
                    self.0.iter().all(|field| {
                        match fields.iter().find(|(name, _)| *name == field.name) {
                            Some((_, Value::Null)) | None => !field.required,
                            Some((_, value)) => field.field_type.matches(value),
                        }
                    })
                    // ... and no undeclared extra field (ISO §4.15.4 set equality).
                        && fields
                            .iter()
                            .all(|(name, _)| self.0.iter().any(|field| field.name == *name))
                }
                _ => false,
            },
            _ => false,
        }
    }
}

/// Validate the shape of a typed-`RECORD` field-type list at catalog time:
/// nesting budget, unique field names, and recursively-valid field types.
pub(super) fn validate_record_field_types(
    type_name: IStr,
    property_name: IStr,
    fields: &RecordFieldTypes,
    depth: u32,
) -> GraphResult<()> {
    if depth > MAX_RECORD_TYPE_NESTING {
        return Err(GraphError::Inconsistent {
            reason: format!(
                "property {property_name} on type {type_name} exceeds RECORD nesting limit"
            ),
        });
    }
    // ISO 39075:2024 §18.10 SR2: a field name shall not equal another field name.
    let mut seen = BTreeSet::new();
    for field in &fields.0 {
        if !seen.insert(field.name.clone()) {
            return Err(GraphError::Inconsistent {
                reason: format!(
                    "property {property_name} on type {type_name} declares duplicate record field name {}",
                    field.name
                ),
            });
        }
        validate_record_field_type(
            type_name.clone(),
            property_name.clone(),
            &field.field_type,
            depth,
        )?;
    }
    Ok(())
}

fn validate_record_field_type(
    type_name: IStr,
    property_name: IStr,
    field_type: &RecordFieldType,
    depth: u32,
) -> GraphResult<()> {
    match field_type {
        RecordFieldType::Scalar(
            value_type @ (PropertyValueType::List
            | PropertyValueType::Record
            | PropertyValueType::RecordTyped),
        ) => Err(GraphError::Inconsistent {
            reason: format!(
                "property {property_name} on type {type_name} uses an unsupported scalar RECORD field type {value_type}"
            ),
        }),
        RecordFieldType::Scalar(_) => Ok(()),
        RecordFieldType::List(inner) => {
            validate_record_field_type(type_name, property_name, inner, depth + 1)
        }
        RecordFieldType::Record(inner) => {
            validate_record_field_types(type_name, property_name, inner, depth + 1)
        }
    }
}
