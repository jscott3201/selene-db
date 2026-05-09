//! Runtime value comparison helpers.

use std::cmp::Ordering;

use selene_core::Value;

const F32_SIGNIFICAND_BITS: u32 = 24;
const F64_SIGNIFICAND_BITS: u32 = 53;

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
