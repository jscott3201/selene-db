use roaring::RoaringBitmap;
use selene_core::{IStr, LabelSet, PropertyMap, Value, intern};
use smallvec::{SmallVec, smallvec};

use super::*;
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
    let properties = smallvec![ts.clone(), location.clone()];
    insert_entry(
        &mut indexes,
        label.clone(),
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let old_props = property_map([
        (ts.clone(), Value::Int(1)),
        (location.clone(), Value::String(intern("north").unwrap())),
    ]);
    let new_props = property_map([
        (ts, Value::Int(2)),
        (location, Value::String(intern("north").unwrap())),
    ]);

    apply_node_create(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        3,
    )
    .unwrap();
    assert!(
        rows(
            &indexes,
            label.clone(),
            &properties,
            &[Value::Int(1), Value::String(intern("north").unwrap())]
        )
        .contains(3)
    );

    apply_node_update(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &old_props,
        &LabelSet::single(label.clone()),
        &new_props,
        3,
    )
    .unwrap();
    assert!(
        rows(
            &indexes,
            label.clone(),
            &properties,
            &[Value::Int(1), Value::String(intern("north").unwrap())]
        )
        .is_empty()
    );
    assert!(
        rows(
            &indexes,
            label.clone(),
            &properties,
            &[Value::Int(2), Value::String(intern("north").unwrap())]
        )
        .contains(3)
    );

    apply_node_delete(
        &mut indexes,
        &LabelSet::single(label.clone()),
        &new_props,
        3,
    )
    .unwrap();
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
        label.clone(),
        smallvec![ts.clone(), location],
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
    let properties = smallvec![ts.clone(), location.clone()];
    insert_entry(
        &mut indexes,
        label.clone(),
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let props = property_map([
        (ts, Value::Int(1)),
        (location, Value::String(intern("north").unwrap())),
    ]);
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
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([
        (ts.clone(), Value::Int(1)),
        (location.clone(), Value::String(intern("north").unwrap())),
    ]));
    graph.node_store.alive.insert(0);
    graph
        .node_store
        .labels
        .push(LabelSet::single(label.clone()));
    graph.node_store.properties.push(property_map([
        (ts.clone(), Value::String(intern("wrong").unwrap())),
        (location.clone(), Value::String(intern("south").unwrap())),
    ]));
    graph.node_store.alive.insert(1);
    graph.composite_property_index.insert(
        (
            label.clone(),
            composite_property_key(&[ts.clone(), location.clone()]),
        ),
        entry(
            smallvec![ts.clone(), location.clone()],
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
fn apply_create_admits_string_string_component() {
    // Composite (I64, STRING) admits a row whose STRING component is a
    // `Value::String`, and the row is findable by the same key.
    let label = intern("cpi.string.create.label").unwrap();
    let ts = intern("cpi.string.create.ts").unwrap();
    let location = intern("cpi.string.create.location").unwrap();
    let mut indexes = CompositeIndexMap::default();
    let properties = smallvec![ts.clone(), location.clone()];
    insert_entry(
        &mut indexes,
        label.clone(),
        properties.clone(),
        smallvec![TypedIndexKind::I64, TypedIndexKind::String],
    );
    let probe = intern("cpi.string.create.unique-1").unwrap();
    let props = property_map([
        (ts, Value::Int(42)),
        (location, Value::String(probe.clone())),
    ]);

    apply_node_create(&mut indexes, &LabelSet::single(label.clone()), &props, 4).unwrap();

    assert!(
        rows(
            &indexes,
            label,
            &properties,
            &[Value::Int(42), Value::String(probe)]
        )
        .contains(4)
    );
}
