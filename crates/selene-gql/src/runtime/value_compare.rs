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
    numeric_equal(lhs, rhs)
        .or_else(|| string_equal(lhs, rhs))
        .unwrap_or(lhs == rhs)
}

pub(crate) fn gql_equal_non_null(lhs: &Value, rhs: &Value) -> Option<bool> {
    debug_assert!(!matches!(lhs, Value::Null));
    debug_assert!(!matches!(rhs, Value::Null));
    if (float_is_nan(lhs) || float_is_nan(rhs)) && numeric_equal(lhs, rhs).is_some() {
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
        (Value::String(lhs), Value::ExternalString(rhs)) => lhs.as_str().cmp(rhs.as_ref()),
        (Value::ExternalString(lhs), Value::String(rhs)) => lhs.as_ref().cmp(rhs.as_str()),
        (Value::ExternalString(lhs), Value::ExternalString(rhs)) => lhs.cmp(rhs),
        (Value::Date(lhs), Value::Date(rhs)) => lhs.cmp(rhs),
        (Value::LocalDateTime(lhs), Value::LocalDateTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedDateTime(lhs), Value::ZonedDateTime(rhs)) => lhs.cmp(rhs),
        (Value::LocalTime(lhs), Value::LocalTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedTime(lhs), Value::ZonedTime(rhs)) => lhs.cmp(rhs),
        (Value::Duration(_), Value::Duration(_)) => duration_key(lhs).cmp(&duration_key(rhs)),
        (Value::Bytes(lhs), Value::Bytes(rhs)) => lhs.as_ref().cmp(rhs.as_ref()),
        (Value::Record(lhs), Value::Record(rhs)) => return record_compare(lhs, rhs),
        (Value::RecordTyped(lhs), Value::RecordTyped(rhs)) => {
            return typed_record_compare(lhs, rhs);
        }
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

fn record_gql_equal(lhs: &Record, rhs: &Record) -> Option<bool> {
    match (lhs, rhs) {
        (Record::Open(lhs), Record::Open(rhs)) => {
            if lhs.len() != rhs.len() {
                return Some(false);
            }
            let mut saw_unknown = false;
            for ((lhs_key, lhs_value), (rhs_key, rhs_value)) in lhs.iter().zip(rhs.iter()) {
                if lhs_key != rhs_key {
                    return Some(false);
                }
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
            for ((lhs_key, lhs_value), (rhs_key, rhs_value)) in lhs.iter().zip(rhs.iter()) {
                let key_ordering = lhs_key.cmp(rhs_key);
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
        _ => return None,
    })
}

fn string_equal(lhs: &Value, rhs: &Value) -> Option<bool> {
    Some(match (lhs, rhs) {
        (Value::String(lhs), Value::String(rhs)) => lhs.as_str() == rhs.as_str(),
        (Value::String(lhs), Value::ExternalString(rhs)) => lhs.as_str() == rhs.as_ref(),
        (Value::ExternalString(lhs), Value::String(rhs)) => lhs.as_ref() == rhs.as_str(),
        (Value::ExternalString(lhs), Value::ExternalString(rhs)) => lhs == rhs,
        _ => return None,
    })
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
        Value::ExternalString(_) => 9,
        Value::Bytes(_) => 10,
        Value::List(_) => 11,
        Value::Record(_) => 12,
        Value::RecordTyped(_) => 13,
        Value::Path(_) => 14,
        Value::NodeRef(_) => 15,
        Value::EdgeRef(_) => 16,
        Value::GraphRef(_) => 17,
        Value::TableRef(_) => 18,
        Value::ZonedDateTime(_) => 19,
        Value::LocalDateTime(_) => 20,
        Value::Date(_) => 21,
        Value::ZonedTime(_) => 22,
        Value::LocalTime(_) => 23,
        Value::Duration(_) => 24,
        Value::Extended { .. } => 25,
        Value::Null => 26,
        Value::Uuid(_) => 27,
        _ => 27,
    }
}

fn numeric_compare(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    match (lhs, rhs) {
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
mod tests {
    use std::{cmp::Ordering, sync::Arc};

    use selene_core::{
        EdgeId, NodeId, Record, RecordTypeId, RecordTyped, Value, intern_with_admission,
    };
    use smallvec::smallvec;

    use super::{
        NullSortOrder, compare_for_sort, compare_non_null, equal_non_null, gql_equal_non_null,
    };

    #[test]
    fn string_comparison_accepts_external_string_payloads() {
        let interned = Value::String(intern_with_admission("same").unwrap().0);
        let external_same = Value::ExternalString(Arc::from("same"));
        let external_later = Value::ExternalString(Arc::from("zzz"));

        assert!(equal_non_null(&interned, &external_same));
        assert_eq!(
            compare_non_null(&interned, &external_same),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_non_null(&interned, &external_later),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn equal_non_null_list_nan_returns_true() {
        let lhs = Value::List(vec![Value::Float(f64::NAN)]);
        let rhs = Value::List(vec![Value::Float(f64::NAN)]);

        assert!(equal_non_null(&lhs, &rhs));
    }

    #[test]
    fn equal_non_null_record_nan_returns_true() {
        let key = intern_with_admission("x").unwrap().0;
        let lhs = Value::Record(Box::new(Record::Open(smallvec![(
            key,
            Value::Float(f64::NAN)
        )])));
        let rhs = Value::Record(Box::new(Record::Open(smallvec![(
            key,
            Value::Float(f64::NAN)
        )])));

        assert!(equal_non_null(&lhs, &rhs));
    }

    #[test]
    fn numeric_equal_top_level_float_nan_returns_null() {
        assert_eq!(
            gql_equal_non_null(&Value::Float(f64::NAN), &Value::Float(f64::NAN)),
            None
        );
    }

    #[test]
    fn gql_equal_record_null_field_returns_null() {
        let key = intern_with_admission("x").unwrap().0;
        let lhs = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Null)])));
        let rhs = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Null)])));

        assert!(equal_non_null(&lhs, &rhs));
        assert_eq!(gql_equal_non_null(&lhs, &rhs), None);
    }

    #[test]
    fn gql_equal_typed_record_null_slot_returns_null() {
        let lhs = Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(7),
            values: smallvec![None],
        }));
        let rhs = Value::RecordTyped(Box::new(RecordTyped {
            type_id: RecordTypeId::new(7),
            values: smallvec![None],
        }));

        assert!(equal_non_null(&lhs, &rhs));
        assert_eq!(gql_equal_non_null(&lhs, &rhs), None);
    }

    #[test]
    fn compare_record_with_null_field_returns_null() {
        let key = intern_with_admission("x").unwrap().0;
        let lhs = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Null)])));
        let rhs = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Int(1))])));

        assert_eq!(compare_non_null(&lhs, &rhs), None);
    }

    #[test]
    fn compare_non_null_date_lt_date() {
        assert_eq!(
            compare_non_null(
                &Value::Date("2024-01-01".parse().unwrap()),
                &Value::Date("2026-01-01".parse().unwrap())
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_local_datetime_eq() {
        let lhs = Value::LocalDateTime("2024-01-01T00:00:00".parse().unwrap());
        let rhs = Value::LocalDateTime("2024-01-01T00:00:00".parse().unwrap());

        assert_eq!(compare_non_null(&lhs, &rhs), Some(Ordering::Equal));
    }

    #[test]
    fn compare_non_null_zoned_and_time_values() {
        assert_eq!(
            compare_non_null(
                &Value::ZonedDateTime(
                    "2024-01-01T00:00:00-05:00[America/New_York]"
                        .parse()
                        .unwrap(),
                ),
                &Value::ZonedDateTime(
                    "2026-01-01T00:00:00-05:00[America/New_York]"
                        .parse()
                        .unwrap(),
                ),
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_non_null(
                &Value::LocalTime("01:00:00".parse().unwrap()),
                &Value::LocalTime("02:00:00".parse().unwrap())
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_non_null(
                &Value::ZonedTime(
                    "2024-01-01T01:00:00-05:00[America/New_York]"
                        .parse()
                        .unwrap(),
                ),
                &Value::ZonedTime(
                    "2024-01-01T02:00:00-05:00[America/New_York]"
                        .parse()
                        .unwrap(),
                ),
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_duration_lt() {
        assert_eq!(
            compare_non_null(
                &Value::Duration("PT1S".parse().unwrap()),
                &Value::Duration("PT2S".parse().unwrap())
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_bytes_lex() {
        assert_eq!(
            compare_non_null(
                &Value::Bytes(Arc::from([1_u8, 2])),
                &Value::Bytes(Arc::from([1_u8, 3]))
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_decimal_lt() {
        assert_eq!(
            compare_non_null(
                &Value::Decimal("1.0".parse().unwrap()),
                &Value::Decimal("2.0".parse().unwrap())
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_int128_uint128_cross_succeeds() {
        assert_eq!(
            compare_non_null(&Value::Uint128(1), &Value::Uint128(2)),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_non_null(&Value::Int128(-1), &Value::Uint128(0)),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_non_null(&Value::Int128(1), &Value::Uint128(1)),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_non_null(&Value::Uint128(2), &Value::Int128(1)),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn compare_non_null_int128_vs_int() {
        assert_eq!(
            compare_non_null(
                &Value::Int128(1_000_000_000_000_000_000_000),
                &Value::Int(1)
            ),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn compare_non_null_int_vs_int128() {
        assert_eq!(
            compare_non_null(
                &Value::Int(1),
                &Value::Int128(1_000_000_000_000_000_000_000)
            ),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_non_null_uint128_vs_uint() {
        assert_eq!(
            compare_non_null(&Value::Uint128(u128::MAX), &Value::Uint(1)),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn compare_non_null_uint128_negative_int() {
        assert_eq!(
            compare_non_null(&Value::Uint128(0), &Value::Int(-1)),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn compare_non_null_string_vs_date_returns_none() {
        let string = Value::String(intern_with_admission("2024-01-01").unwrap().0);
        let date = Value::Date("2024-01-01".parse().unwrap());

        assert_eq!(compare_non_null(&string, &date), None);
    }

    #[test]
    fn compare_for_sort_orders_temporal_payloads() {
        assert_sort_less(
            Value::Date("2024-01-01".parse().unwrap()),
            Value::Date("2026-01-01".parse().unwrap()),
        );
        assert_sort_less(
            Value::LocalDateTime("2024-01-01T00:00:00".parse().unwrap()),
            Value::LocalDateTime("2026-01-01T00:00:00".parse().unwrap()),
        );
        assert_sort_less(
            Value::ZonedDateTime(
                "2024-01-01T00:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            ),
            Value::ZonedDateTime(
                "2026-01-01T00:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            ),
        );
        assert_sort_less(
            Value::LocalTime("01:00:00".parse().unwrap()),
            Value::LocalTime("02:00:00".parse().unwrap()),
        );
        assert_sort_less(
            Value::ZonedTime(
                "2024-01-01T01:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            ),
            Value::ZonedTime(
                "2024-01-01T02:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            ),
        );
        assert_sort_less(
            Value::Duration("PT1H".parse().unwrap()),
            Value::Duration("PT2H".parse().unwrap()),
        );
    }

    #[test]
    fn compare_for_sort_orders_extended_scalar_payloads() {
        assert_sort_less(
            Value::Bytes(Arc::from([1_u8])),
            Value::Bytes(Arc::from([2_u8])),
        );
        assert_sort_less(
            Value::Uuid("00000000-0000-0000-0000-000000000001".parse().unwrap()),
            Value::Uuid("00000000-0000-0000-0000-000000000002".parse().unwrap()),
        );
        assert_sort_less(
            Value::NodeRef(NodeId::new(1)),
            Value::NodeRef(NodeId::new(2)),
        );
        assert_sort_less(
            Value::EdgeRef(EdgeId::new(1)),
            Value::EdgeRef(EdgeId::new(2)),
        );
        assert_sort_less(
            Value::Decimal("1.0".parse().unwrap()),
            Value::Decimal("2.0".parse().unwrap()),
        );
        assert_sort_less(Value::Int128(1), Value::Int128(2));
        assert_sort_less(Value::Uint128(1), Value::Uint128(2));
    }

    fn assert_sort_less(lhs: Value, rhs: Value) {
        assert_eq!(
            compare_for_sort(&lhs, &rhs, NullSortOrder::Last),
            Ordering::Less
        );
        assert_eq!(
            compare_for_sort(&rhs, &lhs, NullSortOrder::Last),
            Ordering::Greater
        );
    }
}
