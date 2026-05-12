//! Integration smoke tests for the BRIEF-57 selene-vector scaffold.

use std::sync::Arc;

use selene_core::{Change, GraphId, LabelSet, NodeId, PropertyMap};
use selene_graph::{GraphError, IndexProvider, ProviderError, ProviderTag, SharedGraph, SubTag};
use selene_pack::{ContentHash, ProcedurePackManifest, parse_manifest};
use selene_vector::{DistanceMetric, HnswConfig, HnswProvider, VectorError, pack_manifest};
use serde_json::{Value, json};

fn config() -> HnswConfig {
    HnswConfig::new(8).expect("test config is valid")
}

fn provider() -> HnswProvider {
    HnswProvider::new(config())
}

#[test]
fn registers_with_shared_graph() {
    let graph = SharedGraph::builder(GraphId::new(1))
        .with_provider(Arc::new(provider()))
        .build()
        .expect("provider registration succeeds");
    let registered = graph
        .index_provider_by_tag(ProviderTag(*b"VECT"))
        .expect("VECT provider is registered");
    assert_eq!(registered.provider_tag(), ProviderTag(*b"VECT"));
}

#[test]
fn declared_sub_tags_are_grph_vecs_qunt() {
    assert_eq!(
        provider().declared_sub_tags(),
        &[SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")]
    );
}

#[test]
fn tag_collision_fails_build() {
    let result = SharedGraph::builder(GraphId::new(1))
        .with_provider(Arc::new(provider()))
        .with_provider(Arc::new(provider()))
        .build();
    assert!(matches!(
        result,
        Err(GraphError::Provider(ProviderError::Inconsistent { reason }))
            if reason.contains("duplicate provider tag VECT")
    ));
}

#[test]
fn empty_section_roundtrips_declared_sub_tags() {
    for sub_tag in [SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")] {
        assert_empty_section_roundtrip(sub_tag);
    }
}

fn assert_empty_section_roundtrip(sub_tag: SubTag) {
    let provider = provider();
    let bytes = provider
        .write_section(sub_tag)
        .expect("declared section writes empty bytes");
    assert!(bytes.is_empty());
    provider
        .read_section(sub_tag, &bytes)
        .expect("declared empty section reads");
    let err = provider
        .read_section(sub_tag, &[0x01])
        .expect_err("non-empty placeholder payload is rejected");
    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("body decoder not yet implemented")
    ));
}

#[test]
fn unknown_sub_tag_returns_invalid_payload() {
    let err = provider()
        .read_section(SubTag(*b"XXXX"), &[])
        .expect_err("unknown sub-tag is rejected");
    assert!(matches!(
        err,
        ProviderError::InvalidPayload { reason }
            if reason.contains("unknown selene-vector sub_tag XXXX")
    ));
}

#[test]
fn on_change_is_no_op() {
    let change = Change::NodeCreated {
        id: NodeId::new(1),
        labels: LabelSet::new(),
        properties: PropertyMap::new(),
    };
    provider()
        .on_change(&change)
        .expect("BRIEF-57 provider ignores changes");
}

#[test]
fn config_defaults_and_validators_match_brief() {
    let config = config();
    assert_eq!(config.dim, 8);
    assert_eq!(config.m, 16);
    assert_eq!(config.ef_construction, 200);
    assert_eq!(config.ef_search, HnswConfig::default_ef_search());
    assert_eq!(HnswConfig::default_ef_search(), 50);
    assert_eq!(config.metric, DistanceMetric::Cosine);

    let cases = [
        HnswConfig::with_params(8, 1, 200, 50, DistanceMetric::Cosine),
        HnswConfig::new(0),
        HnswConfig::new(u16::MAX as usize + 1),
        HnswConfig::with_params(8, 16, 0, 50, DistanceMetric::Cosine),
    ];
    for result in cases {
        assert!(matches!(result, Err(VectorError::InvalidConfig { .. })));
    }
}

#[test]
fn stub_manifest_round_trips_through_canonical_hash() {
    let manifest: ProcedurePackManifest =
        serde_json::from_str(pack_manifest()).expect("stub manifest shape is typed");
    assert_eq!(manifest.pack_name, "vector");
    assert!(manifest.procedures.is_empty());

    let hash = ContentHash::from_validated_manifest(&manifest);
    let mut value: Value =
        serde_json::from_str(pack_manifest()).expect("stub manifest is JSON object");
    value["content_hash"] = json!(prefixed_hash(hash.as_bytes()));
    let bytes = serde_json::to_vec(&value).expect("patched manifest serializes");

    let parsed = parse_manifest(&bytes).expect("patched manifest validates");
    assert_eq!(parsed.pack_name, "vector");
    assert!(parsed.procedures.is_empty());
}

fn prefixed_hash(hash: [u8; 32]) -> String {
    let mut out = String::from("blake3:");
    for byte in hash {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        10..=15 => char::from(b'a' + (nibble - 10)),
        _ => unreachable!("nibble has four bits"),
    }
}
