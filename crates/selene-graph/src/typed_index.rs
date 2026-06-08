//! Built-in per-`(label, property)` value index. See spec 03 section 5.2.
//!
//! # Value coercion
//!
//! `typed_key` is the single `Value`→`TypedKey` coercion shared by every
//! write-side (`insert`, `remove`) and read/diff-side (`lookup_eq`, the
//! per-type closures in `lookup_range`, and `values_share_key`) caller. A
//! `STRING` value always resolves directly to its database-string key. Kind
//! mismatch (e.g. a `Value::Json` against any index) and NaN still raise
//! `TypedIndexValueError`; a kind-mismatched read returns `None` to the caller
//! so it drops to a runtime scan.
//!
//! The same collapse is mirrored in [`crate::composite_typed_index`] for
//! composite indexes.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::{DbString, Value};
use serde::{Deserialize, Serialize};

/// Indexable value kind for v1.0 built-in node property indexes.
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
    /// Finite `f64`. Backs [`Value::Float`]; NaN is rejected.
    F64,
    /// Database string. Backs [`Value::String`].
    String,
    /// Civil date. Backs [`Value::Date`].
    Date,
    /// Civil local date-time. Backs [`Value::LocalDateTime`].
    LocalDateTime,
    /// UUID. Backs [`Value::Uuid`].
    Uuid,
}

/// Marker error returned when a raw `f64` cannot be admitted to [`NotNanF64`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("NaN is not an indexable f64 value")]
pub struct NotNanError;

/// `f64` wrapper with total ordering via [`f64::total_cmp`].
///
/// The constructor rejects NaN because NaN has no useful equality or range
/// semantics for a graph property index. `+0.0` and `-0.0` remain distinct
/// keys because equality and hashing use the underlying bit pattern.
#[derive(Clone, Copy, Debug)]
pub struct NotNanF64(f64);

impl NotNanF64 {
    /// Construct a finite-or-infinite ordered f64 key.
    ///
    /// # Errors
    ///
    /// Returns [`NotNanError`] when `value` is NaN.
    pub fn new(value: f64) -> Result<Self, NotNanError> {
        if value.is_nan() {
            Err(NotNanError)
        } else {
            Ok(Self(value))
        }
    }

    /// Return the underlying `f64`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for NotNanF64 {
    fn eq(&self, rhs: &Self) -> bool {
        self.0.to_bits() == rhs.0.to_bits()
    }
}

impl Eq for NotNanF64 {}

impl PartialOrd for NotNanF64 {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

impl Ord for NotNanF64 {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.total_cmp(&rhs.0)
    }
}

impl Hash for NotNanF64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
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
    /// Floating-point index with NaN excluded.
    F64(BTreeMap<NotNanF64, RoaringBitmap>),
    /// Database-string index.
    String(BTreeMap<DbString, RoaringBitmap>),
    /// Civil date index.
    Date(BTreeMap<jiff::civil::Date, RoaringBitmap>),
    /// Civil local date-time index.
    LocalDateTime(BTreeMap<jiff::civil::DateTime, RoaringBitmap>),
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
            TypedIndexKind::F64 => Self::F64(BTreeMap::new()),
            TypedIndexKind::String => Self::String(BTreeMap::new()),
            TypedIndexKind::Date => Self::Date(BTreeMap::new()),
            TypedIndexKind::LocalDateTime => Self::LocalDateTime(BTreeMap::new()),
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
            Self::F64(_) => TypedIndexKind::F64,
            Self::String(_) => TypedIndexKind::String,
            Self::Date(_) => TypedIndexKind::Date,
            Self::LocalDateTime(_) => TypedIndexKind::LocalDateTime,
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
            Self::F64(index) => cardinality(index),
            Self::String(index) => cardinality(index),
            Self::Date(index) => cardinality(index),
            Self::LocalDateTime(index) => cardinality(index),
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
            Self::F64(index) => index.len() as u64,
            Self::String(index) => index.len() as u64,
            Self::Date(index) => index.len() as u64,
            Self::LocalDateTime(index) => index.len() as u64,
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
            (Self::F64(lhs), Self::F64(rhs)) => lhs == rhs,
            (Self::String(lhs), Self::String(rhs)) => lhs == rhs,
            (Self::Date(lhs), Self::Date(rhs)) => lhs == rhs,
            (Self::LocalDateTime(lhs), Self::LocalDateTime(rhs)) => lhs == rhs,
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
            Self::F64(index) => index.values().any(RoaringBitmap::is_empty),
            Self::String(index) => index.values().any(RoaringBitmap::is_empty),
            Self::Date(index) => index.values().any(RoaringBitmap::is_empty),
            Self::LocalDateTime(index) => index.values().any(RoaringBitmap::is_empty),
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
            (Self::F64(index), TypedKey::F64(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::String(index), TypedKey::String(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::Date(index), TypedKey::Date(key)) => Some(cow_or_empty(index.get(&key))),
            (Self::LocalDateTime(index), TypedKey::LocalDateTime(key)) => {
                Some(cow_or_empty(index.get(&key)))
            }
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
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Bool) {
                        Ok(TypedKey::Bool(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Bool) {
                        Ok(TypedKey::Bool(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::I64(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::I64) {
                        Ok(TypedKey::I64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::I64) {
                        Ok(TypedKey::I64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::U64(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::U64) {
                        Ok(TypedKey::U64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::U64) {
                        Ok(TypedKey::U64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::F64(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::F64) {
                        Ok(TypedKey::F64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::F64) {
                        Ok(TypedKey::F64(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            // String ranges walk the now-lexicographic `BTreeMap<DbString, _>`
            // range directly — result-identical to the old `None` linear-scan
            // fallback (the linear scan compared `Value::String` rows
            // lexicographically, and `DbString` Ord is lexicographic), just
            // O(log n + matched) instead of O(total cardinality).
            Self::String(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::String) {
                        Ok(TypedKey::String(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::String) {
                        Ok(TypedKey::String(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::Date(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Date) {
                        Ok(TypedKey::Date(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Date) {
                        Ok(TypedKey::Date(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::LocalDateTime(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::LocalDateTime) {
                        Ok(TypedKey::LocalDateTime(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::LocalDateTime) {
                        Ok(TypedKey::LocalDateTime(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::Uuid(index) => {
                let start = bound_to_key(range.start_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Uuid) {
                        Ok(TypedKey::Uuid(key)) => Some(key),
                        _ => None,
                    }
                })?;
                let end = bound_to_key(range.end_bound(), |value| {
                    match typed_key(value, TypedIndexKind::Uuid) {
                        Ok(TypedKey::Uuid(key)) => Some(key),
                        _ => None,
                    }
                })?;
                Some(range_union(index, &start, &end))
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
            (Self::F64(_), Ok(TypedKey::F64(lhs)), Ok(TypedKey::F64(rhs))) => lhs == rhs,
            (Self::String(_), Ok(TypedKey::String(lhs)), Ok(TypedKey::String(rhs))) => lhs == rhs,
            (Self::Date(_), Ok(TypedKey::Date(lhs)), Ok(TypedKey::Date(rhs))) => lhs == rhs,
            (
                Self::LocalDateTime(_),
                Ok(TypedKey::LocalDateTime(lhs)),
                Ok(TypedKey::LocalDateTime(rhs)),
            ) => lhs == rhs,
            (Self::Uuid(_), Ok(TypedKey::Uuid(lhs)), Ok(TypedKey::Uuid(rhs))) => lhs == rhs,
            _ => raw_value_same(lhs, rhs),
        }
    }
}

/// Internal value-admission error for index mutation.
#[derive(Debug)]
pub(crate) enum TypedIndexValueError {
    /// Value kind did not match the index kind.
    KindMismatch {
        /// The index kind being updated.
        expected_kind: TypedIndexKind,
        /// The observed value kind.
        observed: &'static str,
    },
    /// A `Value::Float` was NaN.
    NaN {
        /// The index kind being updated.
        expected_kind: TypedIndexKind,
    },
}

impl TypedIndexValueError {
    /// Return the expected index kind.
    pub(crate) fn expected_kind(&self) -> TypedIndexKind {
        match self {
            Self::KindMismatch { expected_kind, .. } | Self::NaN { expected_kind } => {
                *expected_kind
            }
        }
    }

    /// Return the observed value description.
    pub(crate) fn observed(&self) -> &'static str {
        match self {
            Self::KindMismatch { observed, .. } => observed,
            Self::NaN { .. } => "NaN",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TypedKey {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(NotNanF64),
    String(DbString),
    Date(jiff::civil::Date),
    LocalDateTime(jiff::civil::DateTime),
    Uuid(uuid::Uuid),
}

impl TypedKey {
    const fn observed(&self) -> &'static str {
        match self {
            Self::Bool(_) => "Bool",
            Self::I64(_) => "Int",
            Self::U64(_) => "Uint",
            Self::F64(_) => "Float",
            Self::String(_) => "String",
            Self::Date(_) => "Date",
            Self::LocalDateTime(_) => "LocalDateTime",
            Self::Uuid(_) => "Uuid",
        }
    }
}

/// Coerce `value` into a [`TypedKey`].
///
/// This is the single coercion shared by write-side (`insert`/`remove`) and
/// read/diff-side (`lookup_eq`/`lookup_range`/`values_share_key`) callers.
/// A `STRING` value always resolves directly to its database-string key. `Err`
/// carries the kind-mismatch
/// (`expected_kind` set by the caller's index kind) / NaN semantics; the outer
/// `(self, key)` match in `insert`/`remove` enforces the final kind check so a
/// `Value::String` inserted into an `I64` index still rejects with
/// `KindMismatch`.
fn typed_key(
    value: &Value,
    expected_kind: TypedIndexKind,
) -> Result<TypedKey, TypedIndexValueError> {
    match value {
        Value::Bool(value) => Ok(TypedKey::Bool(*value)),
        Value::Int(value) => Ok(TypedKey::I64(*value)),
        Value::Uint(value) => Ok(TypedKey::U64(*value)),
        Value::Float(value) => NotNanF64::new(*value)
            .map(TypedKey::F64)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        Value::String(value) => Ok(TypedKey::String(value.clone())),
        Value::Date(value) => Ok(TypedKey::Date(*value)),
        Value::LocalDateTime(value) => Ok(TypedKey::LocalDateTime(*value)),
        Value::Uuid(value) => Ok(TypedKey::Uuid(*value)),
        _ => Err(TypedIndexValueError::KindMismatch {
            expected_kind,
            observed: observed_value_kind(value),
        }),
    }
}

pub(crate) fn observed_value_kind(value: &Value) -> &'static str {
    value.variant_name()
}

fn raw_value_same(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Float(lhs), Value::Float(rhs)) => lhs.to_bits() == rhs.to_bits(),
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs.to_bits() == rhs.to_bits(),
        _ => lhs == rhs,
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum KeyBound<K> {
    Unbounded,
    Included(K),
    Excluded(K),
}

fn bound_to_key<K>(
    bound: Bound<&Value>,
    convert: impl FnOnce(&Value) -> Option<K>,
) -> Option<KeyBound<K>> {
    match bound {
        Bound::Included(value) => convert(value).map(KeyBound::Included),
        Bound::Excluded(value) => convert(value).map(KeyBound::Excluded),
        Bound::Unbounded => Some(KeyBound::Unbounded),
    }
}

fn range_union<K: Ord>(
    index: &BTreeMap<K, RoaringBitmap>,
    start: &KeyBound<K>,
    end: &KeyBound<K>,
) -> RoaringBitmap {
    // Use BTreeMap's ordered range iteration so a narrow window touches
    // O(log n + matched) keys rather than scanning the entire map.
    let start_bound = match start {
        KeyBound::Unbounded => Bound::Unbounded,
        KeyBound::Included(key) => Bound::Included(key),
        KeyBound::Excluded(key) => Bound::Excluded(key),
    };
    let end_bound = match end {
        KeyBound::Unbounded => Bound::Unbounded,
        KeyBound::Included(key) => Bound::Included(key),
        KeyBound::Excluded(key) => Bound::Excluded(key),
    };
    let mut result = RoaringBitmap::new();
    for (_key, bitmap) in index.range::<K, _>((start_bound, end_bound)) {
        // RoaringBitmap bulk OR (`BitOrAssign<&RoaringBitmap>`) is the union
        // primitive — far cheaper than a per-element scan-and-insert.
        result |= bitmap;
    }
    result
}

#[cfg(test)]
#[path = "typed_index_tests.rs"]
mod tests;
