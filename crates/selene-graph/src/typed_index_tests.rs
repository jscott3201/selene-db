use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use jiff::civil::{date, datetime};
use proptest::prelude::*;
use roaring::RoaringBitmap;
use selene_core::{Value, db_string};

use super::*;

fn row_index(index: &TypedIndex, value: &Value) -> RoaringBitmap {
    index.lookup_eq(value).expect("kind matches").into_owned()
}

#[test]
fn kind_round_trips_for_each_variant() {
    for kind in [
        TypedIndexKind::Bool,
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
        TypedIndexKind::Uuid,
    ] {
        assert_eq!(TypedIndex::new(kind).kind(), kind);
    }
}

#[test]
fn kind_rkyv_round_trips_for_each_variant() {
    for kind in [
        TypedIndexKind::Bool,
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
        TypedIndexKind::Uuid,
    ] {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&kind).unwrap();
        let round: TypedIndexKind =
            rkyv::from_bytes::<TypedIndexKind, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(round, kind);
    }
}

#[test]
fn not_nan_rejects_nan() {
    assert_eq!(NotNanF64::new(f64::NAN), Err(NotNanError));
}

#[test]
fn not_nan_preserves_zero_sign_as_distinct_keys() {
    assert_ne!(NotNanF64::new(0.0).unwrap(), NotNanF64::new(-0.0).unwrap());
}

#[test]
fn not_nan_total_order_matches_total_cmp() {
    let values = [f64::NEG_INFINITY, -1.0, -0.0, 0.0, 1.0, f64::INFINITY]
        .map(|value| NotNanF64::new(value).unwrap());
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn not_nan_hash_agrees_with_eq() {
    let lhs = NotNanF64::new(42.0).unwrap();
    let rhs = NotNanF64::new(42.0).unwrap();
    let mut lhs_hasher = DefaultHasher::new();
    let mut rhs_hasher = DefaultHasher::new();
    lhs.hash(&mut lhs_hasher);
    rhs.hash(&mut rhs_hasher);
    assert_eq!(lhs, rhs);
    assert_eq!(lhs_hasher.finish(), rhs_hasher.finish());
}

#[test]
fn insert_remove_round_trips_for_each_kind() {
    let string = db_string("typed-index.string").unwrap();
    let uuid = uuid::Uuid::from_u128(7);
    let cases = [
        (TypedIndexKind::Bool, Value::Bool(true)),
        (TypedIndexKind::I64, Value::Int(7)),
        (TypedIndexKind::F64, Value::Float(7.0)),
        (TypedIndexKind::String, Value::String(string)),
        (TypedIndexKind::Date, Value::Date(date(2026, 5, 7))),
        (
            TypedIndexKind::LocalDateTime,
            Value::LocalDateTime(datetime(2026, 5, 7, 12, 30, 0, 0)),
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
    // Removing the last row on key 1 prunes the bucket → distinct drops.
    index.remove(&Value::Int(1), 1).unwrap();
    assert_eq!(index.cardinality(), 1);
    assert_eq!(
        index.distinct_keys(),
        1,
        "empty bucket pruned → distinct = 1"
    );
}

#[test]
fn distinct_keys_each_kind() {
    // String, Date, Uuid kinds: distinct_keys == number of distinct inserts.
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
fn prefix_scan_matches_string_keys_only() {
    let alpha = db_string("typed-index.prefix.alpha").unwrap();
    let beta = db_string("typed-index.beta").unwrap();
    let mut index = TypedIndex::new(TypedIndexKind::String);
    index.insert(&Value::String(alpha), 0).unwrap();
    index.insert(&Value::String(beta), 1).unwrap();
    let result = index.lookup_prefix("typed-index.prefix").unwrap();
    assert!(result.contains(0));
    assert!(!result.contains(1));
    assert!(
        TypedIndex::new(TypedIndexKind::I64)
            .lookup_prefix("typed")
            .is_none()
    );
}

#[test]
fn typed_key_string_returns_string_key() {
    let value = Value::String(db_string("typed_key_admit.string.unique-1").unwrap());

    let key = typed_key(&value, TypedIndexKind::String).expect("string coerces");

    let TypedKey::String(db_string) = key else {
        panic!("expected TypedKey::String, got {key:?}");
    };
    assert_eq!(db_string.as_str(), "typed_key_admit.string.unique-1");
}

#[test]
fn typed_key_unindexable_value_rejects_kind_mismatch() {
    // A value whose variant has no typed-key coercion (e.g. NULL) fails
    // KindMismatch for every index kind.
    let value = Value::Null;

    for kind in [
        TypedIndexKind::Bool,
        TypedIndexKind::I64,
        TypedIndexKind::F64,
        TypedIndexKind::String,
        TypedIndexKind::Date,
        TypedIndexKind::LocalDateTime,
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

#[test]
fn string_value_rejected_by_non_string_index() {
    // Inserting a `Value::String` into an I64-kind index is rejected by the
    // outer `(self, key)` kind check, leaving the index empty.
    let mut index = TypedIndex::new(TypedIndexKind::I64);
    let err = index
        .insert(
            &Value::String(db_string("typed_key_admit.kind_mismatch.unique").unwrap()),
            0,
        )
        .expect_err("cross-kind insert rejects");
    assert!(matches!(
        err,
        TypedIndexValueError::KindMismatch {
            observed: "String",
            ..
        }
    ));
    assert_eq!(index.cardinality(), 0);
}

#[test]
fn lookup_eq_string_finds_admitted_row() {
    let mut index = TypedIndex::new(TypedIndexKind::String);
    let content = "lookup_eq.string.cross-variant";
    let db_string = db_string(content).unwrap();
    index.insert(&Value::String(db_string.clone()), 3).unwrap();

    let result = index
        .lookup_eq(&Value::String(db_string))
        .expect("kind matches");

    assert!(result.contains(3));
}

#[test]
fn lookup_eq_returns_none_for_kind_drift_under_open_graph() {
    // An open-graph index can have a probe value whose kind does not match
    // its declared kind. A `Value::String` probe against a non-STRING index
    // must return `None` from `lookup_eq` so the caller drops to a linear
    // scan + cross-variant `value_compare`.
    let index = TypedIndex::new(TypedIndexKind::I64);

    let result = index.lookup_eq(&Value::String(
        db_string("lookup_eq.kind_drift.unique").unwrap(),
    ));

    assert!(
        result.is_none(),
        "non-STRING-kind index with String probe must return None (scan fallback)"
    );
}

#[test]
fn values_share_key_falls_through_for_distinct_strings() {
    let index = TypedIndex::new(TypedIndexKind::String);
    let lhs = Value::String(db_string("values_share_key.string.lhs-unique").unwrap());
    let rhs = Value::String(db_string("values_share_key.string.rhs-unique").unwrap());

    assert!(!index.values_share_key(&lhs, &rhs));
}

#[test]
fn values_share_key_returns_true_for_same_string_content() {
    let index = TypedIndex::new(TypedIndexKind::String);
    let content = "values_share_key.string.same-content-unique";
    let lhs = Value::String(db_string(content).unwrap());
    let rhs = Value::String(db_string(content).unwrap());

    assert!(index.values_share_key(&lhs, &rhs));
}

#[test]
fn string_range_returns_matched_rows_over_lexicographic_keys() {
    // Post-collapse: `DbString` Ord is lexicographic, so the String arm of
    // `lookup_range` walks the BTreeMap range directly instead of refusing
    // with `None`. Half-open `[alpha, charlie)` includes "alpha" and "bravo"
    // and excludes the exclusive end "charlie".
    let alpha = db_string("typed-index.range.alpha").unwrap();
    let bravo = db_string("typed-index.range.bravo").unwrap();
    let charlie = db_string("typed-index.range.charlie").unwrap();
    let mut index = TypedIndex::new(TypedIndexKind::String);
    index.insert(&Value::String(alpha.clone()), 0).unwrap();
    index.insert(&Value::String(bravo.clone()), 1).unwrap();
    index.insert(&Value::String(charlie.clone()), 2).unwrap();

    let rows = index
        .lookup_range(Value::String(alpha)..Value::String(charlie))
        .expect("String ranges now resolve via lexicographic BTreeMap order");

    assert!(rows.contains(0), "alpha (inclusive low) matches");
    assert!(rows.contains(1), "bravo (interior) matches");
    assert!(!rows.contains(2), "charlie (exclusive high) excluded");
    assert_eq!(rows.len(), 2);
}

#[test]
fn string_range_inclusive_includes_high_endpoint() {
    let alpha = db_string("typed-index.range.incl.alpha").unwrap();
    let charlie = db_string("typed-index.range.incl.charlie").unwrap();
    let mut index = TypedIndex::new(TypedIndexKind::String);
    index.insert(&Value::String(alpha.clone()), 0).unwrap();
    index.insert(&Value::String(charlie.clone()), 1).unwrap();

    let rows = index
        .lookup_range(Value::String(alpha)..=Value::String(charlie))
        .expect("inclusive String range resolves");

    assert!(rows.contains(0));
    assert!(rows.contains(1), "inclusive high endpoint matches");
    assert_eq!(rows.len(), 2);
}

/// Reference implementation: the pre-collapse O(n) full-scan `starts_with`
/// filter that `lookup_prefix` replaced with a `BTreeMap::range` seek. The
/// proptest below asserts the two are result-identical.
fn lookup_prefix_full_scan_oracle(keys: &[(DbString, u32)], prefix: &str) -> RoaringBitmap {
    let mut result = RoaringBitmap::new();
    for (key, row) in keys {
        if key.as_str().starts_with(prefix) {
            result.insert(*row);
        }
    }
    result
}

#[test]
fn lookup_prefix_handles_empty_and_high_byte_edges() {
    // Deterministic coverage of the brief's critical edges: the empty prefix
    // (no finite successor → matches every key) and high-code-point keys whose
    // UTF-8 trails in 0xBF/0xFF-range bytes (the prefix span must not silently
    // drop the tail of matching keys).
    let keys = [
        db_string("").unwrap(),
        db_string("a").unwrap(),
        db_string("a\u{FF}").unwrap(),     // ends in 0xC3 0xBF
        db_string("a\u{FFFF}").unwrap(),   // ends in 0xEF 0xBF 0xBF
        db_string("a\u{10FFFF}").unwrap(), // max code point, ends in 0xF4 0x8F 0xBF 0xBF
        db_string("ab").unwrap(),
        db_string("b").unwrap(),
    ];
    let mut index = TypedIndex::new(TypedIndexKind::String);
    let mut pairs: Vec<(DbString, u32)> = Vec::new();
    for (row, key) in keys.iter().enumerate() {
        let row = row as u32;
        index.insert(&Value::String(key.clone()), row).unwrap();
        pairs.push((key.clone(), row));
    }

    for prefix in [
        "",
        "a",
        "a\u{FF}",
        "a\u{10FFFF}",
        "ab",
        "b",
        "z",
        "\u{10FFFF}",
    ] {
        let observed = index.lookup_prefix(prefix).expect("string index");
        let expected = lookup_prefix_full_scan_oracle(&pairs, prefix);
        assert_eq!(
            observed, expected,
            "prefix {prefix:?} range-seek must equal full-scan oracle"
        );
    }
}

proptest! {
    /// `lookup_prefix` (BTreeMap range seek) must equal the old full-scan
    /// `starts_with` filter for arbitrary key sets and prefixes, including
    /// high-code-point (0xFF-range UTF-8 trailing byte) keys, the empty prefix,
    /// and an all-high-code-point prefix.
    #[test]
    fn lookup_prefix_range_equals_full_scan(
        // Keys drawn from a small alphabet that includes high code points so
        // the prefix-span upper edge is exercised.
        raw_keys in proptest::collection::vec(
            proptest::collection::vec(
                proptest::prop_oneof![
                    Just('a'), Just('b'), Just('c'),
                    Just('\u{FF}'), Just('\u{FFFF}'), Just('\u{10FFFF}'),
                ],
                0..4usize,
            ),
            1..16usize,
        ),
        prefix_chars in proptest::collection::vec(
            proptest::prop_oneof![
                Just('a'), Just('b'),
                Just('\u{FF}'), Just('\u{10FFFF}'),
            ],
            0..3usize,
        ),
    ) {
        // Dedup keys (an index has one bucket per distinct key) while assigning
        // each distinct key a stable row.
        let mut seen = std::collections::BTreeMap::<String, u32>::new();
        let mut pairs: Vec<(DbString, u32)> = Vec::new();
        let mut index = TypedIndex::new(TypedIndexKind::String);
        let mut next_row = 0u32;
        for chars in &raw_keys {
            let s: String = chars.iter().collect();
            if seen.contains_key(&s) {
                continue;
            }
            let row = next_row;
            next_row += 1;
            seen.insert(s.clone(), row);
            let key = db_string(&s).unwrap();
            index.insert(&Value::String(key.clone()), row).unwrap();
            pairs.push((key, row));
        }

        let prefix: String = prefix_chars.iter().collect();
        let observed = index.lookup_prefix(&prefix).expect("string index");
        let expected = lookup_prefix_full_scan_oracle(&pairs, &prefix);
        prop_assert_eq!(observed, expected);
    }
}
