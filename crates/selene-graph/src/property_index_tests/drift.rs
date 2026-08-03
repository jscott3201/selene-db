//! Drift-tally maintenance for typed property indexes.
//!
//! The tally decides whether the index answers a probe at all, so every path
//! that adds or removes an unkeyable row must move it by exactly one. An
//! over-count disables the index for good; an under-count lets it answer while
//! rows are missing, which is the wrong-answer bug this machinery exists to
//! prevent.

use super::*;

fn drifted(indexes: &PropertyIndexMap, label: &DbString, property: &DbString) -> u64 {
    indexes
        .get(&(label.clone(), property.clone()))
        .expect("index is registered")
        .drifted_rows
}

fn label_set(label: &DbString) -> LabelSet {
    LabelSet::single(label.clone())
}

/// `(label, property, indexes)` with one I64 index registered.
fn i64_index() -> (DbString, DbString, PropertyIndexMap) {
    let label = db_string("pi.drift.label").unwrap();
    let property = db_string("pi.drift.age").unwrap();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert(
        (label.clone(), property.clone()),
        entry(TypedIndexKind::I64),
    );
    (label, property, indexes)
}

fn text(value: &str) -> Value {
    Value::String(db_string(value).unwrap())
}

#[test]
fn kind_mismatch_on_create_counts_and_delete_clears() {
    let (label, property, mut indexes) = i64_index();
    let props = property_map([(property.clone(), text("pi.drift.not-an-int"))]);

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();
    assert_eq!(
        drifted(&indexes, &label, &property),
        1,
        "a live row the index cannot key must be counted"
    );

    apply_node_delete(&mut indexes, &label_set(&label), &props, 0).unwrap();
    assert_eq!(
        drifted(&indexes, &label, &property),
        0,
        "removing the row must return the tally, or the index stays disabled forever"
    );
}

#[test]
fn matching_values_never_count() {
    let (label, property, mut indexes) = i64_index();
    let props = property_map([(property.clone(), Value::Int(30))]);

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();

    assert_eq!(drifted(&indexes, &label, &property), 0);
    assert!(rows(&indexes, label, property, &Value::Int(30)).contains(0));
}

#[test]
fn nan_is_not_drift() {
    let label = db_string("pi.drift.nan.label").unwrap();
    let property = db_string("pi.drift.nan.score").unwrap();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert(
        (label.clone(), property.clone()),
        entry(TypedIndexKind::F64),
    );
    let props = property_map([(property.clone(), Value::Float(f64::NAN))]);

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();

    assert_eq!(
        drifted(&indexes, &label, &property),
        0,
        "NaN satisfies no equality or range predicate, so a scan omits the row too; \
         counting it would disable an index that already agrees with a scan"
    );
}

#[test]
fn update_between_two_drifted_values_nets_zero() {
    let (label, property, mut indexes) = i64_index();
    let before = property_map([(property.clone(), text("pi.drift.first"))]);
    let after = property_map([(property.clone(), text("pi.drift.second"))]);

    apply_node_create(&mut indexes, &label_set(&label), &before, 0).unwrap();
    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &before,
        &label_set(&label),
        &after,
        0,
    )
    .unwrap();

    assert_eq!(
        drifted(&indexes, &label, &property),
        1,
        "one drifted row replaced by another is still one drifted row; a missing \
         decrement leaks +1 here"
    );
}

#[test]
fn update_from_drifted_to_matching_clears_the_tally() {
    let (label, property, mut indexes) = i64_index();
    let before = property_map([(property.clone(), text("pi.drift.was-text"))]);
    let after = property_map([(property.clone(), Value::Int(41))]);

    apply_node_create(&mut indexes, &label_set(&label), &before, 0).unwrap();
    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &before,
        &label_set(&label),
        &after,
        0,
    )
    .unwrap();

    assert_eq!(drifted(&indexes, &label, &property), 0);
    assert!(
        rows(&indexes, label, property, &Value::Int(41)).contains(0),
        "the repaired row must now be indexed"
    );
}

#[test]
fn update_from_matching_to_drifted_counts() {
    let (label, property, mut indexes) = i64_index();
    let before = property_map([(property.clone(), Value::Int(41))]);
    let after = property_map([(property.clone(), text("pi.drift.now-text"))]);

    apply_node_create(&mut indexes, &label_set(&label), &before, 0).unwrap();
    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &before,
        &label_set(&label),
        &after,
        0,
    )
    .unwrap();

    assert_eq!(drifted(&indexes, &label, &property), 1);
}

#[test]
fn drifted_value_replaced_by_null_clears_the_tally() {
    let (label, property, mut indexes) = i64_index();
    let before = property_map([(property.clone(), text("pi.drift.text"))]);
    let after = property_map([(property.clone(), Value::Null)]);

    apply_node_create(&mut indexes, &label_set(&label), &before, 0).unwrap();
    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &before,
        &label_set(&label),
        &after,
        0,
    )
    .unwrap();

    assert_eq!(
        drifted(&indexes, &label, &property),
        0,
        "a NULL is not indexable and is not drift"
    );
}

#[test]
fn many_drifted_rows_accumulate_and_drain() {
    let (label, property, mut indexes) = i64_index();
    let props: Vec<_> = (0..5)
        .map(|row| property_map([(property.clone(), text(&format!("pi.drift.row{row}")))]))
        .collect();

    for (row, props) in props.iter().enumerate() {
        apply_node_create(
            &mut indexes,
            &label_set(&label),
            props,
            u32::try_from(row).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(drifted(&indexes, &label, &property), 5);

    for (row, props) in props.iter().enumerate() {
        apply_node_delete(
            &mut indexes,
            &label_set(&label),
            props,
            u32::try_from(row).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        drifted(&indexes, &label, &property),
        0,
        "the tally must return to zero so the index can answer again"
    );
}

#[test]
fn probe_declines_while_drifted_and_recovers_after_repair() {
    let (label, property, mut indexes) = i64_index();
    let matching = property_map([(property.clone(), Value::Int(30))]);
    let drifting = property_map([(property.clone(), text("pi.drift.blocker"))]);

    apply_node_create(&mut indexes, &label_set(&label), &matching, 0).unwrap();
    let key = (label.clone(), property.clone());
    assert!(
        indexes
            .get(&key)
            .unwrap()
            .lookup_eq(&Value::Int(30))
            .is_some(),
        "a complete index answers"
    );

    apply_node_create(&mut indexes, &label_set(&label), &drifting, 1).unwrap();
    assert!(
        indexes
            .get(&key)
            .unwrap()
            .lookup_eq(&Value::Int(30))
            .is_none(),
        "one unkeyable row must make every probe decline, so callers scan and see \
         the same rows an index-free graph would return"
    );
    assert!(
        indexes
            .get(&key)
            .unwrap()
            .lookup_eq_ignoring_drift(&Value::Int(30))
            .is_some_and(|bitmap| bitmap.contains(0)),
        "the auditing probe still reports what the index actually holds"
    );

    apply_node_delete(&mut indexes, &label_set(&label), &drifting, 1).unwrap();
    assert!(
        indexes
            .get(&key)
            .unwrap()
            .lookup_eq(&Value::Int(30))
            .is_some(),
        "removing the drifted row must re-enable the index"
    );
}

#[test]
fn edge_indexes_share_the_tally() {
    let label = db_string("pi.drift.edge.label").unwrap();
    let property = db_string("pi.drift.edge.weight").unwrap();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert(
        (label.clone(), property.clone()),
        entry(TypedIndexKind::I64),
    );
    let props = property_map([(property.clone(), text("pi.drift.edge.text"))]);

    apply_edge_create(&mut indexes, &label, &props, 0).unwrap();
    assert_eq!(
        drifted(&indexes, &label, &property),
        1,
        "edge indexes reuse the same commit funnel and must tally the same way"
    );

    apply_edge_delete(&mut indexes, &label, &props, 0).unwrap();
    assert_eq!(drifted(&indexes, &label, &property), 0);
}
