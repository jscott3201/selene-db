use selene_core::{GraphId, db_string};
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
    let label = db_string("core.cpix.sensor").unwrap();
    let ts = db_string("ts").unwrap();
    let location = db_string("location").unwrap();
    let name = db_string("sensor_ts_location_idx").unwrap();
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
    let label = db_string("core.cpix.dup").unwrap();
    let left = db_string("left").unwrap();
    let right = db_string("right").unwrap();
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
                name: Some(db_string("dup_idx").unwrap()),
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
    let label = db_string("core.cpix.single").unwrap();
    let only = db_string("only").unwrap();
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
