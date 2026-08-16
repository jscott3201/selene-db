//! Exhaustive pins for the ISO comparability *families* implemented by
//! [`super`].
//!
//! ISO/IEC 39075:2024 §4.16.5.2 says "any two numbers are essentially
//! comparable values". Every other family is narrower — §4.16.6.2 restricts
//! temporal instants to "the same most specific static value types", §4.16.6.3
//! restricts durations to one unit group, and §4.4.2 NOTE 25 leaves only
//! identical values otherwise. Feature GA04 "Universal comparison" would widen
//! that, and §4.4.2 NOTE 26 spells out the consequence of not claiming it: "no
//! two values are universally comparable values". `selene-core`'s feature
//! register does not claim GA04.
//!
//! The engine-wide consequence is a single sentence: **the numeric types are
//! the only family whose values compare across distinct `Value` variants.**
//! Several places rely on it — `selene_core::Value::is_number`, and the
//! index-drift classifiers in `selene-graph` that decide whether omitting an
//! unkeyable row can change an answer. Those places cannot re-derive it; they
//! can only assume it. So it is pinned here, over the whole variant census
//! rather than a hand-picked sample, and in both directions: no non-numeric
//! cross-variant pair compares, and every numeric cross-variant pair does.

use selene_core::Value;

use super::{compare_non_null, equal_non_null, gql_equal_non_null};

/// Every ordered pair of *distinct* variants from the census, minus NULL.
///
/// NULL is excluded because the functions under test are the non-null halves
/// (each `debug_assert!`s a non-null argument); the 3VL NULL propagation around
/// them is covered in the sibling suite.
///
/// Drawing from [`Value::ALL`] rather than a literal list is the point: adding
/// a variant without extending this sweep is not possible.
fn cross_variant_pairs() -> Vec<(Value, Value)> {
    let values: Vec<Value> = Value::ALL
        .iter()
        .map(|make| make())
        .filter(|value| !matches!(value, Value::Null))
        .collect();
    let mut pairs = Vec::new();
    for (lhs_index, lhs) in values.iter().enumerate() {
        for (rhs_index, rhs) in values.iter().enumerate() {
            if lhs_index != rhs_index {
                pairs.push((lhs.clone(), rhs.clone()));
            }
        }
    }
    pairs
}

fn describe(lhs: &Value, rhs: &Value) -> String {
    format!("{} vs {}", lhs.variant_name(), rhs.variant_name())
}

#[test]
fn the_sweep_covers_every_variant_pair() {
    // Guards the sweep itself: a filter or census change that silently shrank
    // the pair set would make every assertion below vacuous.
    let census = Value::VARIANT_COUNT - 1; // NULL excluded
    assert_eq!(cross_variant_pairs().len(), census * (census - 1));
}

#[test]
fn cross_variant_equality_collapses_only_within_the_numeric_family() {
    for (lhs, rhs) in cross_variant_pairs() {
        if equal_non_null(&lhs, &rhs) {
            assert!(
                lhs.is_number() && rhs.is_number(),
                "{} compared equal across variants, which ISO §4.16.5.2 permits \
                 only for numbers",
                describe(&lhs, &rhs),
            );
        }
    }
}

#[test]
fn cross_variant_gql_equality_collapses_only_within_the_numeric_family() {
    // The 3VL operator path reaches `equal_non_null` through its own record and
    // NaN pre-checks, so it is pinned separately rather than assumed to agree.
    for (lhs, rhs) in cross_variant_pairs() {
        if gql_equal_non_null(&lhs, &rhs) == Some(true) {
            assert!(
                lhs.is_number() && rhs.is_number(),
                "{} compared equal across variants under the `=` operator",
                describe(&lhs, &rhs),
            );
        }
    }
}

#[test]
fn cross_variant_ordering_is_defined_only_within_the_numeric_family() {
    // Ordering matters as much as equality: a range predicate over a
    // cross-variant pair must stay incomparable, which `eval_ordering` turns
    // into a `ValuesNotComparable` data exception rather than a silent false.
    for (lhs, rhs) in cross_variant_pairs() {
        if compare_non_null(&lhs, &rhs).is_some() {
            assert!(
                lhs.is_number() && rhs.is_number(),
                "{} is order-comparable across variants, which ISO §4.16.6.2 / \
                 §4.4.2 NOTE 26 do not permit without Feature GA04",
                describe(&lhs, &rhs),
            );
        }
    }
}

#[test]
fn every_numeric_cross_variant_pair_actually_collapses() {
    // The converse direction. Without it the three sweeps above would pass just
    // as well against a comparison layer that returned `false` / `None` for
    // everything, and `is_number` could name a family wider than the one that
    // really collapses — which is the direction that makes an index-drift
    // classifier under-count.
    let numeric: Vec<Value> = Value::ALL
        .iter()
        .map(|make| make())
        .filter(Value::is_number)
        .collect();
    assert_eq!(
        numeric.len(),
        7,
        "Int, Uint, Int128, Uint128, Float, Float32, Decimal"
    );

    for lhs in &numeric {
        for rhs in &numeric {
            // Every census fixture for a numeric variant is that variant's
            // zero, so all of them are the same number.
            assert!(
                equal_non_null(lhs, rhs),
                "{} did not collapse, so the numeric family is not closed",
                describe(lhs, rhs),
            );
            assert_eq!(
                compare_non_null(lhs, rhs),
                Some(std::cmp::Ordering::Equal),
                "{} is not order-comparable",
                describe(lhs, rhs),
            );
        }
    }
}
