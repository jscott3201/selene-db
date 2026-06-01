use selene_core::{GraphId, intern};
use smallvec::smallvec;

use crate::graph::CompositePropertyIndexEntry;
use crate::{
    CompositeTypedIndex, SeleneGraph, TypedIndexKind,
    core_provider::sections::{
        CompositeSchemaEntry, CompositeSchemaKey, decode_composite_schemas,
        encode_composite_schemas,
    },
};

#[test]
fn round_trip_preserves_composite_registration_metadata() {
    let label = intern("core.cpix.sensor").unwrap();
    let ts = intern("ts").unwrap();
    let location = intern("location").unwrap();
    let name = intern("sensor_ts_location_idx").unwrap();
    let declared_properties = smallvec![ts.clone(), location.clone()];
    let kinds = smallvec![TypedIndexKind::LocalDateTime, TypedIndexKind::String];
    let mut graph = SeleneGraph::new(GraphId::new(9992));
    graph.composite_property_index.insert(
        (
            label.clone(),
            crate::graph::composite_property_key(&declared_properties),
        ),
        CompositePropertyIndexEntry::new(
            CompositeTypedIndex::new(kinds.clone()),
            declared_properties,
            Some(name.clone()),
        ),
    );

    let decoded = decode_composite_schemas(&encode_composite_schemas(&graph).unwrap()).unwrap();

    assert_eq!(
        decoded,
        vec![(
            CompositeSchemaKey {
                label,
                properties: vec![ts, location],
            },
            CompositeSchemaEntry {
                kinds: kinds.into_iter().collect(),
                name: Some(name),
            }
        )]
    );
}

#[test]
fn decode_rejects_duplicate_canonical_property_sets() {
    let label = intern("core.cpix.dup").unwrap();
    let left = intern("left").unwrap();
    let right = intern("right").unwrap();
    let rows = vec![
        (
            CompositeSchemaKey {
                label: label.clone(),
                properties: vec![left.clone(), right.clone()],
            },
            CompositeSchemaEntry {
                kinds: vec![TypedIndexKind::I64, TypedIndexKind::String],
                name: None,
            },
        ),
        (
            CompositeSchemaKey {
                label,
                properties: vec![right, left],
            },
            CompositeSchemaEntry {
                kinds: vec![TypedIndexKind::String, TypedIndexKind::I64],
                name: Some(intern("dup_idx").unwrap()),
            },
        ),
    ];
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rows)
        .unwrap()
        .into_vec();

    let result = decode_composite_schemas(&bytes);

    assert!(result.is_err());
}

#[test]
fn decode_rejects_single_property_composite_registration() {
    let label = intern("core.cpix.single").unwrap();
    let only = intern("only").unwrap();
    let rows = vec![(
        CompositeSchemaKey {
            label,
            properties: vec![only],
        },
        CompositeSchemaEntry {
            kinds: vec![TypedIndexKind::I64],
            name: None,
        },
    )];
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&rows)
        .unwrap()
        .into_vec();

    let result = decode_composite_schemas(&bytes);

    assert!(result.is_err());
}
