//! Lookup helpers for built-in typed value indexes.

use std::borrow::Cow;
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::Value;

use super::keying::{TypedKey, raw_value_same, typed_key, typed_range_union};
use super::{TypedIndex, TypedIndexKind};

impl TypedIndex {
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
            // range directly, preserving the previous scan semantics while
            // avoiding a full cardinality walk.
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
    /// `DbString` orders lexicographically, so every key starting with `prefix`
    /// forms a contiguous run beginning at the first key `>= prefix`. The walk
    /// stops at the first non-matching key.
    #[must_use]
    pub(crate) fn lookup_prefix(&self, prefix: &str) -> Option<RoaringBitmap> {
        match self {
            Self::String(index) => {
                // An over-cap prefix cannot match a stored key, because stored
                // strings are admitted through the same `DbString` cap.
                let Ok(lo_key) = selene_core::db_string(prefix) else {
                    return Some(RoaringBitmap::new());
                };
                let mut result = RoaringBitmap::new();
                for (key, bitmap) in index.range((Bound::Included(lo_key), Bound::Unbounded)) {
                    if !key.as_str().starts_with(prefix) {
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

fn cow_or_empty(bitmap: Option<&RoaringBitmap>) -> Cow<'_, RoaringBitmap> {
    bitmap
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(RoaringBitmap::new()))
}
