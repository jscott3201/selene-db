use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, PropertyDiff, PropertyMap, Value, intern};

use super::*;
use crate::typed_index::TypedIndexKind;

fn property_map(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn rows(indexes: &PropertyIndexMap, label: IStr, property: IStr, value: &Value) -> RoaringRows {
    RoaringRows(
        indexes
            .get(&(label, property))
            .and_then(|index| index.lookup_eq(value))
            .unwrap_or_default(),
    )
}

struct RoaringRows(roaring::RoaringBitmap);

impl RoaringRows {
    fn contains(&self, row: u32) -> bool {
        self.0.contains(row)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[test]
fn apply_node_create_populates_matching_indexes() {
    let label = intern("pi.create.label").unwrap();
    let age = intern("pi.create.age").unwrap();
    let name = intern("pi.create.name").unwrap();
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    indexes.insert(
        (label, name),
        Arc::new(TypedIndex::new(TypedIndexKind::String)),
    );
    let props = property_map([
        (age, Value::Int(30)),
        (name, Value::String(intern("pi.create.ada").unwrap())),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 0);

    assert!(rows(&indexes, label, age, &Value::Int(30)).contains(0));
    assert!(
        rows(
            &indexes,
            label,
            name,
            &Value::String(intern("pi.create.ada").unwrap())
        )
        .contains(0)
    );
}

#[test]
fn apply_node_delete_removes_matching_entries() {
    let label = intern("pi.delete.label").unwrap();
    let age = intern("pi.delete.age").unwrap();
    let props = property_map([(age, Value::Int(30))]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 4);

    apply_node_delete(&mut indexes, &LabelSet::single(label), &props, 4);

    assert!(rows(&indexes, label, age, &Value::Int(30)).is_empty());
}

#[test]
fn apply_node_update_with_label_add_inserts_relevant_property() {
    let label = intern("pi.update.label-add").unwrap();
    let age = intern("pi.update.label-add.age").unwrap();
    let props = property_map([(age, Value::Int(41))]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));

    apply_node_update(
        &mut indexes,
        &LabelSet::new(),
        &props,
        &LabelSet::single(label),
        &props,
        8,
    );

    assert!(rows(&indexes, label, age, &Value::Int(41)).contains(8));
}

#[test]
fn apply_node_update_with_label_remove_deletes_relevant_property() {
    let label = intern("pi.update.label-remove").unwrap();
    let age = intern("pi.update.label-remove.age").unwrap();
    let props = property_map([(age, Value::Int(41))]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 8);

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &props,
        &LabelSet::new(),
        &props,
        8,
    );

    assert!(rows(&indexes, label, age, &Value::Int(41)).is_empty());
}

#[test]
fn apply_node_update_with_property_set_moves_rows_between_keys() {
    let label = intern("pi.update.prop-set").unwrap();
    let age = intern("pi.update.prop-set.age").unwrap();
    let old_props = property_map([(age, Value::Int(41))]);
    let new_props = property_map([(age, Value::Int(42))]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    apply_node_create(&mut indexes, &LabelSet::single(label), &old_props, 8);

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &old_props,
        &LabelSet::single(label),
        &new_props,
        8,
    );

    assert!(rows(&indexes, label, age, &Value::Int(41)).is_empty());
    assert!(rows(&indexes, label, age, &Value::Int(42)).contains(8));
}

#[test]
fn apply_node_update_with_property_remove_drops_row() {
    let label = intern("pi.update.prop-remove").unwrap();
    let age = intern("pi.update.prop-remove.age").unwrap();
    let old_props = property_map([(age, Value::Int(41))]);
    let new_props = PropertyMap::new();
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    apply_node_create(&mut indexes, &LabelSet::single(label), &old_props, 8);

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &old_props,
        &LabelSet::single(label),
        &new_props,
        8,
    );

    assert!(rows(&indexes, label, age, &Value::Int(41)).is_empty());
}

#[test]
fn kind_mismatch_skips_commit_update() {
    let label = intern("pi.kind.label").unwrap();
    let age = intern("pi.kind.age").unwrap();
    let props = property_map([(age, Value::String(intern("pi.kind.old").unwrap()))]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));

    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 0);

    assert_eq!(indexes.get(&(label, age)).unwrap().cardinality(), 0);
}

#[test]
fn null_values_are_skipped() {
    let label = intern("pi.null.label").unwrap();
    let age = intern("pi.null.age").unwrap();
    let props = property_map([(age, Value::Null)]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));

    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 0);

    assert_eq!(indexes.get(&(label, age)).unwrap().cardinality(), 0);
}

#[test]
fn untouched_indexes_keep_their_arc() {
    let label = intern("pi.cow.label").unwrap();
    let age = intern("pi.cow.age").unwrap();
    let name = intern("pi.cow.name").unwrap();
    let old_props = property_map([
        (age, Value::Int(1)),
        (name, Value::String(intern("pi.cow.ada").unwrap())),
    ]);
    let new_props = property_map([
        (age, Value::Int(2)),
        (name, Value::String(intern("pi.cow.ada").unwrap())),
    ]);
    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    indexes.insert(
        (label, name),
        Arc::new(TypedIndex::new(TypedIndexKind::String)),
    );
    apply_node_create(&mut indexes, &LabelSet::single(label), &old_props, 0);
    let name_index = indexes.get(&(label, name)).unwrap().clone();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &old_props,
        &LabelSet::single(label),
        &new_props,
        0,
    );

    assert!(Arc::ptr_eq(
        &name_index,
        indexes.get(&(label, name)).unwrap()
    ));
}

#[test]
fn build_property_index_is_strict_for_existing_data() {
    let label = intern("pi.build.label").unwrap();
    let age = intern("pi.build.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    graph.node_store.labels.push(LabelSet::single(label));
    graph.node_store.properties.push(property_map([(
        age,
        Value::String(intern("wrong").unwrap()),
    )]));
    graph.node_store.alive.insert(0);

    let err = build_property_index(&graph, label, age, TypedIndexKind::I64).unwrap_err();

    assert!(matches!(
        err,
        GraphError::IndexValueRejected {
            label: err_label,
            property: err_property,
            expected_kind: TypedIndexKind::I64,
            observed: "String",
        } if err_label == label && err_property == age
    ));
}

#[test]
fn apply_property_diff_remove_is_covered_by_update_shape() {
    let key = intern("pi.diff.key").unwrap();
    let diff = PropertyDiff::new([], [key]).unwrap();
    assert_eq!(diff.removed.iter().copied().collect::<Vec<_>>(), vec![key]);
}

#[test]
fn rebuild_property_indexes_is_lenient_on_kind_drift() {
    // Snapshot has a property index registered as I64, but the column
    // value is a String — a state that's reachable at runtime when an
    // open-graph update writes a kind-mismatched value (the commit-path
    // logs and skips). Recovery must reconstruct the registry without
    // hard-failing, otherwise a runtime-accepted snapshot becomes
    // unloadable.
    let label = intern("pi.rebuild.label").unwrap();
    let age = intern("pi.rebuild.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    // Row 0: matching kind (Int 30) — should land in the rebuilt index.
    graph.node_store.labels.push(LabelSet::single(label));
    graph
        .node_store
        .properties
        .push(property_map([(age, Value::Int(30))]));
    graph.node_store.alive.insert(0);
    // Row 1: mismatched kind (String) — should be skipped, not abort.
    graph.node_store.labels.push(LabelSet::single(label));
    graph.node_store.properties.push(property_map([(
        age,
        Value::String(intern("pi.rebuild.wrong").unwrap()),
    )]));
    graph.node_store.alive.insert(1);
    // Pre-register the index (will be cleared and rebuilt by
    // rebuild_property_indexes).
    graph
        .property_index
        .insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));

    rebuild_property_indexes(&mut graph).expect("lenient rebuild does not error on drift");

    // The matching row landed; the mismatched row was logged and skipped.
    let index = graph.property_index.get(&(label, age)).unwrap();
    let hits = index.lookup_eq(&Value::Int(30)).unwrap_or_default();
    assert!(hits.contains(0));
    assert!(!hits.contains(1));
}

#[test]
fn apply_node_update_only_touches_affected_indexes() {
    // With many registered (label, property) pairs but a single-property
    // update, candidate_keys must select exactly the affected key rather
    // than scanning the entire registry. Test by registering many
    // unrelated indexes and verifying their `Arc<TypedIndex>` strong
    // counts stay 1 (untouched / unshared) after the update — the
    // affected index's count rises because Arc::make_mut clones it
    // when the bitmap mutates.
    let label = intern("pi.affected.label").unwrap();
    let age = intern("pi.affected.age").unwrap();
    let unrelated_label = intern("pi.affected.other-label").unwrap();
    let unrelated_property = intern("pi.affected.other-prop").unwrap();

    let mut indexes = PropertyIndexMap::new();
    indexes.insert((label, age), Arc::new(TypedIndex::new(TypedIndexKind::I64)));
    // Many unrelated indexes that the update should NOT touch.
    for i in 0..10 {
        let extra_label = intern(&format!("pi.affected.extra-label-{i}")).unwrap();
        let extra_property = intern(&format!("pi.affected.extra-prop-{i}")).unwrap();
        indexes.insert(
            (extra_label, extra_property),
            Arc::new(TypedIndex::new(TypedIndexKind::I64)),
        );
    }
    indexes.insert(
        (unrelated_label, unrelated_property),
        Arc::new(TypedIndex::new(TypedIndexKind::String)),
    );

    let unrelated = indexes
        .get(&(unrelated_label, unrelated_property))
        .unwrap()
        .clone();
    let extra_clones: Vec<_> = (0..10)
        .map(|i| {
            let l = intern(&format!("pi.affected.extra-label-{i}")).unwrap();
            let p = intern(&format!("pi.affected.extra-prop-{i}")).unwrap();
            indexes.get(&(l, p)).unwrap().clone()
        })
        .collect();

    let labels = LabelSet::single(label);
    let old_props = property_map([(age, Value::Int(30))]);
    let new_props = property_map([(age, Value::Int(31))]);
    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 0);

    // Unrelated indexes' Arcs are not cloned — strong_count stays at the
    // pre-update value (this clone + the registry's clone = 2).
    for extra in &extra_clones {
        assert_eq!(Arc::strong_count(extra), 2);
    }
    assert_eq!(Arc::strong_count(&unrelated), 2);
    // The affected index DOES rise above 2 because Arc::make_mut cloned
    // it for the update.
    let affected = indexes.get(&(label, age)).unwrap();
    assert!(!Arc::ptr_eq(affected, &extra_clones[0]));
}
