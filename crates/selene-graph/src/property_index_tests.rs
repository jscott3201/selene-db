use std::sync::Arc;

use selene_core::{
    CoreError, GraphId, IStr, LabelSet, PropertyDiff, PropertyMap, Value, intern, lookup,
};

use super::*;
use crate::graph::PropertyIndexEntry;
use crate::typed_index::{TypedIndexKind, TypedIndexValueError};

fn property_map(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn entry(kind: TypedIndexKind) -> PropertyIndexEntry {
    PropertyIndexEntry::new(TypedIndex::new(kind), None)
}

fn rows(indexes: &PropertyIndexMap, label: IStr, property: IStr, value: &Value) -> RoaringRows {
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
    let label = intern("pi.create.label").unwrap();
    let age = intern("pi.create.age").unwrap();
    let name = intern("pi.create.name").unwrap();
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));
    let props = property_map([
        (age.clone(), Value::Int(30)),
        (
            name.clone(),
            Value::String(intern("pi.create.ada").unwrap()),
        ),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert!(rows(&indexes, label.clone(), age, &Value::Int(30)).contains(0));
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
    let props = property_map([(age.clone(), Value::Int(30))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 4).unwrap();

    apply_node_delete(&mut indexes, &LabelSet::single(label.clone()), &props, 4).unwrap();

    assert!(rows(&indexes, label, age, &Value::Int(30)).is_empty());
}

#[test]
fn apply_node_update_with_label_add_inserts_relevant_property() {
    let label = intern("pi.update.label-add").unwrap();
    let age = intern("pi.update.label-add.age").unwrap();
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
    let label = intern("pi.update.label-remove").unwrap();
    let age = intern("pi.update.label-remove.age").unwrap();
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
    let label = intern("pi.update.prop-set").unwrap();
    let age = intern("pi.update.prop-set.age").unwrap();
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
    let label = intern("pi.update.prop-remove").unwrap();
    let age = intern("pi.update.prop-remove.age").unwrap();
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
    let label = intern("pi.kind.label").unwrap();
    let age = intern("pi.kind.age").unwrap();
    let props = property_map([(age.clone(), Value::String(intern("pi.kind.old").unwrap()))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert_eq!(indexes.get(&(label, age)).unwrap().index.cardinality(), 0);
}

#[test]
fn null_values_are_skipped() {
    let label = intern("pi.null.label").unwrap();
    let age = intern("pi.null.age").unwrap();
    let props = property_map([(age.clone(), Value::Null)]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 0).unwrap();

    assert_eq!(indexes.get(&(label, age)).unwrap().index.cardinality(), 0);
}

#[test]
fn untouched_indexes_keep_their_arc() {
    let label = intern("pi.cow.label").unwrap();
    let age = intern("pi.cow.age").unwrap();
    let name = intern("pi.cow.name").unwrap();
    let old_props = property_map([
        (age.clone(), Value::Int(1)),
        (name.clone(), Value::String(intern("pi.cow.ada").unwrap())),
    ]);
    let new_props = property_map([
        (age.clone(), Value::Int(2)),
        (name.clone(), Value::String(intern("pi.cow.ada").unwrap())),
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
    let label = intern("pi.build.label").unwrap();
    let age = intern("pi.build.age").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(1));
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([(
        age.clone(),
        Value::String(intern("wrong").unwrap()),
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
fn apply_node_create_admits_external_string_into_string_index() {
    // BRIEF-153 bar 1: `Value::ExternalString` reaches the property-index
    // commit and produces an entry equivalent to one produced by
    // `Value::String` with the same content.
    let label = intern("pi.external.create.label").unwrap();
    let name = intern("pi.external.create.name").unwrap();
    let probe = "pi.external.create.unique-1";
    assert!(lookup(probe).is_none());
    let props = property_map([(name.clone(), Value::ExternalString(Arc::<str>::from(probe)))]);
    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), name.clone()), entry(TypedIndexKind::String));

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 5).unwrap();

    // The admitted IStr is now in the pool and the index entry is keyed
    // on it; probing with either variant locates row 5.
    let admitted = lookup(probe).expect("commit admitted the IStr");
    assert!(
        rows(
            &indexes,
            label.clone(),
            name.clone(),
            &Value::String(admitted)
        )
        .contains(5)
    );
    assert!(
        rows(
            &indexes,
            label,
            name,
            &Value::ExternalString(Arc::<str>::from(probe)),
        )
        .contains(5)
    );
}

#[test]
fn apply_node_update_admits_external_string_replacement() {
    // SET against an INDEXED column moves the row from the previous
    // String-bound key to the newly admitted ExternalString-bound key.
    let label = intern("pi.external.update.label").unwrap();
    let name = intern("pi.external.update.name").unwrap();
    let old = intern("pi.external.update.old").unwrap();
    let new_probe = "pi.external.update.new-unique";
    assert!(lookup(new_probe).is_none());
    let old_props = property_map([(name.clone(), Value::String(old.clone()))]);
    let new_props = property_map([(
        name.clone(),
        Value::ExternalString(Arc::<str>::from(new_probe)),
    )]);
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
    let admitted = lookup(new_probe).expect("commit admitted the IStr");
    assert!(rows(&indexes, label, name, &Value::String(admitted)).contains(9));
}

#[test]
fn build_property_index_admits_existing_external_string_rows() {
    // BRIEF-153 bar 9: pre-fix HEAD rejected ExternalString rows during
    // registration; post-fix the strict-build admits them (DDL = consent).
    let label = intern("pi.build.external.label").unwrap();
    let name = intern("pi.build.external.name").unwrap();
    let mut graph = crate::SeleneGraph::new(GraphId::new(11));
    let unique = (0..3)
        .map(|i| format!("pi.build.external.foo_{i}"))
        .collect::<Vec<_>>();
    for (row, content) in unique.iter().enumerate() {
        assert!(lookup(content).is_none());
        graph
            .node_store
            .labels
            .push(LabelSet::single(label.clone()));
        graph.node_store.properties.push(property_map([(
            name.clone(),
            Value::ExternalString(Arc::<str>::from(content.as_str())),
        )]));
        graph.node_store.alive.insert(row as u32);
    }

    let index = build_property_index(&graph, label, name, TypedIndexKind::String).expect("admits");

    assert_eq!(index.cardinality(), 3);
    for content in &unique {
        assert!(
            lookup(content).is_some(),
            "strict build admits each ExternalString row's content"
        );
    }
}

#[test]
fn index_rejection_promotes_admission_failed_to_index_admission_exhausted() {
    // BRIEF-153 bar 3 (synthetic injection per Q7/F10): the mutator
    // commit-path helper promotes the synthetic AdmissionFailed inner
    // error to `GraphError::IndexAdmissionExhausted` carrying the
    // CoreError source intact, with GQLSTATUS 5GQL1.
    let label = intern("pi.admit-fail.label").unwrap();
    let name = intern("pi.admit-fail.name").unwrap();
    let synthetic = TypedIndexValueError::AdmissionFailed {
        expected_kind: TypedIndexKind::String,
        reason: CoreError::IStrCapExceeded {
            count: 1_000_000,
            max: 1_000_000,
        },
    };

    let promoted = index_rejection(label.clone(), name.clone(), synthetic);

    let GraphError::IndexAdmissionExhausted {
        label: err_label,
        property: err_property,
        source,
    } = &promoted
    else {
        panic!("expected IndexAdmissionExhausted, got {promoted:?}");
    };
    assert_eq!(*err_label, label);
    assert_eq!(*err_property, name);
    assert!(matches!(
        source,
        CoreError::IStrCapExceeded {
            count: 1_000_000,
            max: 1_000_000,
        }
    ));
    assert_eq!(promoted.gqlstatus(), "5GQL1");
}

#[test]
fn index_rejection_keeps_kind_mismatch_path_unchanged() {
    let label = intern("pi.kind-mismatch.label").unwrap();
    let name = intern("pi.kind-mismatch.name").unwrap();
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
    let key = intern("pi.diff.key").unwrap();
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
    let label = intern("pi.rebuild.label").unwrap();
    let age = intern("pi.rebuild.age").unwrap();
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
        Value::String(intern("pi.rebuild.wrong").unwrap()),
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
    let label = intern("pi.affected.label").unwrap();
    let age = intern("pi.affected.age").unwrap();
    let unrelated_label = intern("pi.affected.other-label").unwrap();
    let unrelated_property = intern("pi.affected.other-prop").unwrap();

    let mut indexes = PropertyIndexMap::default();
    indexes.insert((label.clone(), age.clone()), entry(TypedIndexKind::I64));
    // Many unrelated indexes that the update should NOT touch.
    for i in 0..10 {
        let extra_label = intern(&format!("pi.affected.extra-label-{i}")).unwrap();
        let extra_property = intern(&format!("pi.affected.extra-prop-{i}")).unwrap();
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
            let l = intern(&format!("pi.affected.extra-label-{i}")).unwrap();
            let p = intern(&format!("pi.affected.extra-prop-{i}")).unwrap();
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
