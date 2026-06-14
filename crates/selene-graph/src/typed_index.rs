//! Built-in per-`(label, property)` value index. See spec 03 section 5.2.
//!
//! # Value coercion
//!
//! `typed_key` is the single `Value`→`TypedKey` coercion shared by write, read,
//! and diff callers. Kind-mismatched reads return `None` so callers can scan.
//!
//! The same collapse is mirrored in [`crate::composite_typed_index`] for
//! composite indexes.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::{DbString, DurationOrderKey, Value};
use serde::{Deserialize, Serialize};

mod keying;

pub(crate) use keying::{TypedIndexValueError, observed_value_kind};
use keying::{TypedKey, raw_value_same, typed_key, typed_range_union};

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

    /// Return the rows matching `value` exactly.
    ///
    /// Returns `None` for kind-mismatched values (callers fall back to a
    /// runtime scan). With a single string space every `STRING` value resolves
    /// to its key directly through [`typed_key`].
    #[must_use]
    pub(crate) fn lookup_eq(&self, value: &Value) -> Option<Cow<'_, RoaringBitmap>> {
        let key = match typed_key(value, self.kind()) {
            Ok(key) => key,
            Err(_) => return None,
        };
        match (self, key) {
            (Self::Bool(index), TypedKey::Bool(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::I64(index), TypedKey::I64(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::U64(index), TypedKey::U64(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::I128(index), TypedKey::I128(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::U128(index), TypedKey::U128(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::Decimal(index), TypedKey::Decimal(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::F32(index), TypedKey::F32(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::F64(index), TypedKey::F64(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::String(index), TypedKey::String(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::Date(index), TypedKey::Date(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::LocalDateTime(index), TypedKey::LocalDateTime(key)) => {
                Some(cow_or_empty(index.get(&key)))
            }
            (Self::ZonedDateTime(index), TypedKey::ZonedDateTime(key)) => {
                Some(cow_or_empty(index.get(&key)))
            }
            (Self::LocalTime(index), TypedKey::LocalTime(key)) => {
                Some(cow_or_empty(index.get(&key)))
            }
            (Self::ZonedTime(index), TypedKey::ZonedTime(key)) => {
                Some(cow_or_empty(index.get(&key)))
            }
            (Self::Duration(index), TypedKey::Duration(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::Uuid(index), TypedKey::Uuid(key)) => Some(cow_or_empty(index.get(&key))),
            _ => None,
        }
    }

    /// Return the union of rows matching `range`.
    #[must_use]
    pub(crate) fn lookup_range<R>(&self, range: R) -> Option<RoaringBitmap>
    where
        R: RangeBounds<Value>,
    {
        match self {
            Self::Bool(index) => {
                typed_range_union(index, &range, TypedIndexKind::Bool, |key| match key {
                    TypedKey::Bool(key) => Some(key),
                    _ => None,
                })
            }
            Self::I64(index) => {
                typed_range_union(index, &range, TypedIndexKind::I64, |key| match key {
                    TypedKey::I64(key) => Some(key),
                    _ => None,
                })
            }
            Self::U64(index) => {
                typed_range_union(index, &range, TypedIndexKind::U64, |key| match key {
                    TypedKey::U64(key) => Some(key),
                    _ => None,
                })
            }
            Self::I128(index) => {
                typed_range_union(index, &range, TypedIndexKind::I128, |key| match key {
                    TypedKey::I128(key) => Some(key),
                    _ => None,
                })
            }
            Self::U128(index) => {
                typed_range_union(index, &range, TypedIndexKind::U128, |key| match key {
                    TypedKey::U128(key) => Some(key),
                    _ => None,
                })
            }
            Self::Decimal(index) => {
                typed_range_union(index, &range, TypedIndexKind::Decimal, |key| match key {
                    TypedKey::Decimal(key) => Some(key),
                    _ => None,
                })
            }
            Self::F32(index) => {
                typed_range_union(index, &range, TypedIndexKind::F32, |key| match key {
                    TypedKey::F32(key) => Some(key),
                    _ => None,
                })
            }
            Self::F64(index) => {
                typed_range_union(index, &range, TypedIndexKind::F64, |key| match key {
                    TypedKey::F64(key) => Some(key),
                    _ => None,
                })
            }
            // String ranges walk the now-lexicographic `BTreeMap<DbString, _>`
            // range directly — result-identical to the old `None` linear-scan
            // fallback (the linear scan compared `Value::String` rows
            // lexicographically, and `DbString` Ord is lexicographic), just
            // O(log n + matched) instead of O(total cardinality).
            Self::String(index) => {
                typed_range_union(index, &range, TypedIndexKind::String, |key| match key {
                    TypedKey::String(key) => Some(key),
                    _ => None,
                })
            }
            Self::Date(index) => {
                typed_range_union(index, &range, TypedIndexKind::Date, |key| match key {
                    TypedKey::Date(key) => Some(key),
                    _ => None,
                })
            }
            Self::LocalDateTime(index) => typed_range_union(
                index,
                &range,
                TypedIndexKind::LocalDateTime,
                |key| match key {
                    TypedKey::LocalDateTime(key) => Some(key),
                    _ => None,
                },
            ),
            Self::ZonedDateTime(index) => typed_range_union(
                index,
                &range,
                TypedIndexKind::ZonedDateTime,
                |key| match key {
                    TypedKey::ZonedDateTime(key) => Some(key),
                    _ => None,
                },
            ),
            Self::LocalTime(index) => {
                typed_range_union(index, &range, TypedIndexKind::LocalTime, |key| match key {
                    TypedKey::LocalTime(key) => Some(key),
                    _ => None,
                })
            }
            Self::ZonedTime(index) => {
                typed_range_union(index, &range, TypedIndexKind::ZonedTime, |key| match key {
                    TypedKey::ZonedTime(key) => Some(key),
                    _ => None,
                })
            }
            Self::Duration(index) => {
                typed_range_union(index, &range, TypedIndexKind::Duration, |key| match key {
                    TypedKey::Duration(key) => Some(key),
                    _ => None,
                })
            }
            Self::Uuid(index) => {
                typed_range_union(index, &range, TypedIndexKind::Uuid, |key| match key {
                    TypedKey::Uuid(key) => Some(key),
                    _ => None,
                })
            }
        }
    }

    /// Return the union of string-key rows whose key starts with `prefix`.
    ///
    /// `DbString` orders **lexicographically**, so every key starting with
    /// `prefix` forms a contiguous run beginning at
    /// the first key `>= prefix`. This seeks that run with `BTreeMap::range`
    /// (`Included(prefix)`, [`Bound::Unbounded`]) and stops at the first key
    /// that no longer starts with `prefix` — O(log n + matched) rather than the
    /// O(total cardinality) full scan, and result-identical to a per-key
    /// `starts_with` filter because it applies the exact same predicate over a
    /// sorted-order seek.
    ///
    /// Seeking from `Included(prefix)` (rather than computing an exclusive upper
    /// bound) sidesteps the encoding hazards an explicit successor key carries:
    /// an empty prefix or an all-`0xFF` prefix has no finite successor, and a
    /// byte-incremented successor can fall out of valid UTF-8. The break-on-
    /// first-mismatch walk handles all of those uniformly — an empty prefix
    /// matches every key (every key seeks to the start and `starts_with("")` is
    /// always true), and no matching tail is ever dropped.
    #[must_use]
    pub(crate) fn lookup_prefix(&self, prefix: &str) -> Option<RoaringBitmap> {
        match self {
            Self::String(index) => {
                // `BTreeMap<DbString, _>` keys are owned `DbString`, so seek with an
                // owned `DbString` lower bound. A prefix within the IL013 cap always
                // constructs; an over-cap prefix matches nothing (no stored key can
                // exceed the cap) — return empty rather than panic.
                let Ok(lo_key) = selene_core::db_string(prefix) else {
                    return Some(RoaringBitmap::new());
                };
                let mut result = RoaringBitmap::new();
                for (key, bitmap) in index.range((Bound::Included(lo_key), Bound::Unbounded)) {
                    if !key.as_str().starts_with(prefix) {
                        // Keys are sorted; the first non-match ends the run.
                        break;
                    }
                    result |= bitmap;
                }
                Some(result)
            }
            _ => None,
        }
    }

    /// Return true when two values address the same key in this index.
    ///
    /// This lets update maintenance avoid touching an index when a mutation
    /// changed unrelated node columns. Uses [`typed_key`]; when either side
    /// cannot be coerced to this index's kind the diff falls through to
    /// [`raw_value_same`] so we compare raw content. If raw values differ the
    /// update path fires remove+insert, which re-coerce through [`typed_key`].
    pub(crate) fn values_share_key(&self, lhs: &Value, rhs: &Value) -> bool {
        let kind = self.kind();
        match (self, typed_key(lhs, kind), typed_key(rhs, kind)) {
            (Self::Bool(_), Ok(TypedKey::Bool(lhs)), Ok(TypedKey::Bool(rhs))) => lhs == rhs,
            (Self::I64(_), Ok(TypedKey::I64(lhs)), Ok(TypedKey::I64(rhs))) => lhs == rhs,
            (Self::U64(_), Ok(TypedKey::U64(lhs)), Ok(TypedKey::U64(rhs))) => lhs == rhs,
            (Self::I128(_), Ok(TypedKey::I128(lhs)), Ok(TypedKey::I128(rhs))) => lhs == rhs,
            (Self::U128(_), Ok(TypedKey::U128(lhs)), Ok(TypedKey::U128(rhs))) => lhs == rhs,
            (Self::Decimal(_), Ok(TypedKey::Decimal(lhs)), Ok(TypedKey::Decimal(rhs))) => {
                lhs == rhs
            }
            (Self::F32(_), Ok(TypedKey::F32(lhs)), Ok(TypedKey::F32(rhs))) => lhs == rhs,
            (Self::F64(_), Ok(TypedKey::F64(lhs)), Ok(TypedKey::F64(rhs))) => lhs == rhs,
            (Self::String(_), Ok(TypedKey::String(lhs)), Ok(TypedKey::String(rhs))) => lhs == rhs,
            (Self::Date(_), Ok(TypedKey::Date(lhs)), Ok(TypedKey::Date(rhs))) => lhs == rhs,
            (
                Self::LocalDateTime(_),
                Ok(TypedKey::LocalDateTime(lhs)),
                Ok(TypedKey::LocalDateTime(rhs)),
            ) => lhs == rhs,
            (
                Self::ZonedDateTime(_),
                Ok(TypedKey::ZonedDateTime(lhs)),
                Ok(TypedKey::ZonedDateTime(rhs)),
            ) => lhs == rhs,
            (Self::LocalTime(_), Ok(TypedKey::LocalTime(lhs)), Ok(TypedKey::LocalTime(rhs))) => {
                lhs == rhs
            }
            (Self::ZonedTime(_), Ok(TypedKey::ZonedTime(lhs)), Ok(TypedKey::ZonedTime(rhs))) => {
                lhs == rhs
            }
            (Self::Duration(_), Ok(TypedKey::Duration(lhs)), Ok(TypedKey::Duration(rhs))) => {
                lhs == rhs
            }
            (Self::Uuid(_), Ok(TypedKey::Uuid(lhs)), Ok(TypedKey::Uuid(rhs))) => lhs == rhs,
            _ => raw_value_same(lhs, rhs),
        }
    }
}

fn cardinality<K>(index: &BTreeMap<K, RoaringBitmap>) -> u64 {
    index.values().map(RoaringBitmap::len).sum()
}

fn cow_or_empty(bitmap: Option<&RoaringBitmap>) -> Cow<'_, RoaringBitmap> {
    bitmap
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(RoaringBitmap::new()))
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
