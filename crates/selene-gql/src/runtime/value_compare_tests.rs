//! Unit tests for [`super`] GQL value comparison and ordering.
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
