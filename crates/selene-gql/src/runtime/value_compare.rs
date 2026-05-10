//! Runtime value comparison helpers.

use std::cmp::Ordering;

use selene_core::Value;

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
    numeric_equal(lhs, rhs).unwrap_or(lhs == rhs)
}

pub(crate) fn compare_non_null(lhs: &Value, rhs: &Value) -> Option<Ordering> {
    debug_assert!(!matches!(lhs, Value::Null));
    debug_assert!(!matches!(rhs, Value::Null));
    match (lhs, rhs) {
        (Value::Bool(lhs), Value::Bool(rhs)) => Some(lhs.cmp(rhs)),
        (Value::String(lhs), Value::String(rhs)) => Some(lhs.as_str().cmp(rhs.as_str())),
        _ => numeric_compare(lhs, rhs),
    }
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
        (Value::Date(lhs), Value::Date(rhs)) => lhs.cmp(rhs),
        (Value::LocalDateTime(lhs), Value::LocalDateTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedDateTime(lhs), Value::ZonedDateTime(rhs)) => lhs.cmp(rhs),
        (Value::LocalTime(lhs), Value::LocalTime(rhs)) => lhs.cmp(rhs),
        (Value::ZonedTime(lhs), Value::ZonedTime(rhs)) => lhs.cmp(rhs),
        (Value::Duration(_), Value::Duration(_)) => duration_key(lhs).cmp(&duration_key(rhs)),
        (Value::Bytes(lhs), Value::Bytes(rhs)) => lhs.as_ref().cmp(rhs.as_ref()),
        (Value::Uuid(lhs), Value::Uuid(rhs)) => lhs.cmp(rhs),
        (Value::NodeRef(lhs), Value::NodeRef(rhs)) => lhs.cmp(rhs),
        (Value::EdgeRef(lhs), Value::EdgeRef(rhs)) => lhs.cmp(rhs),
        (Value::Decimal(lhs), Value::Decimal(rhs)) => lhs.cmp(rhs),
        (Value::Int128(lhs), Value::Int128(rhs)) => lhs.cmp(rhs),
        (Value::Uint128(lhs), Value::Uint128(rhs)) => lhs.cmp(rhs),
        _ => numeric_compare(lhs, rhs)
            .or_else(|| compare_non_null(lhs, rhs))
            .unwrap_or_else(|| value_rank(lhs).cmp(&value_rank(rhs))),
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
        (Value::Int(lhs), Value::Float32(rhs)) => i64_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Int(rhs)) => lhs.partial_cmp(&i64_to_f32_exact(*rhs)?),
        (Value::Uint(lhs), Value::Float32(rhs)) => u64_to_f32_exact(*lhs)?.partial_cmp(rhs),
        (Value::Float32(lhs), Value::Uint(rhs)) => lhs.partial_cmp(&u64_to_f32_exact(*rhs)?),
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

fn i64_to_f64_exact(value: i64) -> Option<f64> {
    u64_representable_by_binary_float(value.unsigned_abs(), F64_SIGNIFICAND_BITS)
        .then_some(value as f64)
}

fn u64_to_f64_exact(value: u64) -> Option<f64> {
    u64_representable_by_binary_float(value, F64_SIGNIFICAND_BITS).then_some(value as f64)
}

fn i64_to_f32_exact(value: i64) -> Option<f32> {
    u64_representable_by_binary_float(value.unsigned_abs(), F32_SIGNIFICAND_BITS)
        .then_some(value as f32)
}

fn u64_to_f32_exact(value: u64) -> Option<f32> {
    u64_representable_by_binary_float(value, F32_SIGNIFICAND_BITS).then_some(value as f32)
}

fn u64_representable_by_binary_float(value: u64, significand_bits: u32) -> bool {
    if value == 0 {
        return true;
    }
    let exponent = u64::BITS - 1 - value.leading_zeros();
    if exponent < significand_bits {
        return true;
    }
    let low_bits = exponent + 1 - significand_bits;
    let mask = (1_u64 << low_bits) - 1;
    value & mask == 0
}

#[cfg(test)]
mod tests {
    use std::{cmp::Ordering, sync::Arc};

    use selene_core::{EdgeId, NodeId, Value};

    use super::{NullSortOrder, compare_for_sort};

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
