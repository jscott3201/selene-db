use std::ops::Bound;

use jiff::civil::time;

use super::*;

#[test]
fn f32_range_scan_uses_total_float_order() {
    let mut index = TypedIndex::new(TypedIndexKind::F32);
    for (row, value) in [
        (0, f32::NEG_INFINITY),
        (1, -1.0_f32),
        (2, 0.0_f32),
        (3, 1.0_f32),
    ] {
        index.insert(&Value::Float32(value), row).unwrap();
    }

    let result = index
        .lookup_range(Value::Float32(-1.0_f32)..=Value::Float32(1.0_f32))
        .expect("f32 range kind matches");

    assert!(!result.contains(0));
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(result.contains(3));
}

/// Both signed zeros key together, so a bound written with either sign selects
/// the same rows a scan would. `total_cmp` alone would sort `-0.0` below `0.0`
/// and let `> -0.0` return a `0.0` row that `0.0 > -0.0` rejects.
#[test]
fn float_range_bounds_ignore_the_sign_of_zero() {
    let mut index = TypedIndex::new(TypedIndexKind::F64);
    for (row, value) in [(0, -1.0_f64), (1, -0.0_f64), (2, 0.0_f64), (3, 1.0_f64)] {
        index.insert(&Value::Float(value), row).unwrap();
    }

    for bound in [-0.0_f64, 0.0_f64] {
        let exclusive = index
            .lookup_range((
                Bound::Excluded(Value::Float(bound)),
                Bound::Unbounded::<Value>,
            ))
            .expect("f64 range kind matches");
        assert!(!exclusive.contains(1), "-0.0 row excluded by > {bound}");
        assert!(!exclusive.contains(2), "0.0 row excluded by > {bound}");
        assert!(exclusive.contains(3), "1.0 row retained by > {bound}");

        let inclusive = index
            .lookup_range((
                Bound::Included(Value::Float(bound)),
                Bound::Unbounded::<Value>,
            ))
            .expect("f64 range kind matches");
        assert!(!inclusive.contains(0), "-1.0 row excluded by >= {bound}");
        assert!(inclusive.contains(1), "-0.0 row retained by >= {bound}");
        assert!(inclusive.contains(2), "0.0 row retained by >= {bound}");
    }
}

#[test]
fn signed_zeros_share_one_equality_key() {
    let mut index = TypedIndex::new(TypedIndexKind::F64);
    index.insert(&Value::Float(-0.0), 1).unwrap();
    index.insert(&Value::Float(0.0), 2).unwrap();

    for probe in [-0.0_f64, 0.0_f64] {
        let hits = index
            .lookup_eq(&Value::Float(probe))
            .expect("f64 equality kind matches");
        assert_eq!(hits.len(), 2, "= {probe} returns both signed-zero rows");
        assert!(hits.contains(1) && hits.contains(2));
    }
}

#[test]
fn temporal_range_scans_use_value_order() {
    let mut local_time = TypedIndex::new(TypedIndexKind::LocalTime);
    for (row, value) in [
        (0, time(9, 0, 0, 0)),
        (1, time(12, 0, 0, 0)),
        (2, time(15, 0, 0, 0)),
    ] {
        local_time.insert(&Value::LocalTime(value), row).unwrap();
    }
    let result = local_time
        .lookup_range(Value::LocalTime(time(10, 0, 0, 0))..=Value::LocalTime(time(15, 0, 0, 0)))
        .expect("local-time range kind matches");
    assert!(!result.contains(0));
    assert!(result.contains(1));
    assert!(result.contains(2));

    let mut zoned_time = TypedIndex::new(TypedIndexKind::ZonedTime);
    let early = Value::ZonedTime(Box::new(
        "2026-05-07T09:00:00-04[America/New_York]".parse().unwrap(),
    ));
    let late = Value::ZonedTime(Box::new(
        "2026-05-07T15:00:00-04[America/New_York]".parse().unwrap(),
    ));
    zoned_time.insert(&early, 0).unwrap();
    zoned_time.insert(&late, 1).unwrap();
    let result = zoned_time
        .lookup_range(early.clone()..=late)
        .expect("zoned-time range kind matches");
    assert!(result.contains(0));
    assert!(result.contains(1));
}

#[test]
fn duration_range_scan_uses_shared_duration_order_key() {
    let mut index = TypedIndex::new(TypedIndexKind::Duration);
    for (row, value) in [(0, "P1M"), (1, "PT1H"), (2, "PT2H"), (3, "P1DT1H")] {
        index
            .insert(&Value::Duration(Box::new(value.parse().unwrap())), row)
            .unwrap();
    }

    let result = index
        .lookup_range(
            Value::Duration(Box::new("PT1H".parse().unwrap()))
                ..=Value::Duration(Box::new("P1DT1H".parse().unwrap())),
        )
        .expect("duration range kind matches");

    assert!(
        !result.contains(0),
        "year/month duration is outside day-time range"
    );
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(result.contains(3));
}

#[test]
fn range_scan_honors_included_and_excluded_bounds() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    for (row, value) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
        index.insert(&Value::Int(value), row).unwrap();
    }
    let result = index
        .lookup_range(Value::Int(2)..Value::Int(4))
        .expect("range kind matches");
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(!result.contains(0));
    assert!(!result.contains(3));
}

#[test]
fn bool_range_scan_uses_false_then_true_order() {
    let mut index = TypedIndex::new(TypedIndexKind::Bool);
    index.insert(&Value::Bool(false), 0).unwrap();
    index.insert(&Value::Bool(true), 1).unwrap();

    let result = index
        .lookup_range(Value::Bool(false)..Value::Bool(true))
        .expect("bool range kind matches");

    assert!(result.contains(0));
    assert!(!result.contains(1), "exclusive high endpoint excluded");
}

#[test]
fn u64_range_scan_uses_unsigned_order() {
    let mut index = TypedIndex::new(TypedIndexKind::U64);
    for (row, value) in [(0, 0), (1, 1), (2, u64::MAX)] {
        index.insert(&Value::Uint(value), row).unwrap();
    }

    let result = index
        .lookup_range(Value::Uint(1)..=Value::Uint(u64::MAX))
        .expect("u64 range kind matches");

    assert!(!result.contains(0));
    assert!(result.contains(1));
    assert!(result.contains(2));
}

#[test]
fn i128_range_scan_uses_wide_signed_order() {
    let mut index = TypedIndex::new(TypedIndexKind::I128);
    for (row, value) in [
        (0, i128::MIN),
        (1, i64::MIN as i128 - 1),
        (2, -1),
        (3, i128::MAX),
    ] {
        index.insert(&Value::Int128(value), row).unwrap();
    }

    let result = index
        .lookup_range(Value::Int128(i128::MIN)..=Value::Int128(-1))
        .expect("i128 range kind matches");

    assert!(result.contains(0));
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(!result.contains(3));
}

#[test]
fn u128_range_scan_uses_wide_unsigned_order() {
    let mut index = TypedIndex::new(TypedIndexKind::U128);
    for (row, value) in [
        (0, 0),
        (1, u64::MAX as u128 + 1),
        (2, u128::MAX - 1),
        (3, u128::MAX),
    ] {
        index.insert(&Value::Uint128(value), row).unwrap();
    }

    let result = index
        .lookup_range(Value::Uint128(u64::MAX as u128 + 1)..Value::Uint128(u128::MAX))
        .expect("u128 range kind matches");

    assert!(!result.contains(0));
    assert!(result.contains(1));
    assert!(result.contains(2));
    assert!(!result.contains(3), "exclusive high endpoint excluded");
}

#[test]
fn decimal_range_scan_uses_numeric_order() {
    let mut index = TypedIndex::new(TypedIndexKind::Decimal);
    for (row, value) in [
        (0, decimal("-1.25")),
        (1, decimal("0.10")),
        (2, decimal("1.5")),
        (3, decimal("10.00")),
    ] {
        index.insert(&Value::Decimal(value), row).unwrap();
    }

    let result = index
        .lookup_range(Value::Decimal(decimal("0.1"))..Value::Decimal(decimal("2")))
        .expect("decimal range kind matches");

    assert!(!result.contains(0));
    assert!(result.contains(1), "0.10 equals inclusive low endpoint 0.1");
    assert!(result.contains(2));
    assert!(!result.contains(3));
}
