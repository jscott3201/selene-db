use std::sync::Arc;

use selene_core::{DbString, GraphId, LabelSet, PropertyDiff, PropertyMap, Value, db_string};

use super::*;
use crate::graph::PropertyIndexEntry;
use crate::typed_index::{TypedIndexKind, TypedIndexValueError};

fn property_map(pairs: impl IntoIterator<Item = (DbString, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn decimal(value: &str) -> rust_decimal::Decimal {
    value.parse().expect("test decimal parses")
}

fn zoned(value: &str) -> jiff::Zoned {
    value.parse().expect("test zoned datetime parses")
}

fn entry(kind: TypedIndexKind) -> PropertyIndexEntry {
    PropertyIndexEntry::new(TypedIndex::new(kind), None)
}

fn rows(
    indexes: &PropertyIndexMap,
    label: DbString,
    property: DbString,
    value: &Value,
) -> RoaringRows {
    RoaringRows(
        indexes
            .get(&(label, property))
            .and_then(|index| index.index.lookup_eq(value))
            .map(std::borrow::Cow::into_owned)
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
fn build_property_index_is_strict_for_existing_data() {
    let label = db_string("pi.build.label").unwrap();
    let age = db_string("pi.build.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([(
        age.clone(),
        Value::String(db_string("wrong").unwrap()),
    )]));
    graph.node_store.alive.insert(0);

    let err =
        build_property_index(&graph, label.clone(), age.clone(), TypedIndexKind::I64).unwrap_err();

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
fn apply_node_create_admits_string_into_string_index() {
    // `Value::String` reaches the property-index commit and produces a
    // findable index entry.
    let label = db_string("pi.string.create.label").unwrap();
    let name = db_string("pi.string.create.name").unwrap();
    let probe = db_string("pi.string.create.unique-1").unwrap();
    let props = property_map([(name.clone(), Value::String(probe.clone()))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 5).unwrap();

    assert!(rows(&indexes, label, name, &Value::String(probe)).contains(5));
}

#[test]
fn apply_node_update_moves_bool_index_key() {
    let label = db_string("pi.bool.update.label").unwrap();
    let active = db_string("pi.bool.update.active").unwrap();
    let old_props = property_map([(active.clone(), Value::Bool(false))]);
    let new_props = property_map([(active.clone(), Value::Bool(true))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), active.clone()), entry(TypedIndexKind::Bool));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        9,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        9,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), active.clone(), &Value::Bool(false)).is_empty());
    assert!(rows(&indexes, label, active, &Value::Bool(true)).contains(9));
}

#[test]
fn apply_node_update_moves_u64_index_key() {
    let label = db_string("pi.u64.update.label").unwrap();
    let count = db_string("pi.u64.update.count").unwrap();
    let old_props = property_map([(count.clone(), Value::Uint(7))]);
    let new_props = property_map([(count.clone(), Value::Uint(u64::MAX))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), count.clone()), entry(TypedIndexKind::U64));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        10,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        10,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), count.clone(), &Value::Uint(7)).is_empty());
    assert!(rows(&indexes, label, count, &Value::Uint(u64::MAX)).contains(10));
}

#[test]
fn apply_node_update_moves_exact_numeric_index_keys() {
    let label = db_string("pi.exact.update.label").unwrap();
    let signed = db_string("pi.exact.update.signed").unwrap();
    let unsigned = db_string("pi.exact.update.unsigned").unwrap();
    let amount = db_string("pi.exact.update.amount").unwrap();
    let old_props = property_map([
        (signed.clone(), Value::Int128(i128::MIN + 1)),
        (unsigned.clone(), Value::Uint128(u64::MAX as u128 + 1)),
        (amount.clone(), Value::Decimal(decimal("1.25"))),
    ]);
    let new_props = property_map([
        (signed.clone(), Value::Int128(i128::MAX - 1)),
        (unsigned.clone(), Value::Uint128(u128::MAX - 1)),
        (amount.clone(), Value::Decimal(decimal("2.50"))),
    ]);
    let labels = LabelSet::single(label.clone());
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), signed.clone()), entry(TypedIndexKind::I128));
    indexes.insert(
        (label.clone(), unsigned.clone()),
        entry(TypedIndexKind::U128),
    );
    indexes.insert(
        (label.clone(), amount.clone()),
        entry(TypedIndexKind::Decimal),
    );
    apply_node_create(&mut indexes, &labels, &old_props, 11).unwrap();

    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 11).unwrap();

    assert!(
        rows(
            &indexes,
            label.clone(),
            signed.clone(),
            &Value::Int128(i128::MIN + 1)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            signed,
            &Value::Int128(i128::MAX - 1)
        )
        .contains(11)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            unsigned.clone(),
            &Value::Uint128(u64::MAX as u128 + 1)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            unsigned,
            &Value::Uint128(u128::MAX - 1)
        )
        .contains(11)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            amount.clone(),
            &Value::Decimal(decimal("1.25"))
        )
        .is_empty()
    );
    assert!(rows(&indexes, label, amount, &Value::Decimal(decimal("2.50"))).contains(11));
}

#[test]
fn apply_node_update_moves_temporal_time_index_keys() {
    let label = db_string("pi.temporal.update.label").unwrap();
    let zoned_dt = db_string("pi.temporal.update.zoned-dt").unwrap();
    let local_time = db_string("pi.temporal.update.local-time").unwrap();
    let zoned_time = db_string("pi.temporal.update.zoned-time").unwrap();
    let old_zdt = zoned("2026-05-07T09:00:00-04[America/New_York]");
    let new_zdt = zoned("2026-05-07T12:00:00-04[America/New_York]");
    let old_zt = zoned("2026-05-07T09:30:00-04[America/New_York]");
    let new_zt = zoned("2026-05-07T12:30:00-04[America/New_York]");
    let old_lt = "09:30:00".parse().unwrap();
    let new_lt = "12:30:00".parse().unwrap();
    let old_props = property_map([
        (
            zoned_dt.clone(),
            Value::ZonedDateTime(Box::new(old_zdt.clone())),
        ),
        (local_time.clone(), Value::LocalTime(old_lt)),
        (
            zoned_time.clone(),
            Value::ZonedTime(Box::new(old_zt.clone())),
        ),
    ]);
    let new_props = property_map([
        (
            zoned_dt.clone(),
            Value::ZonedDateTime(Box::new(new_zdt.clone())),
        ),
        (local_time.clone(), Value::LocalTime(new_lt)),
        (
            zoned_time.clone(),
            Value::ZonedTime(Box::new(new_zt.clone())),
        ),
    ]);
    let labels = LabelSet::single(label.clone());
    let mut indexes = PropertyIndexMap::default();
    indexes.insert(
        (label.clone(), zoned_dt.clone()),
        entry(TypedIndexKind::ZonedDateTime),
    );
    indexes.insert(
        (label.clone(), local_time.clone()),
        entry(TypedIndexKind::LocalTime),
    );
    indexes.insert(
        (label.clone(), zoned_time.clone()),
        entry(TypedIndexKind::ZonedTime),
    );
    apply_node_create(&mut indexes, &labels, &old_props, 12).unwrap();

    apply_node_update(&mut indexes, &labels, &old_props, &labels, &new_props, 12).unwrap();

    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_dt.clone(),
            &Value::ZonedDateTime(Box::new(old_zdt))
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_dt,
            &Value::ZonedDateTime(Box::new(new_zdt))
        )
        .contains(12)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            local_time.clone(),
            &Value::LocalTime(old_lt)
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            local_time,
            &Value::LocalTime(new_lt)
        )
        .contains(12)
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            zoned_time.clone(),
            &Value::ZonedTime(Box::new(old_zt))
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label,
            zoned_time,
            &Value::ZonedTime(Box::new(new_zt))
        )
        .contains(12)
    );
}

#[test]
fn apply_node_update_moves_string_index_key() {
    // SET against an INDEXED column moves the row from the previous key to
    // the new one.
    let label = db_string("pi.string.update.label").unwrap();
    let name = db_string("pi.string.update.name").unwrap();
    let old = db_string("pi.string.update.old").unwrap();
    let new_key = db_string("pi.string.update.new-unique").unwrap();
    let old_props = property_map([(name.clone(), Value::String(old.clone()))]);
    let new_props = property_map([(name.clone(), Value::String(new_key.clone()))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));
    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        9,
    )
    .unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        9,
    )
    .unwrap();

    assert!(rows(&indexes, label.clone(), name.clone(), &Value::String(old)).is_empty());
    assert!(rows(&indexes, label, name, &Value::String(new_key)).contains(9));
}

#[test]
fn build_property_index_admits_existing_string_rows() {
    let label = db_string("pi.build.string.label").unwrap();
    let name = db_string("pi.build.string.name").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(11));
    let unique = (0..3)
        .map(|i| db_string(&format!("pi.build.string.foo_{i}")).unwrap())
        .collect::<Vec<_>>();
    for (row, content) in unique.iter().enumerate() {
        graph
            .node_store
            .labels
            .push(LabelSet::single(label.clone()));
        graph.node_store.properties.push(property_map([(
            name.clone(),
            Value::String(content.clone()),
        )]));
        graph.node_store.alive.insert(row as u32);
    }

    let index = build_property_index(&graph, label, name, TypedIndexKind::String).expect("admits");

    assert_eq!(index.cardinality(), 3);
}

#[test]
fn index_rejection_keeps_kind_mismatch_path_unchanged() {
    let label = db_string("pi.kind-mismatch.label").unwrap();
    let name = db_string("pi.kind-mismatch.name").unwrap();
    let synthetic = TypedIndexValueError::KindMismatch {
        expected_kind: TypedIndexKind::I64,
        observed: "String",
    };

    let promoted = index_rejection(label, name, synthetic);

    assert!(matches!(
        promoted,
        GraphError::IndexValueRejected {
            expected_kind: TypedIndexKind::I64,
            observed: "String",
            ..
        }
    ));
}

#[test]
fn apply_property_diff_remove_is_covered_by_update_shape() {
    let key = db_string("pi.diff.key").unwrap();
    let diff = PropertyDiff::new([], [key.clone()]).unwrap();
    assert_eq!(diff.removed.iter().cloned().collect::<Vec<_>>(), vec![key]);
}

#[test]
fn rebuild_property_indexes_is_lenient_on_kind_drift() {
    // Snapshot has a property index registered as I64, but the column
    // value is a String — a state that's reachable at runtime when an
    // open-graph update writes a kind-mismatched value (the commit-path
    // logs and skips). Recovery must reconstruct the registry without
    // hard-failing, otherwise a runtime-accepted snapshot becomes
    // unloadable.
    let label = db_string("pi.rebuild.label").unwrap();
    let age = db_string("pi.rebuild.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    // Row 0: matching kind (Int 30) — should land in the rebuilt index.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph
        .node_store
        .properties
        .push(property_map([(age.clone(), Value::Int(30))]));
    graph.node_store.alive.insert(0);
    // Row 1: mismatched kind (String) — should be skipped, not abort.
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([(
        age.clone(),
        Value::String(db_string("pi.rebuild.wrong").unwrap()),
    )]));
    graph.node_store.alive.insert(1);
    // Pre-register the index (will be cleared and rebuilt by
    // rebuild_property_indexes).
    graph
        .property_index
        .insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    rebuild_property_indexes(&mut graph).expect("lenient rebuild does not error on drift");

    // The matching row landed; the mismatched row was logged and skipped.
    let index = graph.property_index.get(&(label, age)).unwrap();
    let hits = index
        .index
        .lookup_eq(&Value::Int(30))
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
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

    // Unrelated indexes' Arcs are not cloned — strong_count stays at the
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
