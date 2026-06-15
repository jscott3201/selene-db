use super::*;

#[test]
fn apply_node_create_populates_matching_indexes() {
    let label = db_string("pi.create.label").unwrap();
    let age = db_string("pi.create.age").unwrap();
    let name = db_string("pi.create.name").unwrap();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));
    let props = property_map([
        (age.clone(), Value::Int(30)),
        (
            name.clone(),
            Value::String(db_string("pi.create.ada").unwrap()),
        ),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert!(rows(&indexes, label.clone(), age, &Value::Int(30)).contains(0));
    assert!(
        rows(
            &indexes,
            label,
            name,
            &Value::String(db_string("pi.create.ada").unwrap())
        )
        .contains(0)
    );
}

#[test]
fn apply_node_delete_removes_matching_entries() {
    let label = db_string("pi.delete.label").unwrap();
    let age = db_string("pi.delete.age").unwrap();
    let props = property_map([(age.clone(), Value::Int(30))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 4).unwrap();

    apply_node_delete(&mut indexes, &LabelSet::single(label.clone()), &props, 4).unwrap();

    assert!(rows(&indexes, label, age, &Value::Int(30)).is_empty());
}

#[test]
fn apply_node_update_with_label_add_inserts_relevant_property() {
    let label = db_string("pi.update.label-add").unwrap();
    let age = db_string("pi.update.label-add.age").unwrap();
    let props = property_map([(age.clone(), Value::Int(41))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    apply_node_update(
        &mut indexes,
        &LabelSet::new(),
        &props,
        &LabelSet::single(label.clone()),
        &props,
        8,
    )
    .unwrap();

    assert!(rows(&indexes, label, age, &Value::Int(41)).contains(8));
}

#[test]
fn apply_node_update_with_label_remove_deletes_relevant_property() {
    let label = db_string("pi.update.label-remove").unwrap();
    let age = db_string("pi.update.label-remove.age").unwrap();
    let props = property_map([(age.clone(), Value::Int(41))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 8).unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &props,
        &LabelSet::new(),
        &props,
        8,
    )
    .unwrap();

    assert!(rows(&indexes, label, age, &Value::Int(41)).is_empty());
}

#[test]
fn apply_node_update_with_property_set_moves_rows_between_keys() {
    let label = db_string("pi.update.prop-set").unwrap();
    let age = db_string("pi.update.prop-set.age").unwrap();
    let old_props = property_map([(age.clone(), Value::Int(41))]);
    let new_props = property_map([(age.clone(), Value::Int(42))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        8,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        8,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), age.clone(), &Value::Int(41)).is_empty());
    assert!(rows(&indexes, label, age, &Value::Int(42)).contains(8));
}

#[test]
fn apply_node_update_with_property_remove_drops_row() {
    let label = db_string("pi.update.prop-remove").unwrap();
    let age = db_string("pi.update.prop-remove.age").unwrap();
    let old_props = property_map([(age.clone(), Value::Int(41))]);
    let new_props = PropertyMap::new();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        8,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        8,
    )
    .unwrap();

    assert!(rows(&indexes, label, age, &Value::Int(41)).is_empty());
}

#[test]
fn kind_mismatch_skips_commit_update() {
    let label = db_string("pi.kind.label").unwrap();
    let age = db_string("pi.kind.age").unwrap();
    let props = property_map([(
        age.clone(),
        Value::String(db_string("pi.kind.old").unwrap()),
    )]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert_eq!(indexes.get(&(label, age)).unwrap().index.cardinality(), 0);
}

#[test]
fn null_values_are_skipped() {
    let label = db_string("pi.null.label").unwrap();
    let age = db_string("pi.null.age").unwrap();
    let props = property_map([(age.clone(), Value::Null)]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert_eq!(indexes.get(&(label, age)).unwrap().index.cardinality(), 0);
}

#[test]
fn untouched_indexes_keep_their_arc() {
    let label = db_string("pi.cow.label").unwrap();
    let age = db_string("pi.cow.age").unwrap();
    let name = db_string("pi.cow.name").unwrap();
    let old_props = property_map([
        (age.clone(), Value::Int(1)),
        (
            name.clone(),
            Value::String(db_string("pi.cow.ada").unwrap()),
        ),
    ]);
    let new_props = property_map([
        (age.clone(), Value::Int(2)),
        (
            name.clone(),
            Value::String(db_string("pi.cow.ada").unwrap()),
        ),
    ]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age), entry(TypedIndexKind::I64));
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        0,
    )
    .unwrap();
    let name_index = Arc::clone(&indexes.get(&(label.clone(), name.clone())).unwrap().index);

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        0,
    )
    .unwrap();

    assert!(Arc::ptr_eq(
        &name_index,
        &indexes.get(&(label, name)).unwrap().index
    ));
}

#[test]
fn apply_property_diff_remove_is_covered_by_update_shape() {
    let key = db_string("pi.diff.key").unwrap();
    let diff = PropertyDiff::new([], [key.clone()]).unwrap();
    assert_eq!(diff.removed.iter().cloned().collect::<Vec<_>>(), vec![key]);
}
