use proptest::prelude::*;
use selene_core::{DbString, db_string};

use super::*;

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
    // (no finite successor, so matches every key) and high-code-point keys
    // whose UTF-8 trails in 0xBF/0xFF-range bytes (the prefix span must not
    // silently drop the tail of matching keys).
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
