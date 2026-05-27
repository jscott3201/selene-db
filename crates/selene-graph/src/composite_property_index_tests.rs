use std::sync::Arc;

use roaring::RoaringBitmap;
use selene_core::{CoreError, IStr, LabelSet, PropertyMap, Value, intern, lookup};
use smallvec::{SmallVec, smallvec};

use super::*;
use crate::composite_typed_index::CompositeIndexValueError;
use crate::graph::{CompositePropertyIndexEntry, composite_property_key};
use crate::{CompositeTypedIndex, TypedIndexKind};

fn property_map(pairs: impl IntoIterator<Item = (IStr, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

fn entry(
    properties: SmallVec<[IStr; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) -> CompositePropertyIndexEntry {
    CompositePropertyIndexEntry::new(CompositeTypedIndex::new(kinds), properties, None)
}

fn insert_entry(
    indexes: &mut CompositeIndexMap,
    label: IStr,
    properties: SmallVec<[IStr; 4]>,
    kinds: SmallVec<[TypedIndexKind; 4]>,
) {
    indexes.insert(
        (label, composite_property_key(&properties)),
        CompositePropertyIndexEntry::new(CompositeTypedIndex::new(kinds), properties, None),
    );
}

fn rows(
    indexes: &CompositeIndexMap,
    label: IStr,
    properties: &[IStr],
    values: &[Value],
) -> RoaringBitmap {
    let Some(entry) = indexes.get(&(label, composite_property_key(properties))) else {
        return RoaringBitmap::new();
    };
    let refs = values.iter().collect::<Vec<_>>();
    let key = entry.index.key_from_values_admit(&refs).unwrap();
    entry.index.lookup_key(&key).cloned().unwrap_or_default()
}

#[test]
fn apply_create_update_delete_moves_composite_rows() {
    let label = intern("cpi.maintenance.label").unwrap();
    let ts = intern("cpi.maintenance.ts").unwrap();
    let location = intern("cpi.maintenance.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    let properties = smallvec![ts, location];
    insert_entry(
        &mut indexes,
        label,
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let old_props = property_map([
        (ts, Value::Int(1)),
        (location, Value::String(intern("north").unwrap())),
    ]);
    let new_props = property_map([
        (ts, Value::Int(2)),
        (location, Value::String(intern("north").unwrap())),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label), &old_props, 3).unwrap();
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(1), Value::String(intern("north").unwrap())]
        )
        .contains(3)
    );

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &old_props,
        &LabelSet::single(label),
        &new_props,
        3,
    )
    .unwrap();
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(1), Value::String(intern("north").unwrap())]
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(2), Value::String(intern("north").unwrap())]
        )
        .contains(3)
    );

    apply_node_delete(&mut indexes, &LabelSet::single(label), &new_props, 3).unwrap();
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(2), Value::String(intern("north").unwrap())]
        )
        .is_empty()
    );
}

#[test]
fn apply_create_skips_partial_composite_values() {
    let label = intern("cpi.partial.label").unwrap();
    let ts = intern("cpi.partial.ts").unwrap();
    let location = intern("cpi.partial.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    insert_entry(
        &mut indexes,
        label,
        smallvec![ts, location],
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let props = property_map([(ts, Value::Int(1))]);

    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 0).unwrap();

    let entry = indexes
        .values()
        .next()
        .expect("composite registration remains");
    assert_eq!(entry.index.cardinality(), 0);
}

#[test]
fn apply_update_label_remove_deletes_composite_row() {
    let label = intern("cpi.label-remove.label").unwrap();
    let ts = intern("cpi.label-remove.ts").unwrap();
    let location = intern("cpi.label-remove.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    let properties = smallvec![ts, location];
    insert_entry(
        &mut indexes,
        label,
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let props = property_map([
        (ts, Value::Int(1)),
        (location, Value::String(intern("north").unwrap())),
    ]);
    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 8).unwrap();

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label),
        &props,
        &LabelSet::new(),
        &props,
        8,
    )
    .unwrap();

    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(1), Value::String(intern("north").unwrap())]
        )
        .is_empty()
    );
}

#[test]
fn rebuild_composite_property_indexes_is_lenient_on_kind_drift() {
    let label = intern("cpi.rebuild.label").unwrap();
    let ts = intern("cpi.rebuild.ts").unwrap();
    let location = intern("cpi.rebuild.location").unwrap();
    let mut graph = crate::SeleneGraph::new(selene_core::GraphId::new(1));
    graph.node_store.labels.push(LabelSet::single(label));
    graph.node_store.properties.push(property_map([
        (ts, Value::Int(1)),
        (location, Value::String(intern("north").unwrap())),
    ]));
    graph.node_store.alive.insert(0);
    graph.node_store.labels.push(LabelSet::single(label));
    graph.node_store.properties.push(property_map([
        (ts, Value::String(intern("wrong").unwrap())),
        (location, Value::String(intern("south").unwrap())),
    ]));
    graph.node_store.alive.insert(1);
    graph.composite_property_index.insert(
        (label, composite_property_key(&[ts, location])),
        entry(
            smallvec![ts, location],
            smallvec![TypedIndexKind::I64, TypedIndexKind::String],
        ),
    );

    rebuild_composite_property_indexes(&mut graph).unwrap();

    let rows = rows(
        &graph.composite_property_index,
        label,
        &[ts, location],
        &[Value::Int(1), Value::String(intern("north").unwrap())],
    );
    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![0]);
}

#[test]
fn apply_create_admits_external_string_string_component() {
    // BRIEF-153 bar 4: composite (I64, STRING) admits a row whose STRING
    // component arrives as `Value::ExternalString`.
    let label = intern("cpi.external.create.label").unwrap();
    let ts = intern("cpi.external.create.ts").unwrap();
    let location = intern("cpi.external.create.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    let properties = smallvec![ts, location];
    insert_entry(
        &mut indexes,
        label,
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let probe = "cpi.external.create.unique-1";
    assert!(lookup(probe).is_none());
    let props = property_map([
        (ts, Value::Int(42)),
        (location, Value::ExternalString(Arc::<str>::from(probe))),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 4).unwrap();

    let admitted = lookup(probe).expect("admission lands");
    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(42), Value::String(admitted)]
        )
        .contains(4)
    );
}

#[test]
fn composite_index_rejection_promotes_admission_failed() {
    // BRIEF-153 bar 3 (synthetic, composite): the commit-path helper
    // promotes ComponentAdmissionFailed → IndexAdmissionExhausted with
    // the IStr-pool source intact.
    let label = intern("cpi.admit-fail.label").unwrap();
    let ts = intern("cpi.admit-fail.ts").unwrap();
    let location = intern("cpi.admit-fail.location").unwrap();
    let properties: Vec<IStr> = vec![ts, location];
    let synthetic = CompositeIndexValueError::ComponentAdmissionFailed {
        index: 1,
        expected_kind: TypedIndexKind::String,
        reason: CoreError::IStrCapExceeded {
            count: 1_000_000,
            max: 1_000_000,
        },
    };

    let promoted = index_rejection(label, &properties, synthetic);

    let GraphError::IndexAdmissionExhausted {
        label: err_label,
        property: err_property,
        source,
    } = &promoted
    else {
        panic!("expected IndexAdmissionExhausted, got {promoted:?}");
    };
    assert_eq!(*err_label, label);
    assert_eq!(*err_property, location);
    assert!(matches!(source, CoreError::IStrCapExceeded { .. }));
    assert_eq!(promoted.gqlstatus(), "5GQL1");
}

#[test]
fn composite_lookup_does_not_admit_unpoolable_string_probe() {
    // BRIEF-153 bar 10 (composite read-path): registering a composite
    // index and probing via `key_from_values_lookup` with a fresh
    // `Value::ExternalString` returns `Ok(None)` and never admits the
    // probe content into the IStr pool. (The GQL composite-scan path
    // in `scan.rs::composite_lookup_rows` calls the same helper, so this
    // exercises the same admission boundary.)
    let label = intern("cpi.lookup-no-admit.label").unwrap();
    let ts = intern("cpi.lookup-no-admit.ts").unwrap();
    let location = intern("cpi.lookup-no-admit.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    let properties = smallvec![ts, location];
    insert_entry(
        &mut indexes,
        label,
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    // Populate with String-bound row so the index is non-empty.
    let north = intern("north").unwrap();
    let props = property_map([(ts, Value::Int(1)), (location, Value::String(north))]);
    apply_node_create(&mut indexes, &LabelSet::single(label), &props, 0).unwrap();

    let probe_content = "cpi.lookup-no-admit.unique-not-admitted";
    assert!(lookup(probe_content).is_none());
    let probe_ts = Value::Int(1);
    let probe_loc = Value::ExternalString(Arc::<str>::from(probe_content));
    let refs: Vec<&Value> = vec![&probe_ts, &probe_loc];

    let entry = indexes
        .get(&(label, composite_property_key(&properties)))
        .unwrap();
    let result = entry
        .index
        .key_from_values_lookup(&refs)
        .expect("kind matches");

    assert!(result.is_none());
    assert!(
        lookup(probe_content).is_none(),
        "composite lookup must not admit the probe content"
    );
}
