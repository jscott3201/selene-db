use selene_core::{GraphId, db_string};

use crate::{
    ProviderError, SeleneGraph, TextIndex,
    core_provider::sections::{
        TextSchemaEntry, TextSchemaKey, decode_text_schemas, encode_text_schemas,
    },
    graph::TextIndexEntry,
};

#[test]
fn round_trip_preserves_text_index_name() {
    let label = db_string("core.tidx.label").unwrap();
    let property = db_string("core.tidx.property").unwrap();
    let name = db_string("core.tidx.index").unwrap();
    let mut graph = SeleneGraph::new(GraphId::new(9992));
    graph.text_index.insert(
        (label.clone(), property.clone()),
        TextIndexEntry::new(
            TextIndex::empty(label.clone(), property.clone()),
            Some(name.clone()),
        ),
    );

    let decoded = decode_text_schemas(&encode_text_schemas(&graph).unwrap()).unwrap();

    assert_eq!(
        decoded,
        vec![(
            TextSchemaKey { label, property },
            TextSchemaEntry { name: Some(name) }
        )]
    );
}

#[test]
fn decode_rejects_duplicate_text_registration() {
    let label = db_string("core.tidx.dup.label").unwrap();
    let property = db_string("core.tidx.dup.property").unwrap();
    let duplicate = vec![
        (
            TextSchemaKey {
                label: label.clone(),
                property: property.clone(),
            },
            TextSchemaEntry { name: None },
        ),
        (
            TextSchemaKey { label, property },
            TextSchemaEntry { name: None },
        ),
    ];
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&duplicate)
        .unwrap()
        .into_vec();

    let result = decode_text_schemas(&bytes);

    assert!(
        matches!(result, Err(ProviderError::InvalidPayload { reason }) if reason.contains("CORE/TIDX")),
        "duplicate CORE/TIDX key must be rejected"
    );
}
