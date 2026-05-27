//! Built-in per-`(label, property)` value index. See spec 03 section 5.2.
//!
//! # Caller classes (BRIEF-153)
//!
//! `typed_key_admit` and `typed_key_lookup` split the `Value`→`TypedKey`
//! conversion into three caller classes, each chosen at every call site:
//!
//! - **Write side** ([`TypedIndex::insert`], [`TypedIndex::remove`]):
//!   `typed_key_admit`. Cap exhaustion surfaces as
//!   [`TypedIndexValueError::AdmissionFailed`] and the mutator promotes it
//!   to a hard [`crate::error::GraphError::IndexAdmissionExhausted`]
//!   (GQLSTATUS `5GQL1`).
//! - **Read side** ([`TypedIndex::lookup_eq`], the per-type closures in
//!   [`TypedIndex::lookup_range`]): `typed_key_lookup`. Returning
//!   `Ok(None)` for `STRING` content that is not in the pool renders as an
//!   empty result; the read path MUST NOT admit a new `IStr` just to probe.
//! - **Diff side** ([`TypedIndex::values_share_key`]): `typed_key_lookup`,
//!   with `Ok(None)` falling through to [`raw_value_same`] so the diff
//!   path never charges admission against a no-op update. If the values
//!   genuinely differ, the resulting remove+insert pair admits through the
//!   write path.
//!
//! See `selene_core::value::Value::ExternalString` rustdoc for the
//! variant-strict storage carve-out this enables.
//!
//! The same three-helper split is mirrored in
//! [`crate::composite_typed_index`] for composite indexes.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::{CoreError, IStr, Value};
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
            typed_key_admit(value).map_err(|err| err.with_expected(expected_kind))?,
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
            typed_key_admit(value).map_err(|err| err.with_expected(expected_kind))?,
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
    ///
    /// Returns `None` for kind-mismatched values (callers fall back to a
    /// runtime scan). Returns `Some(empty)` when the value's kind matches
    /// the index but its content is not in any indexed row — including the
    /// case where `value` is a [`Value::ExternalString`] whose content has
    /// never been admitted to the global [`IStr`] pool (the read path
    /// MUST NOT admit; the absence of a pool entry proves the absence of an
    /// indexed row).
    #[must_use]
    pub(crate) fn lookup_eq(&self, value: &Value) -> Option<Cow<'_, RoaringBitmap>> {
        let key = match typed_key_lookup(value) {
            Ok(Some(key)) => key,
            Ok(None) => return Some(Cow::Owned(RoaringBitmap::new())),
            Err(_) => return None,
        };
        match (self, key) {
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
                let start =
                    bound_to_key(range.start_bound(), |value| match typed_key_lookup(value) {
                        Ok(Some(TypedKey::I64(key))) => Some(key),
                        _ => None,
                    })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key_lookup(value) {
                    Ok(Some(TypedKey::I64(key))) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::F64(index) => {
                let start =
                    bound_to_key(range.start_bound(), |value| match typed_key_lookup(value) {
                        Ok(Some(TypedKey::F64(key))) => Some(key),
                        _ => None,
                    })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key_lookup(value) {
                    Ok(Some(TypedKey::F64(key))) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            // Why: `IStr` ordering is admission-order, not lexicographic, so
            // string ranges fall back to runtime scans until a string-bytes
            // secondary index exists.
            Self::String(_) => None,
            Self::Date(index) => {
                let start =
                    bound_to_key(range.start_bound(), |value| match typed_key_lookup(value) {
                        Ok(Some(TypedKey::Date(key))) => Some(key),
                        _ => None,
                    })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key_lookup(value) {
                    Ok(Some(TypedKey::Date(key))) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::LocalDateTime(index) => {
                let start =
                    bound_to_key(range.start_bound(), |value| match typed_key_lookup(value) {
                        Ok(Some(TypedKey::LocalDateTime(key))) => Some(key),
                        _ => None,
                    })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key_lookup(value) {
                    Ok(Some(TypedKey::LocalDateTime(key))) => Some(key),
                    _ => None,
                })?;
                Some(range_union(index, &start, &end))
            }
            Self::Uuid(index) => {
                let start =
                    bound_to_key(range.start_bound(), |value| match typed_key_lookup(value) {
                        Ok(Some(TypedKey::Uuid(key))) => Some(key),
                        _ => None,
                    })?;
                let end = bound_to_key(range.end_bound(), |value| match typed_key_lookup(value) {
                    Ok(Some(TypedKey::Uuid(key))) => Some(key),
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
    /// map. String range lookups return `None` from [`Self::lookup_range`],
    /// letting runtime scan fallback preserve query semantics until a
    /// string-bytes secondary index exists.
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
    /// changed unrelated node columns. Uses [`typed_key_lookup`] (no
    /// admission); when either side cannot be resolved through the existing
    /// IStr pool the diff falls through to [`raw_value_same`] so we never
    /// admit just to compare keys. If raw values differ the update path
    /// fires remove+insert, which then admit through [`typed_key_admit`].
    pub(crate) fn values_share_key(&self, lhs: &Value, rhs: &Value) -> bool {
        match (self, typed_key_lookup(lhs), typed_key_lookup(rhs)) {
            (Self::I64(_), Ok(Some(TypedKey::I64(lhs))), Ok(Some(TypedKey::I64(rhs)))) => {
                lhs == rhs
            }
            (Self::F64(_), Ok(Some(TypedKey::F64(lhs))), Ok(Some(TypedKey::F64(rhs)))) => {
                lhs == rhs
            }
            (Self::String(_), Ok(Some(TypedKey::String(lhs))), Ok(Some(TypedKey::String(rhs)))) => {
                lhs == rhs
            }
            (Self::Date(_), Ok(Some(TypedKey::Date(lhs))), Ok(Some(TypedKey::Date(rhs)))) => {
                lhs == rhs
            }
            (
                Self::LocalDateTime(_),
                Ok(Some(TypedKey::LocalDateTime(lhs))),
                Ok(Some(TypedKey::LocalDateTime(rhs))),
            ) => lhs == rhs,
            (Self::Uuid(_), Ok(Some(TypedKey::Uuid(lhs))), Ok(Some(TypedKey::Uuid(rhs)))) => {
                lhs == rhs
            }
            _ => raw_value_same(lhs, rhs),
        }
    }
}

/// Internal value-admission error for index mutation.
///
/// `Clone + Copy + Eq + PartialEq` are intentionally NOT derived: the
/// `AdmissionFailed` variant carries [`selene_core::CoreError`] which is a
/// `Diagnostic`-bearing source-error type and is non-`Copy`/`Clone` by
/// design.
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
    /// A [`Value::ExternalString`] could not be admitted to the global
    /// [`IStr`] pool for use as a `STRING`-kind index key (typically because
    /// the pool reached [`selene_core::MAX_INTERNED_STRINGS`]).
    ///
    /// Per D20 and the BRIEF-153 carve-out, DDL `INDEXED` is the user's
    /// consent to admit a column's content to the pool; cap exhaustion at
    /// that boundary is a hard error rather than a silent skip. The
    /// mutator promotes this to [`crate::error::GraphError::IndexAdmissionExhausted`].
    AdmissionFailed {
        /// The index kind being updated (always [`TypedIndexKind::String`]
        /// today; future non-STRING admissions could broaden this).
        expected_kind: TypedIndexKind,
        /// Underlying admission failure source.
        reason: CoreError,
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
            Self::AdmissionFailed { reason, .. } => Self::AdmissionFailed {
                expected_kind,
                reason,
            },
        }
    }

    /// Return the expected index kind.
    pub(crate) fn expected_kind(&self) -> TypedIndexKind {
        match self {
            Self::KindMismatch { expected_kind, .. }
            | Self::NaN { expected_kind }
            | Self::AdmissionFailed { expected_kind, .. } => *expected_kind,
        }
    }

    /// Return the observed value description.
    pub(crate) fn observed(&self) -> &'static str {
        match self {
            Self::KindMismatch { observed, .. } => observed,
            Self::NaN { .. } => "NaN",
            Self::AdmissionFailed { .. } => "ExternalString",
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

/// Write-side coercion of `value` into a [`TypedKey`].
///
/// [`Value::ExternalString`] is the only variant that may admit to the
/// global [`IStr`] pool here: when the index is `STRING`-kind and the
/// content is not yet pooled, [`selene_core::intern`] is invoked and the
/// returned handle becomes the index key. Cap exhaustion at the pool
/// surfaces as [`TypedIndexValueError::AdmissionFailed`], which the
/// mutator promotes to a hard
/// [`crate::error::GraphError::IndexAdmissionExhausted`] (BRIEF-153).
fn typed_key_admit(value: &Value) -> Result<TypedKey, TypedIndexValueError> {
    match value {
        Value::Int(value) => Ok(TypedKey::I64(*value)),
        Value::Float(value) => NotNanF64::new(*value)
            .map(TypedKey::F64)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        Value::String(value) => Ok(TypedKey::String(*value)),
        Value::ExternalString(value) => match selene_core::intern(value.as_ref()) {
            Ok(interned) => Ok(TypedKey::String(interned)),
            Err(reason) => Err(TypedIndexValueError::AdmissionFailed {
                expected_kind: TypedIndexKind::String,
                reason,
            }),
        },
        Value::Date(value) => Ok(TypedKey::Date(*value)),
        Value::LocalDateTime(value) => Ok(TypedKey::LocalDateTime(*value)),
        Value::Uuid(value) => Ok(TypedKey::Uuid(*value)),
        _ => Err(TypedIndexValueError::KindMismatch {
            expected_kind: TypedIndexKind::I64,
            observed: observed_value_kind(value),
        }),
    }
}

/// Read-side coercion of `value` into a [`TypedKey`] **without admitting
/// new strings to the global [`IStr`] pool**.
///
/// Returns `Ok(None)` when `value` is a [`Value::ExternalString`] whose
/// content is not yet in the pool — because no row could be keyed on that
/// content, callers render this as an empty result rather than admitting
/// just to probe. `Ok(Some(_))` matches [`typed_key_admit`] for all
/// already-admissible inputs. `Err` retains the kind-mismatch / NaN
/// semantics from the admit path.
fn typed_key_lookup(value: &Value) -> Result<Option<TypedKey>, TypedIndexValueError> {
    match value {
        Value::Int(value) => Ok(Some(TypedKey::I64(*value))),
        Value::Float(value) => NotNanF64::new(*value)
            .map(|key| Some(TypedKey::F64(key)))
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        Value::String(value) => Ok(Some(TypedKey::String(*value))),
        Value::ExternalString(value) => {
            Ok(selene_core::lookup(value.as_ref()).map(TypedKey::String))
        }
        Value::Date(value) => Ok(Some(TypedKey::Date(*value))),
        Value::LocalDateTime(value) => Ok(Some(TypedKey::LocalDateTime(*value))),
        Value::Uuid(value) => Ok(Some(TypedKey::Uuid(*value))),
        _ => Err(TypedIndexValueError::KindMismatch {
            expected_kind: TypedIndexKind::I64,
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
