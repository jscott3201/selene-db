//! Built-in per-`(label, property)` value index. See spec 03 section 5.2.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::{IStr, Value};
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
    /// Signed 64-bit integer. Backs [`Value::Int`].
    I64,
    /// Finite `f64`. Backs [`Value::Float`]; NaN is rejected.
    F64,
    /// Interned string. Backs [`Value::String`].
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
    /// Signed integer index.
    I64(BTreeMap<i64, RoaringBitmap>),
    /// Floating-point index with NaN excluded.
    F64(BTreeMap<NotNanF64, RoaringBitmap>),
    /// Interned string index.
    String(BTreeMap<IStr, RoaringBitmap>),
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
            TypedIndexKind::I64 => Self::I64(BTreeMap::new()),
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
            Self::I64(_) => TypedIndexKind::I64,
            Self::F64(_) => TypedIndexKind::F64,
            Self::String(_) => TypedIndexKind::String,
            Self::Date(_) => TypedIndexKind::Date,
            Self::LocalDateTime(_) => TypedIndexKind::LocalDateTime,
            Self::Uuid(_) => TypedIndexKind::Uuid,
        }
    }

    /// Return total row cardinality across all indexed keys.
    #[must_use]
    pub fn cardinality(&self) -> u64 {
        match self {
            Self::I64(index) => cardinality(index),
            Self::F64(index) => cardinality(index),
            Self::String(index) => cardinality(index),
            Self::Date(index) => cardinality(index),
            Self::LocalDateTime(index) => cardinality(index),
            Self::Uuid(index) => cardinality(index),
        }
    }

    /// Insert `row` into the bitmap for `value`.
    pub(crate) fn insert(&mut self, value: &Value, row: u32) -> Result<(), TypedIndexValueError> {
        let expected_kind = self.kind();
        match (
            self,
            typed_key(value).map_err(|err| err.with_expected(expected_kind))?,
        ) {
            (Self::I64(index), TypedKey::I64(key)) => {
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
        match (
            self,
            typed_key(value).map_err(|err| err.with_expected(expected_kind))?,
        ) {
            (Self::I64(index), TypedKey::I64(key)) => {
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
    #[must_use]
    pub(crate) fn lookup_eq(&self, value: &Value) -> Option<Cow<'_, RoaringBitmap>> {
        match (self, typed_key(value).ok()?) {
            (Self::I64(index), TypedKey::I64(key)) => Some(cow_or_empty(index.get(&key))),
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
            Self::I64(index) => {
                let start = bound_to_key(range.start_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::I64(key)) => Some(key),
                    _ => None,
                })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::I64(key)) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::F64(index) => {
                let start = bound_to_key(range.start_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::F64(key)) => Some(key),
                    _ => None,
                })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::F64(key)) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            // Why: `IStr` ordering is admission-order, not lexicographic.
            // BRIEF-92 makes the v1.0 correctness cut by forcing runtime scan
            // fallback for string ranges until a string-bytes secondary index
            // exists.
            Self::String(_) => None,
            Self::Date(index) => {
                let start = bound_to_key(range.start_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::Date(key)) => Some(key),
                    _ => None,
                })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::Date(key)) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::LocalDateTime(index) => {
                let start = bound_to_key(range.start_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::LocalDateTime(key)) => Some(key),
                    _ => None,
                })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::LocalDateTime(key)) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::Uuid(index) => {
                let start = bound_to_key(range.start_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::Uuid(key)) => Some(key),
                    _ => None,
                })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key(value) {
                    Ok(TypedKey::Uuid(key)) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
        }
    }

    /// Return the union of string-key rows whose key starts with `prefix`.
    ///
    /// This scans every key in the BTreeMap and runs `starts_with` per
    /// entry — O(total cardinality), not O(matching prefix span). The
    /// reason is that `IStr` ordering is **interner-key order** (allocation
    /// order), not lexicographic — see `selene_core::IStr` rustdoc. So a
    /// `BTreeMap<IStr, _>::range` walk over a string-prefix interval is
    /// not possible; lex-equivalent keys can be scattered throughout the
    /// map. BRIEF-92 applies the same v1.0 correctness cut to string range
    /// lookups by returning `None` from [`Self::lookup_range`], letting
    /// runtime scan fallback preserve query semantics until a string-bytes
    /// secondary index lands in a future brief.
    #[must_use]
    pub(crate) fn lookup_prefix(&self, prefix: &str) -> Option<RoaringBitmap> {
        match self {
            Self::String(index) => {
                let mut result = RoaringBitmap::new();
                for (key, bitmap) in index {
                    if key.as_str().starts_with(prefix) {
                        insert_all(&mut result, bitmap);
                    }
                }
                Some(result)
            }
            _ => None,
        }
    }

    /// Return true when two values address the same key in this index.
    ///
    /// This lets update maintenance avoid touching an index when a mutation
    /// changed unrelated node columns.
    pub(crate) fn values_share_key(&self, lhs: &Value, rhs: &Value) -> bool {
        match (self, typed_key(lhs), typed_key(rhs)) {
            (Self::I64(_), Ok(TypedKey::I64(lhs)), Ok(TypedKey::I64(rhs))) => lhs == rhs,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    fn with_expected(self, expected_kind: TypedIndexKind) -> Self {
        match self {
            Self::KindMismatch { observed, .. } => Self::KindMismatch {
                expected_kind,
                observed,
            },
            Self::NaN { .. } => Self::NaN { expected_kind },
        }
    }

    /// Return the expected index kind.
    pub(crate) const fn expected_kind(self) -> TypedIndexKind {
        match self {
            Self::KindMismatch { expected_kind, .. } | Self::NaN { expected_kind } => expected_kind,
        }
    }

    /// Return the observed value description.
    pub(crate) const fn observed(self) -> &'static str {
        match self {
            Self::KindMismatch { observed, .. } => observed,
            Self::NaN { .. } => "NaN",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypedKey {
    I64(i64),
    F64(NotNanF64),
    String(IStr),
    Date(jiff::civil::Date),
    LocalDateTime(jiff::civil::DateTime),
    Uuid(uuid::Uuid),
}

impl TypedKey {
    const fn observed(self) -> &'static str {
        match self {
            Self::I64(_) => "Int",
            Self::F64(_) => "Float",
            Self::String(_) => "String",
            Self::Date(_) => "Date",
            Self::LocalDateTime(_) => "LocalDateTime",
            Self::Uuid(_) => "Uuid",
        }
    }
}

fn typed_key(value: &Value) -> Result<TypedKey, TypedIndexValueError> {
    match value {
        Value::Int(value) => Ok(TypedKey::I64(*value)),
        Value::Float(value) => NotNanF64::new(*value)
            .map(TypedKey::F64)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        Value::String(value) => Ok(TypedKey::String(*value)),
        Value::Date(value) => Ok(TypedKey::Date(*value)),
        Value::LocalDateTime(value) => Ok(TypedKey::LocalDateTime(*value)),
        Value::Uuid(value) => Ok(TypedKey::Uuid(*value)),
        _ => Err(TypedIndexValueError::KindMismatch {
            expected_kind: TypedIndexKind::I64,
            observed: observed_value_kind(value),
        }),
    }
}

pub(crate) fn observed_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Uint(_) => "Uint",
        Value::Int128(_) => "Int128",
        Value::Uint128(_) => "Uint128",
        Value::Float(value) if value.is_nan() => "NaN",
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
        Value::Null => "Null",
        Value::Uuid(_) => "Uuid",
        _ => "Unknown",
    }
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
        insert_all(&mut result, bitmap);
    }
    result
}

fn insert_all(target: &mut RoaringBitmap, source: &RoaringBitmap) {
    for row in source {
        target.insert(row);
    }
}

#[cfg(test)]
#[path = "typed_index_tests.rs"]
mod tests;
