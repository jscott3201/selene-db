//! Drift-tally maintenance for composite property indexes.
//!
//! The composite path has its own tally arithmetic and its own NaN
//! classification, so the single-key suite proves nothing about it. The update
//! path is the sharp edge: a composite tuple is skipped wholesale when neither
//! side can be keyed, and that skip must not strand the tally when the two
//! sides classify differently.

use super::*;

fn label_set(label: &DbString) -> LabelSet {
    LabelSet::single(label.clone())
}

fn drifted(indexes: &CompositeIndexMap, label: &DbString, properties: &[DbString]) -> u64 {
    indexes
        .get(&(label.clone(), composite_property_key(properties)))
        .expect("composite index is registered")
        .drifted_rows
}

/// `(label, [a, b], indexes)` with one `[F64, I64]` composite index.
fn float_int_index() -> (DbString, SmallVec<[DbString; 4]>, CompositeIndexMap) {
    let label = db_string("cpi.drift.label").unwrap();
    let a = db_string("cpi.drift.a").unwrap();
    let b = db_string("cpi.drift.b").unwrap();
    let properties: SmallVec<[DbString; 4]> = smallvec![a, b];
    let mut indexes = CompositeIndexMap::default();
    insert_entry(
        &mut indexes,
        label.clone(),
        properties.clone(),
        smallvec![TypedIndexKind::F64, TypedIndexKind::I64],
    );
    (label, properties, indexes)
}

fn props_for(properties: &[DbString], values: [Value; 2]) -> PropertyMap {
    let [first, second] = values;
    property_map([
        (properties[0].clone(), first),
        (properties[1].clone(), second),
    ])
}

#[test]
fn kind_mismatch_counts_and_delete_clears() {
    let (label, properties, mut indexes) = float_int_index();
    let props = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.text").unwrap()),
            Value::Int(1),
        ],
    );

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();
    assert_eq!(drifted(&indexes, &label, &properties), 1);

    apply_node_delete(&mut indexes, &label_set(&label), &props, 0).unwrap();
    assert_eq!(drifted(&indexes, &label, &properties), 0);
}

#[test]
fn keyable_tuple_never_counts() {
    let (label, properties, mut indexes) = float_int_index();
    let props = props_for(&properties, [Value::Float(1.5), Value::Int(1)]);

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();

    assert_eq!(drifted(&indexes, &label, &properties), 0);
}

#[test]
fn nan_component_is_not_drift() {
    let (label, properties, mut indexes) = float_int_index();
    let props = props_for(&properties, [Value::Float(f64::NAN), Value::Int(1)]);

    apply_node_create(&mut indexes, &label_set(&label), &props, 0).unwrap();

    assert_eq!(
        drifted(&indexes, &label, &properties),
        0,
        "a NaN component matches no predicate, so a scan omits the row too"
    );
}

/// The defect an adversarial review caught: a tuple moving between a NaN
/// component and a kind-mismatched one is skipped wholesale by the update
/// short-circuit, because neither side can be keyed. Both sides are absent from
/// the index either way, but only one of them counts as drift, so skipping
/// strands the tally and the index answers while still missing the row.
#[test]
fn nan_to_kind_mismatch_still_moves_the_tally() {
    let (label, properties, mut indexes) = float_int_index();
    let nan = props_for(&properties, [Value::Float(f64::NAN), Value::Int(1)]);
    let mismatched = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.text").unwrap()),
            Value::Int(1),
        ],
    );

    apply_node_create(&mut indexes, &label_set(&label), &nan, 0).unwrap();
    assert_eq!(drifted(&indexes, &label, &properties), 0);

    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &nan,
        &label_set(&label),
        &mismatched,
        0,
    )
    .unwrap();

    assert_eq!(
        drifted(&indexes, &label, &properties),
        1,
        "the row is now unkeyable for a reason a scan WOULD match, so it counts"
    );
}

#[test]
fn kind_mismatch_to_nan_still_moves_the_tally() {
    let (label, properties, mut indexes) = float_int_index();
    let mismatched = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.text").unwrap()),
            Value::Int(1),
        ],
    );
    let nan = props_for(&properties, [Value::Float(f64::NAN), Value::Int(1)]);

    apply_node_create(&mut indexes, &label_set(&label), &mismatched, 0).unwrap();
    assert_eq!(drifted(&indexes, &label, &properties), 1);

    apply_node_update(
        &mut indexes,
        &label_set(&label),
        &mismatched,
        &label_set(&label),
        &nan,
        0,
    )
    .unwrap();

    assert_eq!(
        drifted(&indexes, &label, &properties),
        0,
        "the reverse transition must release the tally, or the index stays \
         disabled on a row a scan would omit anyway"
    );
}

#[test]
fn update_between_two_mismatched_tuples_nets_zero() {
    let (label, properties, mut indexes) = float_int_index();
    let before = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.one").unwrap()),
            Value::Int(1),
        ],
    );
    let after = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.two").unwrap()),
            Value::Int(2),
        ],
    );

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

    assert_eq!(drifted(&indexes, &label, &properties), 1);
}

#[test]
fn repairing_the_tuple_clears_the_tally_and_indexes_the_row() {
    let (label, properties, mut indexes) = float_int_index();
    let before = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.text").unwrap()),
            Value::Int(1),
        ],
    );
    let after = props_for(&properties, [Value::Float(2.5), Value::Int(1)]);

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

    assert_eq!(drifted(&indexes, &label, &properties), 0);
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Float(2.5), Value::Int(1)]
        )
        .contains(0)
    );
}

#[test]
fn probe_declines_while_drifted() {
    let (label, properties, mut indexes) = float_int_index();
    let clean = props_for(&properties, [Value::Float(1.0), Value::Int(1)]);
    let blocker = props_for(
        &properties,
        [
            Value::String(db_string("cpi.drift.blocker").unwrap()),
            Value::Int(2),
        ],
    );
    let key = (label.clone(), composite_property_key(&properties));

    apply_node_create(&mut indexes, &label_set(&label), &clean, 0).unwrap();
    assert!(indexes.get(&key).unwrap().probe_arc().is_some());

    apply_node_create(&mut indexes, &label_set(&label), &blocker, 1).unwrap();
    assert!(
        indexes.get(&key).unwrap().probe_arc().is_none(),
        "one unkeyable tuple must make the composite index decline"
    );

    apply_node_delete(&mut indexes, &label_set(&label), &blocker, 1).unwrap();
    assert!(indexes.get(&key).unwrap().probe_arc().is_some());
}
