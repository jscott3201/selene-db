use super::*;

#[test]
fn apply_node_update_only_touches_affected_indexes() {
    // With many registered (label, property) pairs but a single-property
    // update, candidate_keys must select exactly the affected key rather
    // than scanning the entire registry. Test by registering many
    // unrelated indexes and verifying their `Arc<TypedIndex>` strong
    // counts stay 1 (untouched / unshared) after the update - the
    // affected index's count rises because Arc::make_mut clones it
    // when the bitmap mutates.
    let label = db_string("pi.affected.label").unwrap();
    let age = db_string("pi.affected.age").unwrap();
    let unrelated_label = db_string("pi.affected.other-label").unwrap();
    let unrelated_property = db_string("pi.affected.other-prop").unwrap();

    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    // Many unrelated indexes that the update should NOT touch.
    for i in 0..10 {
        let extra_label = db_string(&format!("pi.affected.extra-label-{i}")).unwrap();
        let extra_property = db_string(&format!("pi.affected.extra-prop-{i}")).unwrap();
        indexes.insert((extra_label, extra_property), entry(TypedIndexKind::I64));
    }
    indexes.insert(
        (unrelated_label.clone(), unrelated_property.clone()),
        entry(TypedIndexKind::String),
    );

    let unrelated = Arc::clone(
        &indexes
            .get(&(unrelated_label, unrelated_property))
            .unwrap()
            .index,
    );
    let extra_clones: Vec<_> = (0..10)
        .map(|i| {
            let l = db_string(&format!("pi.affected.extra-label-{i}")).unwrap();
            let p = db_string(&format!("pi.affected.extra-prop-{i}")).unwrap();
            Arc::clone(&indexes.get(&(l, p)).unwrap().index)
        })
        .collect();

    let labels = LabelSet::single(label.clone());
    let old_props = property_map([(age.clone(), Value::Int(30))]);
    let new_props = property_map([(age.clone(), Value::Int(31))]);
    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 0).unwrap();

    // Unrelated indexes' Arcs are not cloned - strong_count stays at the
    // pre-update value (this clone + the registry's clone = 2).
    for extra in &extra_clones {
        assert_eq!(Arc::strong_count(extra), 2);
    }
    assert_eq!(Arc::strong_count(&unrelated), 2);
    // The affected index DOES rise above 2 because Arc::make_mut cloned
    // it for the update.
    let affected = indexes.get(&(label, age)).unwrap();
    assert!(!Arc::ptr_eq(&affected.index, &extra_clones[0]));
}
