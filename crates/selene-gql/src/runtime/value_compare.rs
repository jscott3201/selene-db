//! Runtime value comparison helpers.
//!
//! GQL predicate equality lives here instead of on `Value::PartialEq` so row
//! keys and snapshot diffs can stay structurally stable while GQL `=` still
//! propagates NULL and NaN through nested records.

use std::cmp::Ordering;

use selene_core::{Record, RecordTyped, Value};

const F32_SIGNIFICAND_BITS: u32 = 24;
const F64_SIGNIFICAND_BITS: u32 = 53;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub(crate) enum NullSortOrder {
    First,
    Last,
}

pub(crate) fn equal_non_null(lhs: &Value, rhs: &Value) -> bool {
    debug_assert!(!matches!(lhs, Value::Null));
    debug_assert!(!matches!(rhs, Value::Null));
    if let Some(equal) = numeric_equal(lhs, rhs).or_else(|| string_equal(lhs, rhs)) {
        return equal;
    }
    match (lhs, rhs) {
        // Records compare under a field-name bijection (GQLRT-14), so the key
        // regime that backs DISTINCT / GROUP BY / set-ops must NOT use the
        // positional `Value::PartialEq`. Field values use 2VL identity here
        // (NULL == NULL is `true` for keying); the 3VL `=` operator path lives
        // in `gql_equal_non_null`.
        (Value::Record(lhs), Value::Record(rhs)) => record_key_equal(lhs, rhs),
        // Lists are positional, but a list of records must recurse through the
        // name-keyed record equality rather than positional `PartialEq`.
        (Value::List(lhs), Value::List(rhs)) => {
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs)
                    .all(|(lhs, rhs)| value_key_equal(lhs, rhs))
        }
        _ => lhs == rhs,
    }
}

/// 2VL-identity equality used by the DISTINCT / GROUP BY / set-op key regime.
///
/// Unlike the 3VL `=` operator, `NULL` and `NaN` are definite, self-equal key
/// values: the `equal_non_null` cross-type numeric/string collapse applies
/// first, then a `Value::PartialEq` fallback restores the round-trip identity
/// `NaN == NaN` and `-0.0 == 0.0` that the variant-strict storage relies on.
fn value_key_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        _ => equal_non_null(lhs, rhs) || lhs == rhs,
    }
}

/// Field-name-keyed record equality for the key regime (2VL identity).
fn record_key_equal(lhs: &Record, rhs: &Record) -> bool {
    match (lhs, rhs) {
        (Record::Open(lhs), Record::Open(rhs)) => {
            lhs.len() == rhs.len()
                && lhs.iter().all(|(lhs_key, lhs_value)| {
                    rhs.iter()
                        .find(|(rhs_key, _)| rhs_key == lhs_key)
                        .is_some_and(|(_, rhs_value)| value_key_equal(lhs_value, rhs_value))
                })
        }
        _ => lhs == rhs,
    }
}

pub(crate) fn gql_equal_non_null(lhs: &Value, rhs: &Value) -> Option<bool> {
    debug_assert!(!matches!(lhs, Value::Null));
    debug_assert!(!matches!(rhs, Value::Null));
    // A NaN float meeting any numeric operand (including Decimal) yields the
    // 3VL "unknown" outcome, never a spurious false from the fall-through path.
    if (float_is_nan(lhs) && is_numeric(rhs)) || (float_is_nan(rhs) && is_numeric(lhs)) {
        return None;
    }
    match (lhs, rhs) {
        (Value::Record(lhs), Value::Record(rhs)) => return record_gql_equal(lhs, rhs),
        (Value::RecordTyped(lhs), Value::RecordTyped(rhs)) => {
            return typed_record_gql_equal(lhs, rhs);
        }
        _ => {}
    }
    Some(equal_non_null(lhs, rhs))
}

/// Compare non-null values for predicate ordering.
///
/// TODO(BRIEF-followup): NodeRef/EdgeRef/Uuid predicate ordering awaits analyzer policy.
pub(crate) fn compare_non_null(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    debug_assert!(!matches!(lhs, Value::Null));
    debug_assert!(!matches!(rhs, Value::Null));
    compare_value_pair(lhs, rhs)
}

pub(crate) fn compare_for_sort(lhs: &Value, rhs: &Value, nulls: NullSortOrder) -> Ordering {
    match (lhs, rhs) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => match nulls {
            NullSortOrder::First => Ordering::Less,
            NullSortOrder::Last => Ordering::Greater,
        },
        (_, Value::Null) => match nulls {
            NullSortOrder::First => Ordering::Greater,
            NullSortOrder::Last => Ordering::Less,
        },
        _ => compare_non_null_for_sort(lhs, rhs),
    }
}

fn compare_non_null_for_sort(lhs: &Value, rhs: &Value) -> Ordering {
    match (lhs, rhs) {
        (Value::Float(lhs), Value::Float(rhs)) => lhs.total_cmp(rhs),
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs.total_cmp(rhs),
        (Value::Float(lhs), Value::Float32(rhs)) => lhs.total_cmp(&f64::from(*rhs)),
        (Value::Float32(lhs), Value::Float(rhs)) => f64::from(*lhs).total_cmp(rhs),
        (Value::Uuid(lhs), Value::Uuid(rhs)) => lhs.cmp(rhs),
        (Value::NodeRef(lhs), Value::NodeRef(rhs)) => lhs.cmp(rhs),
        (Value::EdgeRef(lhs), Value::EdgeRef(rhs)) => lhs.cmp(rhs),
        _ => compare_value_pair(lhs, rhs).unwrap_or_else(|| value_rank(lhs).cmp(&value_rank(rhs))),
    }
}

fn compare_value_pair(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    Some(match (lhs, rhs) {
        (Value::Bool(lhs), Value::Bool(rhs)) => lhs.cmp(rhs),
        (Value::String(lhs), Value::String(rhs)) => lhs.as_str().cmp(rhs.as_str()),
        (Value::Date(lhs), Value::Date(rhs)) => lhs.cmp(rhs),
        (Value::LocalDateTime(lhs), Value::LocalDateTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedDateTime(lhs), Value::ZonedDateTime(rhs)) => lhs.cmp(rhs),
        (Value::LocalTime(lhs), Value::LocalTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedTime(lhs), Value::ZonedTime(rhs)) => lhs.cmp(rhs),
        (Value::Duration(_), Value::Duration(_)) => duration_key(lhs).cmp(&duration_key(rhs)),
        (Value::Bytes(lhs), Value::Bytes(rhs)) => lhs.as_ref().cmp(rhs.as_ref()),
        (Value::List(lhs), Value::List(rhs)) => return list_compare(lhs, rhs),
        (Value::Record(lhs), Value::Record(rhs)) => return record_compare(lhs, rhs),
        (Value::RecordTyped(lhs), Value::RecordTyped(rhs)) => {
            return typed_record_compare(lhs, rhs);
        }
        (Value::Vector(lhs), Value::Vector(rhs)) => vector_compare(lhs.as_slice(), rhs.as_slice()),
        (Value::Decimal(lhs), Value::Decimal(rhs)) => lhs.cmp(rhs),
        (Value::Int128(lhs), Value::Int128(rhs)) => lhs.cmp(rhs),
        (Value::Int128(lhs), Value::Int(rhs)) => lhs.cmp(&i128::from(*rhs)),
        (Value::Int(lhs), Value::Int128(rhs)) => i128::from(*lhs).cmp(rhs),
        (Value::Int128(lhs), Value::Uint(rhs)) => lhs.cmp(&i128::from(*rhs)),
        (Value::Uint(lhs), Value::Int128(rhs)) => i128::from(*lhs).cmp(rhs),
        (Value::Uint128(lhs), Value::Uint128(rhs)) => lhs.cmp(rhs),
        (Value::Uint128(lhs), Value::Uint(rhs)) => lhs.cmp(&u128::from(*rhs)),
        (Value::Uint(lhs), Value::Uint128(rhs)) => u128::from(*lhs).cmp(rhs),
        (Value::Uint128(lhs), Value::Int(rhs)) => {
            if *rhs < 0 {
                Ordering::Greater
            } else {
                lhs.cmp(&u128::from(*rhs as u64))
            }
        }
        (Value::Int(lhs), Value::Uint128(rhs)) => {
            if *lhs < 0 {
                Ordering::Less
            } else {
                u128::from(*lhs as u64).cmp(rhs)
            }
        }
        (Value::Int128(lhs), Value::Uint128(rhs)) => i128_cmp_u128(*lhs, *rhs),
        (Value::Uint128(lhs), Value::Int128(rhs)) => i128_cmp_u128(*rhs, *lhs).reverse(),
        _ => return numeric_compare(lhs, rhs),
    })
}

fn vector_compare(lhs: &[f32], rhs: &[f32]) -> Ordering {
    for (&lhs_component, &rhs_component) in lhs.iter().zip(rhs.iter()) {
        if lhs_component == rhs_component {
            continue;
        }
        return lhs_component.total_cmp(&rhs_component);
    }
    lhs.len().cmp(&rhs.len())
}

fn record_gql_equal(lhs: &Record, rhs: &Record) -> Option<bool> {
    match (lhs, rhs) {
        (Record::Open(lhs), Record::Open(rhs)) => {
            // ISO §4.15: records are equal under a field-name bijection, not a
            // positional zip. Differing field sets (arity or names) are a
            // definite `false`; on matching names the field values combine
            // under 3VL (a NULL/incomparable pair yields the unknown outcome,
            // but only after every shared field has been checked for a definite
            // mismatch that short-circuits to `false`).
            if lhs.len() != rhs.len() {
                return Some(false);
            }
            let mut saw_unknown = false;
            for (lhs_key, lhs_value) in lhs {
                let Some((_, rhs_value)) = rhs.iter().find(|(rhs_key, _)| rhs_key == lhs_key)
                else {
                    return Some(false);
                };
                match gql_equal(lhs_value, rhs_value) {
                    Some(true) => {}
                    Some(false) => return Some(false),
                    None => saw_unknown = true,
                }
            }
            if saw_unknown { None } else { Some(true) }
        }
        _ => Some(false),
    }
}

fn typed_record_gql_equal(lhs: &RecordTyped, rhs: &RecordTyped) -> Option<bool> {
    if lhs.type_id != rhs.type_id || lhs.values.len() != rhs.values.len() {
        return Some(false);
    }
    let mut saw_unknown = false;
    for (lhs_value, rhs_value) in lhs.values.iter().zip(rhs.values.iter()) {
        match (lhs_value, rhs_value) {
            (Some(lhs_value), Some(rhs_value)) => match gql_equal(lhs_value, rhs_value) {
                Some(true) => {}
                Some(false) => return Some(false),
                None => saw_unknown = true,
            },
            (None, _) | (_, None) => saw_unknown = true,
        }
    }
    if saw_unknown { None } else { Some(true) }
}

fn gql_equal(lhs: &Value, rhs: &Value) -> Option<bool> {
    match (lhs, rhs) {
        (Value::Null, _) | (_, Value::Null) => None,
        _ => gql_equal_non_null(lhs, rhs),
    }
}

fn record_compare(lhs: &Record, rhs: &Record) -> Option<Ordering> {
    match (lhs, rhs) {
        (Record::Open(lhs), Record::Open(rhs)) => {
            // Order over a field-name-sorted view so permuted-equal records
            // compare `Equal` and the ordering agrees with name-keyed equality
            // (GQLRT-14). Field names sort by string content so the order is
            // stable.
            let lhs = sorted_record_fields(lhs);
            let rhs = sorted_record_fields(rhs);
            for (&(lhs_key, lhs_value), &(rhs_key, rhs_value)) in lhs.iter().zip(rhs.iter()) {
                let key_ordering = lhs_key.as_str().cmp(rhs_key.as_str());
                if !key_ordering.is_eq() {
                    return Some(key_ordering);
                }
                let value_ordering = compare_values(lhs_value, rhs_value)?;
                if !value_ordering.is_eq() {
                    return Some(value_ordering);
                }
            }
            Some(lhs.len().cmp(&rhs.len()))
        }
        _ => None,
    }
}

/// Borrow a record's fields sorted by field-name string content.
///
/// Sort by string content so wire codecs and name-keyed comparisons stay
/// deterministic even if the internal string representation changes again.
fn sorted_record_fields(
    fields: &[(selene_core::DbString, Value)],
) -> Vec<&(selene_core::DbString, Value)> {
    let mut sorted: Vec<&(selene_core::DbString, Value)> = fields.iter().collect();
    sorted.sort_by(|lhs, rhs| lhs.0.as_str().cmp(rhs.0.as_str()));
    sorted
}

/// Compare two list values lexicographically.
///
/// Ordering of list values falls under ISO/IEC 39075:2024 §22.14 "Ordering
/// operations". Lists compare element by element in ascending ordinal position;
/// the first non-identical pair decides, and on an equal prefix the shorter list
/// precedes (cardinality tiebreak). A `NULL` or otherwise incomparable element
/// yields `None` (the 3VL "unknown" outcome), exactly as [`record_compare`]
/// handles record-field values.
fn list_compare(lhs: &[Value], rhs: &[Value]) -> Option<Ordering> {
    for (lhs_value, rhs_value) in lhs.iter().zip(rhs.iter()) {
        let value_ordering = compare_values(lhs_value, rhs_value)?;
        if !value_ordering.is_eq() {
            return Some(value_ordering);
        }
    }
    Some(lhs.len().cmp(&rhs.len()))
}

fn typed_record_compare(lhs: &RecordTyped, rhs: &RecordTyped) -> Option<Ordering> {
    let type_ordering = lhs.type_id.cmp(&rhs.type_id);
    if !type_ordering.is_eq() {
        return Some(type_ordering);
    }
    for (lhs_value, rhs_value) in lhs.values.iter().zip(rhs.values.iter()) {
        let (Some(lhs_value), Some(rhs_value)) = (lhs_value, rhs_value) else {
            return None;
        };
        let value_ordering = compare_values(lhs_value, rhs_value)?;
        if !value_ordering.is_eq() {
            return Some(value_ordering);
        }
    }
    Some(lhs.values.len().cmp(&rhs.values.len()))
}

fn compare_values(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    match (lhs, rhs) {
        (Value::Null, _) | (_, Value::Null) => None,
        _ => compare_non_null(lhs, rhs),
    }
}

fn duration_key(value: &selene_core::Value) -> (i16, i32, i32, i32, i32, i64, i64, i64, i64, i64) {
    let Value::Duration(value) = value else {
        unreachable!("duration_key only receives Value::Duration");
    };
    (
        value.get_years(),
        value.get_months(),
        value.get_weeks(),
        value.get_days(),
        value.get_hours(),
        value.get_minutes(),
        value.get_seconds(),
        value.get_milliseconds(),
        value.get_microseconds(),
        value.get_nanoseconds(),
    )
}

fn numeric_equal(lhs: &Value, rhs: &Value) -> Option<bool> {
    Some(match (lhs, rhs) {
        (Value::Int(lhs), Value::Int(rhs)) => lhs == rhs,
        (Value::Uint(lhs), Value::Uint(rhs)) => lhs == rhs,
        (Value::Int(lhs), Value::Uint(rhs)) => i64_eq_u64(*lhs, *rhs),
        (Value::Uint(lhs), Value::Int(rhs)) => i64_eq_u64(*rhs, *lhs),
        (Value::Float(lhs), Value::Float(rhs)) => lhs == rhs,
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs == rhs,
        (Value::Float(lhs), Value::Float32(rhs)) => *lhs == f64::from(*rhs),
        (Value::Float32(lhs), Value::Float(rhs)) => f64::from(*lhs) == *rhs,
        (Value::Int(lhs), Value::Float(rhs)) => {
            i64_to_f64_exact(*lhs).is_some_and(|lhs| lhs == *rhs)
        }
        (Value::Float(lhs), Value::Int(rhs)) => {
            i64_to_f64_exact(*rhs).is_some_and(|rhs| *lhs == rhs)
        }
        (Value::Uint(lhs), Value::Float(rhs)) => {
            u64_to_f64_exact(*lhs).is_some_and(|lhs| lhs == *rhs)
        }
        (Value::Float(lhs), Value::Uint(rhs)) => {
            u64_to_f64_exact(*rhs).is_some_and(|rhs| *lhs == rhs)
        }
        (Value::Int(lhs), Value::Float32(rhs)) => {
            i64_to_f32_exact(*lhs).is_some_and(|lhs| lhs == *rhs)
        }
        (Value::Float32(lhs), Value::Int(rhs)) => {
            i64_to_f32_exact(*rhs).is_some_and(|rhs| *lhs == rhs)
        }
        (Value::Uint(lhs), Value::Float32(rhs)) => {
            u64_to_f32_exact(*lhs).is_some_and(|lhs| lhs == *rhs)
        }
        (Value::Float32(lhs), Value::Uint(rhs)) => {
            u64_to_f32_exact(*rhs).is_some_and(|rhs| *lhs == rhs)
        }
        // 128-bit integer cross-type — exact integer equality, mirroring the
        // `compare_value_pair` ordering arms (GQLRT-26).
        (Value::Int128(lhs), Value::Int128(rhs)) => lhs == rhs,
        (Value::Uint128(lhs), Value::Uint128(rhs)) => lhs == rhs,
        (Value::Int128(lhs), Value::Int(rhs)) => *lhs == i128::from(*rhs),
        (Value::Int(lhs), Value::Int128(rhs)) => i128::from(*lhs) == *rhs,
        (Value::Int128(lhs), Value::Uint(rhs)) => *lhs == i128::from(*rhs),
        (Value::Uint(lhs), Value::Int128(rhs)) => i128::from(*lhs) == *rhs,
        (Value::Uint128(lhs), Value::Uint(rhs)) => *lhs == u128::from(*rhs),
        (Value::Uint(lhs), Value::Uint128(rhs)) => u128::from(*lhs) == *rhs,
        (Value::Uint128(lhs), Value::Int(rhs)) => *rhs >= 0 && *lhs == u128::from(*rhs as u64),
        (Value::Int(lhs), Value::Uint128(rhs)) => *lhs >= 0 && u128::from(*lhs as u64) == *rhs,
        (Value::Int128(lhs), Value::Uint128(rhs)) => *lhs >= 0 && (*lhs as u128) == *rhs,
        (Value::Uint128(lhs), Value::Int128(rhs)) => *rhs >= 0 && *lhs == (*rhs as u128),
        // 128-bit vs binary float — exact-representability, mirroring
        // `numeric_compare`'s `i128_to_f64_exact` / `u128_to_f64_exact` arms.
        (Value::Int128(lhs), Value::Float(rhs)) => {
            i128_to_f64_exact(*lhs).is_some_and(|l| l == *rhs)
        }
        (Value::Float(lhs), Value::Int128(rhs)) => {
            i128_to_f64_exact(*rhs).is_some_and(|r| *lhs == r)
        }
        (Value::Uint128(lhs), Value::Float(rhs)) => {
            u128_to_f64_exact(*lhs).is_some_and(|l| l == *rhs)
        }
        (Value::Float(lhs), Value::Uint128(rhs)) => {
            u128_to_f64_exact(*rhs).is_some_and(|r| *lhs == r)
        }
        (Value::Int128(lhs), Value::Float32(rhs)) => {
            i128_to_f32_exact(*lhs).is_some_and(|l| l == *rhs)
        }
        (Value::Float32(lhs), Value::Int128(rhs)) => {
            i128_to_f32_exact(*rhs).is_some_and(|r| *lhs == r)
        }
        (Value::Uint128(lhs), Value::Float32(rhs)) => {
            u128_to_f32_exact(*lhs).is_some_and(|l| l == *rhs)
        }
        (Value::Float32(lhs), Value::Uint128(rhs)) => {
            u128_to_f32_exact(*rhs).is_some_and(|r| *lhs == r)
        }
        // Decimal cross-type — exact policy shared with `decimal_compare`.
        (Value::Decimal(lhs), Value::Decimal(rhs)) => lhs == rhs,
        (Value::Decimal(lhs), _) => return decimal_equal(lhs, rhs),
        (_, Value::Decimal(rhs)) => return decimal_equal(rhs, lhs),
        _ => return None,
    })
}

/// Exact ordering of a [`rust_decimal::Decimal`] against another numeric value.
///
/// Decimal↔integer is lossless: an integer that fits inside `Decimal`'s range
/// is converted exactly; one that overflows the range cannot equal any finite
/// Decimal, so ordering falls back to a sign/magnitude comparison. Decimal↔
/// binary-float uses the exact binary expansion of the float
/// (`Decimal::from_f64_retain`), so `0.1_f64` (which is really
/// `0.1000…0055`) is strictly greater than the decimal `0.1`. A non-finite
/// float or an out-of-Decimal-range float orders by sign with `None` for NaN.
fn decimal_compare(lhs: &rust_decimal::Decimal, rhs: &Value) -> Option<Ordering> {
    match rhs {
        Value::Int(rhs) => Some(lhs.cmp(&rust_decimal::Decimal::from(*rhs))),
        Value::Uint(rhs) => Some(lhs.cmp(&rust_decimal::Decimal::from(*rhs))),
        Value::Int128(rhs) => Some(decimal_cmp_i128(lhs, *rhs)),
        Value::Uint128(rhs) => Some(decimal_cmp_u128(lhs, *rhs)),
        Value::Float(rhs) => decimal_cmp_f64(lhs, *rhs),
        Value::Float32(rhs) => decimal_cmp_f64(lhs, f64::from(*rhs)),
        Value::Decimal(rhs) => Some(lhs.cmp(rhs)),
        _ => None,
    }
}

fn decimal_equal(lhs: &rust_decimal::Decimal, rhs: &Value) -> Option<bool> {
    Some(decimal_compare(lhs, rhs)? == Ordering::Equal)
}

fn decimal_cmp_i128(lhs: &rust_decimal::Decimal, rhs: i128) -> Ordering {
    // `try_from_i128_with_scale` is the non-panicking i128→Decimal path
    // (the std `From<i128>`/blanket `TryFrom` impl panics on out-of-range).
    match rust_decimal::Decimal::try_from_i128_with_scale(rhs, 0) {
        Ok(rhs) => lhs.cmp(&rhs),
        // `rhs` magnitude exceeds Decimal's range, so it dominates by sign.
        Err(_) => {
            if rhs < 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
    }
}

fn decimal_cmp_u128(lhs: &rust_decimal::Decimal, rhs: u128) -> Ordering {
    // Decimal's positive range tops out below `i128::MAX`, so any `u128` above
    // it necessarily overflows and is the larger value.
    if rhs > i128::MAX as u128 {
        return Ordering::Less;
    }
    decimal_cmp_i128(lhs, rhs as i128)
}

fn decimal_cmp_f64(lhs: &rust_decimal::Decimal, rhs: f64) -> Option<Ordering> {
    if rhs.is_nan() {
        return None;
    }
    match rust_decimal::Decimal::from_f64_retain(rhs) {
        Some(rhs) => Some(lhs.cmp(&rhs)),
        // `rhs` is ±∞ or overflows Decimal's range; it dominates by sign.
        None => {
            if rhs < 0.0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Less)
            }
        }
    }
}

fn string_equal(lhs: &Value, rhs: &Value) -> Option<bool> {
    Some(match (lhs, rhs) {
        (Value::String(lhs), Value::String(rhs)) => lhs.as_str() == rhs.as_str(),
        _ => return None,
    })
}

fn is_numeric(value: &Value) -> bool {
    matches!(
        value,
        Value::Int(_)
            | Value::Uint(_)
            | Value::Int128(_)
            | Value::Uint128(_)
            | Value::Float(_)
            | Value::Float32(_)
            | Value::Decimal(_)
    )
}

fn float_is_nan(value: &Value) -> bool {
    match value {
        Value::Float(value) => value.is_nan(),
        Value::Float32(value) => value.is_nan(),
        _ => false,
    }
}

fn value_rank(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 0,
        Value::Int(_) => 1,
        Value::Uint(_) => 2,
        Value::Int128(_) => 3,
        Value::Uint128(_) => 4,
        Value::Float(_) => 5,
        Value::Float32(_) => 6,
        Value::Decimal(_) => 7,
        Value::String(_) => 8,
        Value::Bytes(_) => 9,
        Value::List(_) => 10,
        Value::Record(_) => 11,
        Value::RecordTyped(_) => 12,
        Value::Path(_) => 13,
        Value::NodeRef(_) => 14,
        Value::EdgeRef(_) => 15,
        Value::GraphRef(_) => 16,
        Value::TableRef(_) => 17,
        Value::ZonedDateTime(_) => 18,
        Value::LocalDateTime(_) => 19,
        Value::Date(_) => 20,
        Value::ZonedTime(_) => 21,
        Value::LocalTime(_) => 22,
        Value::Duration(_) => 23,
        Value::Extended { .. } => 24,
        Value::Null => 25,
        Value::Uuid(_) => 26,
        Value::Vector(_) => 27,
        // Any future `Value` variant ranks after every enumerated one so it
        // never silently ties with `Vector` (27) in the sort fallback.
        _ => 28,
    }
}

fn numeric_compare(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    match (lhs, rhs) {
        // Decimal cross-type. `compare_value_pair` handles Decimal↔Decimal; the
        // mixed arms live here so `numeric_compare` is the single source of the
        // exact-representability policy that `numeric_equal` mirrors.
        (Value::Decimal(lhs), _) => decimal_compare(lhs, rhs),
        (_, Value::Decimal(rhs)) => decimal_compare(rhs, lhs).map(Ordering::reverse),
        (Value::Int(lhs), Value::Int(rhs)) => Some(lhs.cmp(rhs)),
        (Value::Uint(lhs), Value::Uint(rhs)) => Some(lhs.cmp(rhs)),
        (Value::Int(lhs), Value::Uint(rhs)) => Some(i64_cmp_u64(*lhs, *rhs)),
        (Value::Uint(lhs), Value::Int(rhs)) => Some(i64_cmp_u64(*rhs, *lhs).reverse()),
        (Value::Float(lhs), Value::Float(rhs)) => lhs.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Float32(rhs)) => lhs.partial_cmp(rhs),
        (Value::Float(lhs), Value::Float32(rhs)) => lhs.partial_cmp(&f64::from(*rhs)),
        (Value::Float32(lhs), Value::Float(rhs)) => f64::from(*lhs).partial_cmp(rhs),
        (Value::Int(lhs), Value::Float(rhs)) => i64_to_f64_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float(lhs), Value::Int(rhs)) => lhs.partial_cmp(&i64_to_f64_exact(*rhs)?),
        (Value::Uint(lhs), Value::Float(rhs)) => u64_to_f64_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float(lhs), Value::Uint(rhs)) => lhs.partial_cmp(&u64_to_f64_exact(*rhs)?),
        (Value::Int128(lhs), Value::Float(rhs)) => i128_to_f64_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float(lhs), Value::Int128(rhs)) => lhs.partial_cmp(&i128_to_f64_exact(*rhs)?),
        (Value::Uint128(lhs), Value::Float(rhs)) => u128_to_f64_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float(lhs), Value::Uint128(rhs)) => lhs.partial_cmp(&u128_to_f64_exact(*rhs)?),
        (Value::Int(lhs), Value::Float32(rhs)) => i64_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Int(rhs)) => lhs.partial_cmp(&i64_to_f32_exact(*rhs)?),
        (Value::Uint(lhs), Value::Float32(rhs)) => u64_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Uint(rhs)) => lhs.partial_cmp(&u64_to_f32_exact(*rhs)?),
        (Value::Int128(lhs), Value::Float32(rhs)) => i128_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Int128(rhs)) => lhs.partial_cmp(&i128_to_f32_exact(*rhs)?),
        (Value::Uint128(lhs), Value::Float32(rhs)) => u128_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Uint128(rhs)) => lhs.partial_cmp(&u128_to_f32_exact(*rhs)?),
        _ => None,
    }
}

fn i64_eq_u64(lhs: i64, rhs: u64) -> bool {
    lhs >= 0 && lhs as u64 == rhs
}

fn i64_cmp_u64(lhs: i64, rhs: u64) -> Ordering {
    if lhs < 0 {
        Ordering::Less
    } else {
        (lhs as u64).cmp(&rhs)
    }
}

fn i128_cmp_u128(lhs: i128, rhs: u128) -> Ordering {
    if lhs < 0 {
        Ordering::Less
    } else {
        (lhs as u128).cmp(&rhs)
    }
}

fn i64_to_f64_exact(value: i64) -> Option<f64> {
    u128_representable_by_binary_float(u128::from(value.unsigned_abs()), F64_SIGNIFICAND_BITS)
        .then_some(value as f64)
}

fn u64_to_f64_exact(value: u64) -> Option<f64> {
    u128_representable_by_binary_float(u128::from(value), F64_SIGNIFICAND_BITS)
        .then_some(value as f64)
}

fn i128_to_f64_exact(value: i128) -> Option<f64> {
    u128_representable_by_binary_float(value.unsigned_abs(), F64_SIGNIFICAND_BITS)
        .then_some(value as f64)
}

fn u128_to_f64_exact(value: u128) -> Option<f64> {
    u128_representable_by_binary_float(value, F64_SIGNIFICAND_BITS).then_some(value as f64)
}

fn i64_to_f32_exact(value: i64) -> Option<f32> {
    u128_representable_by_binary_float(u128::from(value.unsigned_abs()), F32_SIGNIFICAND_BITS)
        .then_some(value as f32)
}

fn u64_to_f32_exact(value: u64) -> Option<f32> {
    u128_representable_by_binary_float(u128::from(value), F32_SIGNIFICAND_BITS)
        .then_some(value as f32)
}

fn i128_to_f32_exact(value: i128) -> Option<f32> {
    u128_representable_by_binary_float(value.unsigned_abs(), F32_SIGNIFICAND_BITS)
        .then_some(value as f32)
}

fn u128_to_f32_exact(value: u128) -> Option<f32> {
    u128_representable_by_binary_float(value, F32_SIGNIFICAND_BITS).then_some(value as f32)
}

fn u128_representable_by_binary_float(value: u128, significand_bits: u32) -> bool {
    if value == 0 {
        return true;
    }
    let exponent = u128::BITS - 1 - value.leading_zeros();
    if exponent < significand_bits {
        return true;
    }
    let low_bits = exponent + 1 - significand_bits;
    let mask = (1_u128 << low_bits) - 1;
    value & mask == 0
}

#[cfg(test)]
#[path = "value_compare_tests.rs"]
mod tests;
