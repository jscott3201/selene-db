use jiff::civil::{date, datetime, time};
use selene_core::db_string;

use super::*;

#[test]
fn insert_remove_round_trips_for_each_kind() {
    let string = db_string("typed-index.string").unwrap();
    let uuid = uuid::Uuid::from_u128(7);
    let cases = [
        (TypedIndexKind::Bool, Value::Bool(true)),
        (TypedIndexKind::I64, Value::Int(7)),
        (TypedIndexKind::U64, Value::Uint(7)),
        (TypedIndexKind::I128, Value::Int128(i128::MIN + 7)),
        (TypedIndexKind::U128, Value::Uint128(u128::MAX - 7)),
        (TypedIndexKind::Decimal, Value::Decimal(decimal("7.50"))),
        (TypedIndexKind::F32, Value::Float32(7.0_f32)),
        (TypedIndexKind::F64, Value::Float(7.0)),
        (TypedIndexKind::String, Value::String(string)),
        (TypedIndexKind::Date, Value::Date(date(2026, 5, 7))),
        (
            TypedIndexKind::LocalDateTime,
            Value::LocalDateTime(datetime(2026, 5, 7, 12, 30, 0, 0)),
        ),
        (
            TypedIndexKind::ZonedDateTime,
            Value::ZonedDateTime(Box::new(
                "2026-05-07T12:30:00-04[America/New_York]".parse().unwrap(),
            )),
        ),
        (
            TypedIndexKind::LocalTime,
            Value::LocalTime(time(12, 30, 0, 0)),
        ),
        (
            TypedIndexKind::ZonedTime,
            Value::ZonedTime(Box::new(
                "2026-05-07T12:30:00-04[America/New_York]".parse().unwrap(),
            )),
        ),
        (
            TypedIndexKind::Duration,
            Value::Duration(Box::new("PT1H2S".parse().unwrap())),
        ),
        (TypedIndexKind::Uuid, Value::Uuid(uuid)),
    ];
    for (kind, value) in cases {
        let mut index = TypedIndex::new(kind);
        index.insert(&value, 3).unwrap();
        assert!(row_index(&index, &value).contains(3));
        index.remove(&value, 3).unwrap();
        assert!(row_index(&index, &value).is_empty());
        assert_eq!(index.cardinality(), 0);
    }
}

#[test]
fn lookup_eq_returns_cow_variants_for_hit_and_empty_match() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    index.insert(&Value::Int(7), 3).unwrap();

    let hit = index.lookup_eq(&Value::Int(7)).expect("kind matches");
    assert!(matches!(hit, std::borrow::Cow::Borrowed(_)));
    assert!(hit.contains(3));

    let missing = index.lookup_eq(&Value::Int(8)).expect("kind matches");
    assert!(matches!(missing, std::borrow::Cow::Owned(_)));
    assert!(missing.is_empty());

    assert!(
        index
            .lookup_eq(&Value::String(db_string("wrong").unwrap()))
            .is_none()
    );
}

#[test]
fn insert_errors_on_kind_mismatch() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    let err = index.insert(&Value::String(db_string("wrong-kind").unwrap()), 0);
    assert!(matches!(
        err,
        Err(TypedIndexValueError::KindMismatch {
            expected_kind: TypedIndexKind::I64,
            observed: "String"
        })
    ));
}

#[test]
fn float_insert_errors_on_nan() {
    let mut index = TypedIndex::new(TypedIndexKind::F32);
    let err = index.insert(&Value::Float32(f32::NAN), 0);
    assert!(matches!(
        err,
        Err(TypedIndexValueError::NaN {
            expected_kind: TypedIndexKind::F32
        })
    ));

    let mut index = TypedIndex::new(TypedIndexKind::F64);
    let err = index.insert(&Value::Float(f64::NAN), 0);
    assert!(matches!(
        err,
        Err(TypedIndexValueError::NaN {
            expected_kind: TypedIndexKind::F64
        })
    ));
}

#[test]
fn cardinality_sums_across_keys_and_prunes_empty_keys() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    index.insert(&Value::Int(1), 0).unwrap();
    index.insert(&Value::Int(1), 1).unwrap();
    index.insert(&Value::Int(2), 2).unwrap();
    assert_eq!(index.cardinality(), 3);
    index.remove(&Value::Int(1), 0).unwrap();
    index.remove(&Value::Int(1), 1).unwrap();
    assert_eq!(index.cardinality(), 1);
    assert!(matches!(&index, TypedIndex::I64(inner) if !inner.contains_key(&1)));
}

#[test]
fn distinct_keys_counts_buckets_not_rows() {
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    assert_eq!(
        index.distinct_keys(),
        0,
        "empty index has zero distinct keys"
    );
    // Two rows on key 1, one row on key 2: 3 total rows, 2 distinct keys.
    index.insert(&Value::Int(1), 0).unwrap();
    index.insert(&Value::Int(1), 1).unwrap();
    index.insert(&Value::Int(2), 2).unwrap();
    assert_eq!(index.cardinality(), 3, "total rows = 3");
    assert_eq!(
        index.distinct_keys(),
        2,
        "distinct keys = 2 (dup value bucket)"
    );
    // Removing one of the two rows on key 1 keeps the bucket alive.
    index.remove(&Value::Int(1), 0).unwrap();
    assert_eq!(index.cardinality(), 2);
    assert_eq!(
        index.distinct_keys(),
        2,
        "bucket still present after partial remove"
    );
    // Removing the last row on key 1 prunes the bucket, so distinct drops.
    index.remove(&Value::Int(1), 1).unwrap();
    assert_eq!(index.cardinality(), 1);
    assert_eq!(
        index.distinct_keys(),
        1,
        "empty bucket pruned, so distinct = 1"
    );
}

#[test]
fn distinct_keys_each_kind() {
    // String, Date, temporal, Uuid kinds: distinct_keys == number of distinct inserts.
    let mut s = TypedIndex::new(TypedIndexKind::String);
    s.insert(&Value::String(db_string("a").unwrap()), 0)
        .unwrap();
    s.insert(&Value::String(db_string("b").unwrap()), 1)
        .unwrap();
    s.insert(&Value::String(db_string("a").unwrap()), 2)
        .unwrap();
    assert_eq!(s.distinct_keys(), 2);
    assert_eq!(s.cardinality(), 3);

    let mut d = TypedIndex::new(TypedIndexKind::Date);
    d.insert(&Value::Date(date(2020, 1, 1)), 0).unwrap();
    d.insert(&Value::Date(date(2020, 1, 2)), 1).unwrap();
    assert_eq!(d.distinct_keys(), 2);

    let mut lt = TypedIndex::new(TypedIndexKind::LocalTime);
    lt.insert(&Value::LocalTime(time(1, 0, 0, 0)), 0).unwrap();
    lt.insert(&Value::LocalTime(time(2, 0, 0, 0)), 1).unwrap();
    assert_eq!(lt.distinct_keys(), 2);

    let mut dur = TypedIndex::new(TypedIndexKind::Duration);
    dur.insert(&Value::Duration(Box::new("PT1H".parse().unwrap())), 0)
        .unwrap();
    dur.insert(&Value::Duration(Box::new("PT2H".parse().unwrap())), 1)
        .unwrap();
    assert_eq!(dur.distinct_keys(), 2);
}

#[test]
fn typed_key_unindexable_value_rejects_kind_mismatch() {
    // A value whose variant has no typed-key coercion (e.g. NULL) fails
    // KindMismatch for every index kind.
    let value = Value::Null;

    for kind in [
        TypedIndexKind::Bool,
        TypedIndexKind::I64,
        TypedIndexKind::U64,
        TypedIndexKind::I128,
        TypedIndexKind::U128,
        TypedIndexKind::Decimal,
        TypedIndexKind::F32,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
        TypedIndexKind::ZonedDateTime,
        TypedIndexKind::LocalTime,
        TypedIndexKind::ZonedTime,
        TypedIndexKind::Duration,
        TypedIndexKind::Uuid,
    ] {
        let err = typed_key(&value, kind).expect_err("unindexable value rejects");
        assert!(matches!(
            err,
            TypedIndexValueError::KindMismatch {
                observed: "Null",
                ..
            }
        ));
    }
}
