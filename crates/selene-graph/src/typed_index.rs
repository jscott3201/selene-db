//! Built-in per-`(label, property)` value index. See spec 03 section 5.2.
//!
//! # Value coercion
//!
//! `typed_key` is the single `Value`→`TypedKey` coercion shared by write, read,
//! and diff callers. Kind-mismatched reads return `None` so callers can scan.
//!
//! The same collapse is mirrored in [`crate::composite_typed_index`] for
//! composite indexes.

use std::collections::BTreeMap;

use roaring::RoaringBitmap;
use selene_core::{DbString, DurationOrderKey, Value};
use serde::{Deserialize, Serialize};

mod keying;
mod lookup;

pub(crate) use keying::{TypedIndexValueError, observed_value_kind};
use keying::{TypedKey, typed_key};

pub use crate::typed_float_key::{NotNanError, NotNanF32, NotNanF64};

/// Indexable value kind for built-in node property indexes.
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
pub enum TypedIndexKind {
    /// Boolean value. Backs [`Value::Bool`].
    Bool,
    /// Signed 64-bit integer. Backs [`Value::Int`].
    I64,
    /// Unsigned 64-bit integer. Backs [`Value::Uint`].
    U64,
    /// Signed 128-bit integer. Backs [`Value::Int128`].
    I128,
    /// Unsigned 128-bit integer. Backs [`Value::Uint128`].
    U128,
    /// Fixed-precision decimal. Backs [`Value::Decimal`].
    Decimal,
    /// Finite `f32`. Backs [`Value::Float32`]; NaN is rejected.
    F32,
    /// Finite `f64`. Backs [`Value::Float`]; NaN is rejected.
    F64,
    /// Database string. Backs [`Value::String`].
    String,
    /// Civil date. Backs [`Value::Date`].
    Date,
    /// Civil local date-time. Backs [`Value::LocalDateTime`].
    LocalDateTime,
    /// Zoned date-time. Backs [`Value::ZonedDateTime`].
    ZonedDateTime,
    /// Civil local time. Backs [`Value::LocalTime`].
    LocalTime,
    /// Zoned time. Backs [`Value::ZonedTime`].
    ZonedTime,
    /// Duration. Backs [`Value::Duration`].
    Duration,
    /// UUID. Backs [`Value::Uuid`].
    Uuid,
}

/// Built-in per-`(label, property)` node value index.
#[derive(Clone, Debug)]
pub enum TypedIndex {
    /// Boolean index.
    Bool(BTreeMap<bool, RoaringBitmap>),
    /// Signed integer index.
    I64(BTreeMap<i64, RoaringBitmap>),
    /// Unsigned integer index.
    U64(BTreeMap<u64, RoaringBitmap>),
    /// Signed 128-bit integer index.
    I128(BTreeMap<i128, RoaringBitmap>),
    /// Unsigned 128-bit integer index.
    U128(BTreeMap<u128, RoaringBitmap>),
    /// Fixed-precision decimal index.
    Decimal(BTreeMap<rust_decimal::Decimal, RoaringBitmap>),
    /// 32-bit floating-point index with NaN excluded.
    F32(BTreeMap<NotNanF32, RoaringBitmap>),
    /// Floating-point index with NaN excluded.
    F64(BTreeMap<NotNanF64, RoaringBitmap>),
    /// Database-string index.
    String(BTreeMap<DbString, RoaringBitmap>),
    /// Civil date index.
    Date(BTreeMap<jiff::civil::Date, RoaringBitmap>),
    /// Civil local date-time index.
    LocalDateTime(BTreeMap<jiff::civil::DateTime, RoaringBitmap>),
    /// Zoned date-time index.
    ZonedDateTime(BTreeMap<jiff::Zoned, RoaringBitmap>),
    /// Civil local time index.
    LocalTime(BTreeMap<jiff::civil::Time, RoaringBitmap>),
    /// Zoned time index.
    ZonedTime(BTreeMap<jiff::Zoned, RoaringBitmap>),
    /// Duration index.
    Duration(BTreeMap<DurationOrderKey, RoaringBitmap>),
    /// UUID index.
    Uuid(BTreeMap<uuid::Uuid, RoaringBitmap>),
}

impl TypedIndex {
    /// Construct an empty index of `kind`.
    #[must_use]
    pub fn new(kind: TypedIndexKind) -> Self {
        match kind {
            TypedIndexKind::Bool => Self::Bool(BTreeMap::new()),
            TypedIndexKind::I64 => Self::I64(BTreeMap::new()),
            TypedIndexKind::U64 => Self::U64(BTreeMap::new()),
            TypedIndexKind::I128 => Self::I128(BTreeMap::new()),
            TypedIndexKind::U128 => Self::U128(BTreeMap::new()),
            TypedIndexKind::Decimal => Self::Decimal(BTreeMap::new()),
            TypedIndexKind::F32 => Self::F32(BTreeMap::new()),
            TypedIndexKind::F64 => Self::F64(BTreeMap::new()),
            TypedIndexKind::String => Self::String(BTreeMap::new()),
            TypedIndexKind::Date => Self::Date(BTreeMap::new()),
            TypedIndexKind::LocalDateTime => Self::LocalDateTime(BTreeMap::new()),
            TypedIndexKind::ZonedDateTime => Self::ZonedDateTime(BTreeMap::new()),
            TypedIndexKind::LocalTime => Self::LocalTime(BTreeMap::new()),
            TypedIndexKind::ZonedTime => Self::ZonedTime(BTreeMap::new()),
            TypedIndexKind::Duration => Self::Duration(BTreeMap::new()),
            TypedIndexKind::Uuid => Self::Uuid(BTreeMap::new()),
        }
    }

    /// Return the value kind indexed by this index.
    #[must_use]
    pub const fn kind(&self) -> TypedIndexKind {
        match self {
            Self::Bool(_) => TypedIndexKind::Bool,
            Self::I64(_) => TypedIndexKind::I64,
            Self::U64(_) => TypedIndexKind::U64,
            Self::I128(_) => TypedIndexKind::I128,
            Self::U128(_) => TypedIndexKind::U128,
            Self::Decimal(_) => TypedIndexKind::Decimal,
            Self::F32(_) => TypedIndexKind::F32,
            Self::F64(_) => TypedIndexKind::F64,
            Self::String(_) => TypedIndexKind::String,
            Self::Date(_) => TypedIndexKind::Date,
            Self::LocalDateTime(_) => TypedIndexKind::LocalDateTime,
            Self::ZonedDateTime(_) => TypedIndexKind::ZonedDateTime,
            Self::LocalTime(_) => TypedIndexKind::LocalTime,
            Self::ZonedTime(_) => TypedIndexKind::ZonedTime,
            Self::Duration(_) => TypedIndexKind::Duration,
            Self::Uuid(_) => TypedIndexKind::Uuid,
        }
    }

    /// Return total row cardinality across all indexed keys.
    ///
    /// This is the sum of every bucket's row count, NOT the number of distinct
    /// keys. For the distinct-key count (e.g. to estimate an average bucket size
    /// `cardinality / distinct_keys` for parameter-equality cost estimation) use
    /// [`TypedIndex::distinct_keys`].
    #[must_use]
    pub fn cardinality(&self) -> u64 {
        match self {
            Self::Bool(index) => cardinality(index),
            Self::I64(index) => cardinality(index),
            Self::U64(index) => cardinality(index),
            Self::I128(index) => cardinality(index),
            Self::U128(index) => cardinality(index),
            Self::Decimal(index) => cardinality(index),
            Self::F32(index) => cardinality(index),
            Self::F64(index) => cardinality(index),
            Self::String(index) => cardinality(index),
            Self::Date(index) => cardinality(index),
            Self::LocalDateTime(index) => cardinality(index),
            Self::ZonedDateTime(index) => cardinality(index),
            Self::LocalTime(index) => cardinality(index),
            Self::ZonedTime(index) => cardinality(index),
            Self::Duration(index) => cardinality(index),
            Self::Uuid(index) => cardinality(index),
        }
    }

    /// Return the number of distinct indexed keys (BTreeMap entry count).
    ///
    /// Unlike [`TypedIndex::cardinality`] (total rows), this is the number of
    /// distinct values present in the index. The optimizer cost model divides
    /// `cardinality / distinct_keys` to estimate the expected rows returned by a
    /// parameter-equality probe whose value is unknown at plan time. Returns `0`
    /// for an empty index.
    #[must_use]
    pub fn distinct_keys(&self) -> u64 {
        match self {
            Self::Bool(index) => index.len() as u64,
            Self::I64(index) => index.len() as u64,
            Self::U64(index) => index.len() as u64,
            Self::I128(index) => index.len() as u64,
            Self::U128(index) => index.len() as u64,
            Self::Decimal(index) => index.len() as u64,
            Self::F32(index) => index.len() as u64,
            Self::F64(index) => index.len() as u64,
            Self::String(index) => index.len() as u64,
            Self::Date(index) => index.len() as u64,
            Self::LocalDateTime(index) => index.len() as u64,
            Self::ZonedDateTime(index) => index.len() as u64,
            Self::LocalTime(index) => index.len() as u64,
            Self::ZonedTime(index) => index.len() as u64,
            Self::Duration(index) => index.len() as u64,
            Self::Uuid(index) => index.len() as u64,
        }
    }

    /// Return true when this index holds exactly the same `(key -> rows)`
    /// buckets as `reference`.
    ///
    /// Used by the debug-only structural consistency net
    /// ([`crate::SeleneGraph::assert_indexes_consistent`]) to compare the
    /// commit-path-maintained index against a freshly re-derived reference
    /// built with the same lenient admission policy. Two indexes are equal
    /// only when their kinds match and every bucket maps to an identical
    /// row bitmap; a missing key, an extra key, or a differing bitmap all
    /// fail the comparison.
    #[must_use]
    pub(crate) fn buckets_eq(&self, reference: &Self) -> bool {
        match (self, reference) {
            (Self::Bool(lhs), Self::Bool(rhs)) => lhs == rhs,
            (Self::I64(lhs), Self::I64(rhs)) => lhs == rhs,
            (Self::U64(lhs), Self::U64(rhs)) => lhs == rhs,
            (Self::I128(lhs), Self::I128(rhs)) => lhs == rhs,
            (Self::U128(lhs), Self::U128(rhs)) => lhs == rhs,
            (Self::Decimal(lhs), Self::Decimal(rhs)) => lhs == rhs,
            (Self::F32(lhs), Self::F32(rhs)) => lhs == rhs,
            (Self::F64(lhs), Self::F64(rhs)) => lhs == rhs,
            (Self::String(lhs), Self::String(rhs)) => lhs == rhs,
            (Self::Date(lhs), Self::Date(rhs)) => lhs == rhs,
            (Self::LocalDateTime(lhs), Self::LocalDateTime(rhs)) => lhs == rhs,
            (Self::ZonedDateTime(lhs), Self::ZonedDateTime(rhs)) => lhs == rhs,
            (Self::LocalTime(lhs), Self::LocalTime(rhs)) => lhs == rhs,
            (Self::ZonedTime(lhs), Self::ZonedTime(rhs)) => lhs == rhs,
            (Self::Duration(lhs), Self::Duration(rhs)) => lhs == rhs,
            (Self::Uuid(lhs), Self::Uuid(rhs)) => lhs == rhs,
            _ => false,
        }
    }

    /// Return true when any indexed key maps to an empty row bitmap.
    ///
    /// Commit-path maintenance prunes a bucket when its bitmap empties
    /// (see `remove_row`), so a present-but-empty bucket is a maintenance
    /// leak the debug-only consistency net flags.
    #[must_use]
    pub(crate) fn has_empty_bucket(&self) -> bool {
        match self {
            Self::Bool(index) => index.values().any(RoaringBitmap::is_empty),
            Self::I64(index) => index.values().any(RoaringBitmap::is_empty),
            Self::U64(index) => index.values().any(RoaringBitmap::is_empty),
            Self::I128(index) => index.values().any(RoaringBitmap::is_empty),
            Self::U128(index) => index.values().any(RoaringBitmap::is_empty),
            Self::Decimal(index) => index.values().any(RoaringBitmap::is_empty),
            Self::F32(index) => index.values().any(RoaringBitmap::is_empty),
            Self::F64(index) => index.values().any(RoaringBitmap::is_empty),
            Self::String(index) => index.values().any(RoaringBitmap::is_empty),
            Self::Date(index) => index.values().any(RoaringBitmap::is_empty),
            Self::LocalDateTime(index) => index.values().any(RoaringBitmap::is_empty),
            Self::ZonedDateTime(index) => index.values().any(RoaringBitmap::is_empty),
            Self::LocalTime(index) => index.values().any(RoaringBitmap::is_empty),
            Self::ZonedTime(index) => index.values().any(RoaringBitmap::is_empty),
            Self::Duration(index) => index.values().any(RoaringBitmap::is_empty),
            Self::Uuid(index) => index.values().any(RoaringBitmap::is_empty),
        }
    }

    /// Insert `row` into the bitmap for `value`.
    pub(crate) fn insert(&mut self, value: &Value, row: u32) -> Result<(), TypedIndexValueError> {
        let expected_kind = self.kind();
        match (self, typed_key(value, expected_kind)?) {
            (Self::Bool(index), TypedKey::Bool(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::I64(index), TypedKey::I64(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::U64(index), TypedKey::U64(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::I128(index), TypedKey::I128(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::U128(index), TypedKey::U128(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::Decimal(index), TypedKey::Decimal(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::F32(index), TypedKey::F32(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::F64(index), TypedKey::F64(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::String(index), TypedKey::String(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::Date(index), TypedKey::Date(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::LocalDateTime(index), TypedKey::LocalDateTime(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::ZonedDateTime(index), TypedKey::ZonedDateTime(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::LocalTime(index), TypedKey::LocalTime(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::ZonedTime(index), TypedKey::ZonedTime(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::Duration(index), TypedKey::Duration(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (Self::Uuid(index), TypedKey::Uuid(key)) => {
                index.entry(key).or_default().insert(row);
                Ok(())
            }
            (index, key) => Err(TypedIndexValueError::KindMismatch {
                expected_kind: index.kind(),
                observed: key.observed(),
            }),
        }
    }

    /// Remove `row` from the bitmap for `value`.
    ///
    /// Missing rows are ignored. If the bitmap for a key becomes empty, the key
    /// is pruned from the inner map.
    pub(crate) fn remove(&mut self, value: &Value, row: u32) -> Result<(), TypedIndexValueError> {
        let expected_kind = self.kind();
        match (self, typed_key(value, expected_kind)?) {
            (Self::Bool(index), TypedKey::Bool(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::I64(index), TypedKey::I64(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::U64(index), TypedKey::U64(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::I128(index), TypedKey::I128(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::U128(index), TypedKey::U128(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::Decimal(index), TypedKey::Decimal(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::F32(index), TypedKey::F32(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::F64(index), TypedKey::F64(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::String(index), TypedKey::String(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::Date(index), TypedKey::Date(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::LocalDateTime(index), TypedKey::LocalDateTime(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::ZonedDateTime(index), TypedKey::ZonedDateTime(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::LocalTime(index), TypedKey::LocalTime(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::ZonedTime(index), TypedKey::ZonedTime(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::Duration(index), TypedKey::Duration(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (Self::Uuid(index), TypedKey::Uuid(key)) => {
                remove_row(index, &key, row);
                Ok(())
            }
            (index, key) => Err(TypedIndexValueError::KindMismatch {
                expected_kind: index.kind(),
                observed: key.observed(),
            }),
        }
    }
}

fn cardinality<K>(index: &BTreeMap<K, RoaringBitmap>) -> u64 {
    index.values().map(RoaringBitmap::len).sum()
}

fn remove_row<K: Ord>(index: &mut BTreeMap<K, RoaringBitmap>, key: &K, row: u32) {
    if let Some(bitmap) = index.get_mut(key) {
        bitmap.remove(row);
        if bitmap.is_empty() {
            index.remove(key);
        }
    }
}

#[cfg(test)]
#[path = "typed_index_tests.rs"]
mod tests;
