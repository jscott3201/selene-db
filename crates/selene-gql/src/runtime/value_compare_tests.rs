//! Unit tests for [`super`] GQL value comparison and ordering.
use std::{cmp::Ordering, sync::Arc};

use selene_core::{
    EdgeId, NodeId, Record, RecordTypeId, RecordTyped, Value, VectorValue, db_string,
};
use smallvec::smallvec;

use super::{
    NullSortOrder, compare_for_sort, compare_non_null, equal_non_null, gql_equal_non_null,
};

#[test]
fn string_comparison_is_content_based() {
    let same_a = Value::String(db_string("same").unwrap());
    let same_b = Value::String(db_string("same").unwrap());
    let later = Value::String(db_string("zzz").unwrap());

    assert!(equal_non_null(&same_a, &same_b));
    assert_eq!(compare_non_null(&same_a, &same_b), Some(Ordering::Equal));
    assert_eq!(compare_non_null(&same_a, &later), Some(Ordering::Less));
}

#[test]
fn vector_comparison_is_componentwise() {
    let lhs = Value::Vector(VectorValue::new(vec![1.0, -0.0, 3.0]).unwrap());
    let same = Value::Vector(VectorValue::new(vec![1.0, 0.0, 3.0]).unwrap());
    let higher_component = Value::Vector(VectorValue::new(vec![1.0, 0.5, 3.0]).unwrap());
    let longer = Value::Vector(VectorValue::new(vec![1.0, 0.0, 3.0, 4.0]).unwrap());

    assert!(equal_non_null(&lhs, &same));
    assert_eq!(gql_equal_non_null(&lhs, &same), Some(true));
    assert_eq!(compare_non_null(&lhs, &same), Some(Ordering::Equal));
    assert_eq!(
        compare_non_null(&lhs, &higher_component),
        Some(Ordering::Less)
    );
    assert_eq!(compare_non_null(&same, &longer), Some(Ordering::Less));
}

#[test]
fn equal_non_null_list_nan_returns_true() {
    let lhs = Value::List(vec![Value::Float(f64::NAN)]);
    let rhs = Value::List(vec![Value::Float(f64::NAN)]);

    assert!(equal_non_null(&lhs, &rhs));
}

#[test]
fn equal_non_null_record_nan_returns_true() {
    let key = db_string("x").unwrap();
    let lhs = Value::Record(Box::new(Record::Open(smallvec![(
        key.clone(),
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
    let key = db_string("x").unwrap();
    let lhs = Value::Record(Box::new(Record::Open(smallvec![(
        key.clone(),
        Value::Null
    )])));
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
    let key = db_string("x").unwrap();
    let lhs = Value::Record(Box::new(Record::Open(smallvec![(
        key.clone(),
        Value::Null
    )])));
    let rhs = Value::Record(Box::new(Record::Open(smallvec![(key, Value::Int(1))])));

    assert_eq!(compare_non_null(&lhs, &rhs), None);
}

#[test]
fn compare_list_with_null_element_returns_null() {
    // ISO §16.7 element-wise comparison propagates 3VL: a NULL element makes the pair
    // incomparable, so the whole list comparison is unknown (None), never a value_rank
    // tiebreak collapsing to Equal.
    let lhs = Value::List(vec![Value::Null]);
    let rhs = Value::List(vec![Value::Int(1)]);

    assert_eq!(compare_non_null(&lhs, &rhs), None);
}

#[test]
fn compare_list_with_incomparable_element_returns_null() {
    // Cross-type elements are not comparable → None (not a silent ordering).
    let lhs = Value::List(vec![Value::String(db_string("a").unwrap())]);
    let rhs = Value::List(vec![Value::Int(1)]);

    assert_eq!(compare_non_null(&lhs, &rhs), None);
}

#[test]
fn compare_nested_list_orders_by_inner_element() {
    // Recursion through compare_values: the inner lists decide the outer ordering.
    let lhs = Value::List(vec![Value::List(vec![Value::Int(2)])]);
    let rhs = Value::List(vec![Value::List(vec![Value::Int(1)])]);

    assert_eq!(compare_non_null(&lhs, &rhs), Some(Ordering::Greater));
}

#[test]
fn compare_list_equal_prefix_shorter_precedes() {
    // Equal prefix → the shorter list precedes (cardinality tiebreak); the empty list
    // precedes any non-empty list; equal-length-equal-elements compare Equal.
    assert_eq!(
        compare_non_null(
            &Value::List(vec![Value::Int(1)]),
            &Value::List(vec![Value::Int(1), Value::Int(2)]),
        ),
        Some(Ordering::Less)
    );
    assert_eq!(
        compare_non_null(&Value::List(Vec::new()), &Value::List(vec![Value::Int(1)])),
        Some(Ordering::Less)
    );
    assert_eq!(
        compare_non_null(&Value::List(Vec::new()), &Value::List(Vec::new())),
        Some(Ordering::Equal)
    );
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
            &Value::ZonedDateTime(Box::new(
                "2024-01-01T00:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            )),
            &Value::ZonedDateTime(Box::new(
                "2026-01-01T00:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            )),
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
            &Value::ZonedTime(Box::new(
                "2024-01-01T01:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            )),
            &Value::ZonedTime(Box::new(
                "2024-01-01T02:00:00-05:00[America/New_York]"
                    .parse()
                    .unwrap(),
            )),
        ),
        Some(Ordering::Less)
    );
}

#[test]
fn compare_non_null_duration_lt() {
    assert_eq!(
        compare_non_null(
            &Value::Duration(Box::new("PT1S".parse().unwrap())),
            &Value::Duration(Box::new("PT2S".parse().unwrap()))
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
    let string = Value::String(db_string("2024-01-01").unwrap());
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
        Value::ZonedDateTime(Box::new(
            "2024-01-01T00:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
        Value::ZonedDateTime(Box::new(
            "2026-01-01T00:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
    );
    assert_sort_less(
        Value::LocalTime("01:00:00".parse().unwrap()),
        Value::LocalTime("02:00:00".parse().unwrap()),
    );
    assert_sort_less(
        Value::ZonedTime(Box::new(
            "2024-01-01T01:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
        Value::ZonedTime(Box::new(
            "2024-01-01T02:00:00-05:00[America/New_York]"
                .parse()
                .unwrap(),
        )),
    );
    assert_sort_less(
        Value::Duration(Box::new("PT1H".parse().unwrap())),
        Value::Duration(Box::new("PT2H".parse().unwrap())),
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
    assert_sort_less(
        Value::Vector(VectorValue::new(vec![1.0, 2.0]).unwrap()),
        Value::Vector(VectorValue::new(vec![1.0, 3.0]).unwrap()),
    );
}

// --- GQLRT-14: RECORD equality / ordering is field-name-keyed (ISO §4.15) ---

fn open_record(fields: &[(&str, Value)]) -> Value {
    let mut record = smallvec![];
    for (name, value) in fields {
        record.push((db_string(name).unwrap(), value.clone()));
    }
    Value::Record(Box::new(Record::Open(record)))
}

#[test]
fn record_equal_ignores_field_order() {
    // ISO §4.15: records are equal under a field-name bijection, not positional.
    let lhs = open_record(&[("a", Value::Int(1)), ("b", Value::Int(2))]);
    let rhs = open_record(&[("b", Value::Int(2)), ("a", Value::Int(1))]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), Some(true));
    assert!(equal_non_null(&lhs, &rhs));
}

#[test]
fn record_unequal_when_field_sets_differ() {
    let lhs = open_record(&[("a", Value::Int(1))]);
    let rhs = open_record(&[("a", Value::Int(1)), ("b", Value::Int(2))]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), Some(false));
    assert!(!equal_non_null(&lhs, &rhs));
}

#[test]
fn record_unequal_when_field_names_differ_same_arity() {
    let lhs = open_record(&[("a", Value::Int(1))]);
    let rhs = open_record(&[("z", Value::Int(1))]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), Some(false));
}

#[test]
fn record_null_field_yields_unknown_regardless_of_order() {
    let lhs = open_record(&[("a", Value::Int(1)), ("b", Value::Null)]);
    let rhs = open_record(&[("b", Value::Null), ("a", Value::Int(1))]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), None);
}

#[test]
fn record_unequal_short_circuits_before_null_field() {
    // A definite mismatch on a shared field beats a NULL elsewhere → Some(false),
    // not None.
    let lhs = open_record(&[("a", Value::Int(1)), ("b", Value::Null)]);
    let rhs = open_record(&[("a", Value::Int(2)), ("b", Value::Null)]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), Some(false));
}

#[test]
fn nested_record_equal_ignores_inner_field_order() {
    let inner_lhs = open_record(&[("x", Value::Int(1)), ("y", Value::Int(2))]);
    let inner_rhs = open_record(&[("y", Value::Int(2)), ("x", Value::Int(1))]);
    let lhs = open_record(&[("inner", inner_lhs)]);
    let rhs = open_record(&[("inner", inner_rhs)]);

    assert_eq!(gql_equal_non_null(&lhs, &rhs), Some(true));
}

#[test]
fn record_compare_is_field_name_keyed() {
    // Permuted-but-equal records compare Equal; differing field values order by
    // the field-name-sorted view.
    let lhs = open_record(&[("a", Value::Int(1)), ("b", Value::Int(2))]);
    let rhs = open_record(&[("b", Value::Int(2)), ("a", Value::Int(1))]);
    assert_eq!(compare_non_null(&lhs, &rhs), Some(Ordering::Equal));

    let smaller = open_record(&[("a", Value::Int(1)), ("b", Value::Int(2))]);
    let larger = open_record(&[("b", Value::Int(9)), ("a", Value::Int(1))]);
    assert_eq!(compare_non_null(&smaller, &larger), Some(Ordering::Less));
}

// --- GQLRT-26: cross-type numeric equality for Int128 / Uint128 / Decimal ---
//
// Ordering (`compare_non_null`) already handles every numeric cross-type pair;
// equality (`equal_non_null` / `gql_equal_non_null`) must agree so that
// `$int128_one = 1` is TRUE exactly when `<=1 AND >=1` is TRUE. Each `Equal`
// outcome from ordering must coincide with `equal_non_null == true` (parity).

/// Exhaustive numeric cross-product used to pin the equality↔ordering parity.
fn numeric_cross_product() -> Vec<Value> {
    vec![
        Value::Int(1),
        Value::Int(-3),
        Value::Uint(1),
        Value::Uint(5),
        Value::Int128(1),
        Value::Int128(-3),
        Value::Int128(170_141_183_460_469_231_731_687_303_715_884_105_727),
        Value::Uint128(1),
        Value::Uint128(u128::MAX),
        Value::Float(1.0),
        Value::Float(-3.0),
        Value::Float(1.5),
        Value::Float32(1.0),
        Value::Float32(1.5),
        Value::Decimal("1".parse().unwrap()),
        Value::Decimal("-3".parse().unwrap()),
        Value::Decimal("1.5".parse().unwrap()),
        Value::Decimal("1.1".parse().unwrap()),
    ]
}

#[test]
fn numeric_equality_agrees_with_ordering_across_cross_product() {
    let values = numeric_cross_product();
    for lhs in &values {
        for rhs in &values {
            let ordering = compare_non_null(lhs, rhs);
            let equal = equal_non_null(lhs, rhs);
            // Parity: ordering says Equal IFF equality says true.
            match ordering {
                Some(Ordering::Equal) => assert!(
                    equal,
                    "ordering Equal but equality false for {lhs:?} vs {rhs:?}"
                ),
                Some(_) => assert!(
                    !equal,
                    "ordering non-Equal but equality true for {lhs:?} vs {rhs:?}"
                ),
                None => {}
            }
        }
    }
}

#[test]
fn int128_equals_int_when_representable() {
    assert!(equal_non_null(&Value::Int128(1), &Value::Int(1)));
    assert!(equal_non_null(&Value::Int(1), &Value::Int128(1)));
    assert!(!equal_non_null(&Value::Int128(2), &Value::Int(1)));
}

#[test]
fn uint128_equals_uint_and_int() {
    assert!(equal_non_null(&Value::Uint128(1), &Value::Uint(1)));
    assert!(equal_non_null(&Value::Uint128(1), &Value::Int(1)));
    assert!(!equal_non_null(&Value::Uint128(1), &Value::Int(-1)));
}

#[test]
fn int128_equals_uint128_when_value_matches() {
    assert!(equal_non_null(&Value::Int128(7), &Value::Uint128(7)));
    assert!(!equal_non_null(&Value::Int128(-1), &Value::Uint128(1)));
}

#[test]
fn decimal_equals_integer_when_integral() {
    assert!(equal_non_null(
        &Value::Decimal("1".parse().unwrap()),
        &Value::Int(1)
    ));
    assert!(equal_non_null(
        &Value::Int(1),
        &Value::Decimal("1".parse().unwrap())
    ));
    assert!(equal_non_null(
        &Value::Decimal("1".parse().unwrap()),
        &Value::Uint(1)
    ));
    assert!(equal_non_null(
        &Value::Decimal("1".parse().unwrap()),
        &Value::Int128(1)
    ));
    assert!(equal_non_null(
        &Value::Decimal("1".parse().unwrap()),
        &Value::Uint128(1)
    ));
    assert!(!equal_non_null(
        &Value::Decimal("1.5".parse().unwrap()),
        &Value::Int(1)
    ));
    assert!(!equal_non_null(
        &Value::Decimal("1.5".parse().unwrap()),
        &Value::Int(2)
    ));
}

#[test]
fn decimal_equals_float_when_exact() {
    // 1.5 and 0.5 are dyadic — exactly representable in both bases.
    assert!(equal_non_null(
        &Value::Decimal("1.5".parse().unwrap()),
        &Value::Float(1.5)
    ));
    assert!(equal_non_null(
        &Value::Float(1.5),
        &Value::Decimal("1.5".parse().unwrap())
    ));
    assert!(equal_non_null(
        &Value::Decimal("0.5".parse().unwrap()),
        &Value::Float32(0.5)
    ));
    // 0.1_f64 is not exactly 0.1 in decimal, so they are NOT equal.
    assert!(!equal_non_null(
        &Value::Decimal("0.1".parse().unwrap()),
        &Value::Float(0.1)
    ));
}

#[test]
fn decimal_nan_float_equality_is_unknown() {
    // NaN poisons equality (3VL): the gql layer returns None when a NaN operand
    // shares a numeric comparison arm.
    assert_eq!(
        gql_equal_non_null(
            &Value::Decimal("1.5".parse().unwrap()),
            &Value::Float(f64::NAN)
        ),
        None
    );
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
