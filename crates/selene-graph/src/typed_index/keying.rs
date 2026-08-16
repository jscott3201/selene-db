//! Key coercion and ordered range helpers for typed property indexes.

use std::collections::BTreeMap;
use std::ops::{Bound, RangeBounds};

use roaring::RoaringBitmap;
use selene_core::{DbString, DurationOrderKey, Value, duration_order_key};

use super::{NotNanError, NotNanF32, NotNanF64, TypedIndexKind};

/// Diagnostic spelling of a rejected NaN in `observed` positions.
///
/// One constant rather than a literal per site: the composite path used to
/// recover "this was a NaN, not a kind mismatch" by comparing `observed`
/// against this text, so the two spellings had to agree for a correctness
/// classifier to work. That classifier is now a discriminant test
/// (`CompositeIndexValueError::ComponentNaN`) and this text is purely
/// diagnostic, but keeping one definition keeps the two paths' messages
/// identical.
pub(crate) const NAN_OBSERVED: &str = "NaN";

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
            Self::NaN { .. } => NAN_OBSERVED,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TypedKey {
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Decimal(rust_decimal::Decimal),
    F32(NotNanF32),
    F64(NotNanF64),
    String(DbString),
    Date(jiff::civil::Date),
    LocalDateTime(jiff::civil::DateTime),
    ZonedDateTime(jiff::Zoned),
    LocalTime(jiff::civil::Time),
    ZonedTime(jiff::Zoned),
    Duration(DurationOrderKey),
    Uuid(uuid::Uuid),
}

impl TypedKey {
    pub(super) const fn observed(&self) -> &'static str {
        match self {
            Self::Bool(_) => "Bool",
            Self::I64(_) => "Int",
            Self::U64(_) => "Uint",
            Self::I128(_) => "Int128",
            Self::U128(_) => "Uint128",
            Self::Decimal(_) => "Decimal",
            Self::F32(_) => "Float32",
            Self::F64(_) => "Float",
            Self::String(_) => "String",
            Self::Date(_) => "Date",
            Self::LocalDateTime(_) => "LocalDateTime",
            Self::ZonedDateTime(_) => "ZonedDateTime",
            Self::LocalTime(_) => "LocalTime",
            Self::ZonedTime(_) => "ZonedTime",
            Self::Duration(_) => "Duration",
            Self::Uuid(_) => "Uuid",
        }
    }
}

/// Coerce `value` into a [`TypedKey`].
pub(super) fn typed_key(
    value: &Value,
    expected_kind: TypedIndexKind,
) -> Result<TypedKey, TypedIndexValueError> {
    match value {
        Value::Bool(value) => Ok(TypedKey::Bool(*value)),
        Value::Int(value) => Ok(TypedKey::I64(*value)),
        Value::Uint(value) => Ok(TypedKey::U64(*value)),
        Value::Int128(value) => Ok(TypedKey::I128(*value)),
        Value::Uint128(value) => Ok(TypedKey::U128(*value)),
        Value::Decimal(value) => Ok(TypedKey::Decimal(*value)),
        Value::Float32(value) => NotNanF32::new(*value)
            .map(TypedKey::F32)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F32,
            }),
        Value::Float(value) => NotNanF64::new(*value)
            .map(TypedKey::F64)
            .map_err(|NotNanError| TypedIndexValueError::NaN {
                expected_kind: TypedIndexKind::F64,
            }),
        Value::String(value) => Ok(TypedKey::String(value.clone())),
        Value::Date(value) => Ok(TypedKey::Date(*value)),
        Value::LocalDateTime(value) => Ok(TypedKey::LocalDateTime(*value)),
        Value::ZonedDateTime(value) => Ok(TypedKey::ZonedDateTime((**value).clone())),
        Value::LocalTime(value) => Ok(TypedKey::LocalTime(*value)),
        Value::ZonedTime(value) => Ok(TypedKey::ZonedTime((**value).clone())),
        Value::Duration(value) => Ok(TypedKey::Duration(duration_order_key(value))),
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

/// Compare two raw values bit-exactly for floats, by value otherwise.
///
/// Reached only when at least one side cannot be coerced to the index's kind,
/// so neither side is keyable and the comparison decides whether maintenance
/// may be skipped. It stays bit-exact — and so reports the two signed zeros as
/// different — deliberately: the only cost is a redundant remove+insert, and
/// both halves re-coerce through [`typed_key`], which agrees on the outcome.
///
/// Canonicalising here instead of in the key constructors would invert that
/// safety. It would report `-0.0` and `0.0` as the same raw value and skip
/// maintenance while the key underneath still moved.
pub(super) fn raw_value_same(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Float(lhs), Value::Float(rhs)) => lhs.to_bits() == rhs.to_bits(),
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs.to_bits() == rhs.to_bits(),
        _ => lhs == rhs,
    }
}

pub(super) fn typed_range_union<K, R>(
    index: &BTreeMap<K, RoaringBitmap>,
    range: &R,
    kind: TypedIndexKind,
    convert: impl Fn(TypedKey) -> Option<K>,
) -> Option<RoaringBitmap>
where
    K: Ord,
    R: RangeBounds<Value> + ?Sized,
{
    let start = bound_to_key(range.start_bound(), |value| {
        typed_key(value, kind).ok().and_then(&convert)
    })?;
    let end = bound_to_key(range.end_bound(), |value| {
        typed_key(value, kind).ok().and_then(&convert)
    })?;
    Some(range_union(index, &start, &end))
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
        result |= bitmap;
    }
    result
}
