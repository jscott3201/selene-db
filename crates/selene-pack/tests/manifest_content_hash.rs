//! Canonical manifest content-hash tests.

mod common;

use common::manifest_fixture::{
    build_default_manifest, build_manifest_value_with_canonical_hash,
    build_manifest_with_canonical_hash, bytes_with_canonical_hash, default_procedure,
    expected_hash_for_manifest, parse_value,
};
use selene_pack::{
    ContentHash, Gate, ManifestError, ProcedurePackManifest, ProcedurePackRegistry,
    ProcedureRegistry, parse_manifest,
};
use serde_json::{Value, json};

const SENTINEL: &str = "blake3:0000000000000000000000000000000000000000000000000000000000000000";

fn declared_hash(value: &Value) -> &str {
    value["content_hash"]
        .as_str()
        .expect("content_hash is string")
}

fn value_with_declared_content_hash(hash: &str) -> Value {
    let mut value = build_manifest_value_with_canonical_hash("demo_pack");
    value["content_hash"] = json!(hash);
    value
}

fn hash_manifest_value(value: Value) -> ContentHash {
    let mut value = value;
    value["content_hash"] = json!(SENTINEL);
    let manifest =
        serde_json::from_value::<ProcedurePackManifest>(value).expect("test manifest deserializes");
    ContentHash::from_validated_manifest(&manifest)
}

fn wrong_blake3_hash() -> String {
    format!("blake3:{}", "1".repeat(64))
}

#[test]
fn canonical_hash_round_trips() {
    let value = build_manifest_value_with_canonical_hash("demo_pack");

    let manifest = parse_value(value).expect("canonical hash validates");

    assert_eq!(manifest.content_hash, expected_hash_for_manifest(&manifest));
}

#[test]
fn mismatch_declared_vs_computed_rejected() {
    let err = parse_value(value_with_declared_content_hash(&wrong_blake3_hash()))
        .expect_err("mismatched hash fails");

    assert!(matches!(
        err,
        ManifestError::ContentHashMismatch { ref declared, ref computed }
            if declared.as_str() == wrong_blake3_hash() && computed.starts_with("blake3:")
    ));
    assert_eq!(err.gate(), Gate::ContentHashConsistency);
}

#[test]
fn prefix_not_blake3_rejected() {
    let err = parse_value(value_with_declared_content_hash(
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ))
    .expect_err("wrong prefix fails");

    assert!(matches!(
        err,
        ManifestError::InvalidContentHashFormat { .. }
    ));
    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn wrong_hex_length_rejected() {
    for len in [63, 65] {
        let err = parse_value(value_with_declared_content_hash(&format!(
            "blake3:{}",
            "0".repeat(len)
        )))
        .expect_err("wrong hex length fails");

        assert!(matches!(
            err,
            ManifestError::InvalidContentHashFormat { .. }
        ));
        assert_eq!(err.gate(), Gate::ContentHashCanonical);
    }
}

#[test]
fn non_hex_chars_rejected() {
    let err = parse_value(value_with_declared_content_hash(&format!(
        "blake3:{}",
        "z".repeat(64)
    )))
    .expect_err("non-hex chars fail");

    assert!(matches!(
        err,
        ManifestError::InvalidContentHashFormat { .. }
    ));
    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn missing_colon_rejected() {
    let err = parse_value(value_with_declared_content_hash(&format!(
        "blake3{}",
        "0".repeat(64)
    )))
    .expect_err("missing colon fails");

    assert!(matches!(
        err,
        ManifestError::InvalidContentHashFormat { .. }
    ));
    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn canonical_form_is_keys_sorted_no_whitespace() {
    let compact = br#"{"schema_version":1,"pack_name":"demo_pack","pack_version":"1.2.3","content_hash":"blake3:0000000000000000000000000000000000000000000000000000000000000000","procedures":[{"name":"demo_pack.echo","tier":"graph","mutability":"read","input_schema":{"inline":{"type":"object"}},"output_schema":{"inline":{"type":"object"}},"capability_required":null}]}"#;
    let spaced = br#"{
        "procedures": [{
            "output_schema": { "inline": { "type": "object" } },
            "capability_required": null,
            "mutability": "read",
            "input_schema": { "inline": { "type": "object" } },
            "tier": "graph",
            "name": "demo_pack.echo"
        }],
        "content_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
        "pack_version": "1.2.3",
        "pack_name": "demo_pack",
        "schema_version": 1
    }"#;

    let compact_manifest =
        serde_json::from_slice::<ProcedurePackManifest>(compact).expect("compact deserializes");
    let spaced_manifest =
        serde_json::from_slice::<ProcedurePackManifest>(spaced).expect("spaced deserializes");

    assert_eq!(
        ContentHash::from_validated_manifest(&compact_manifest),
        ContentHash::from_validated_manifest(&spaced_manifest)
    );
}

#[test]
fn canonical_form_uses_sentinel_content_hash() {
    let mut first = build_default_manifest("demo_pack");
    let mut second = first.clone();
    first.content_hash = wrong_blake3_hash();
    second.content_hash = format!("blake3:{}", "2".repeat(64));

    assert_eq!(
        ContentHash::from_validated_manifest(&first),
        ContentHash::from_validated_manifest(&second)
    );
}

#[test]
fn procedure_order_affects_hash() {
    let first = default_procedure("demo_pack");
    let mut second = default_procedure("demo_pack");
    second.name = "demo_pack.other".to_owned();
    let left = build_manifest_with_canonical_hash(
        "demo_pack",
        "1.2.3",
        vec![first.clone(), second.clone()],
    );
    let right = build_manifest_with_canonical_hash("demo_pack", "1.2.3", vec![second, first]);

    assert_ne!(
        ContentHash::from_validated_manifest(&left),
        ContentHash::from_validated_manifest(&right)
    );
}

#[test]
fn pack_name_change_changes_hash() {
    let left = build_default_manifest("demo_pack");
    let right = build_default_manifest("other_pack");

    assert_ne!(
        ContentHash::from_validated_manifest(&left),
        ContentHash::from_validated_manifest(&right)
    );
}

#[test]
fn placeholder_string_rejected_with_canonical_gate() {
    let err = parse_value(value_with_declared_content_hash(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    ))
    .expect_err("placeholder rejected");

    assert!(matches!(
        err,
        ManifestError::InvalidContentHashFormat { .. }
    ));
    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn gate_mapping_canonical_format_to_canonical_gate() {
    let err = ManifestError::InvalidContentHashFormat {
        content_hash: "not-a-hash".to_owned(),
    };

    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn gate_mapping_mismatch_to_consistency_gate() {
    let err = ManifestError::ContentHashMismatch {
        declared: wrong_blake3_hash(),
        computed: format!("blake3:{}", "2".repeat(64)),
    };

    assert_eq!(err.gate(), Gate::ContentHashConsistency);
}

#[test]
fn canonical_form_is_alphabetical_at_every_object_level() {
    let first = json!({
        "schema_version": 1,
        "pack_name": "demo_pack",
        "pack_version": "1.2.3",
        "content_hash": SENTINEL,
        "procedures": [{
            "name": "demo_pack.echo",
            "tier": "graph",
            "mutability": "read",
            "input_schema": { "inline": { "z": true, "a": false, "type": "object" } },
            "output_schema": { "inline": { "type": "object", "properties": { "b": { "type": "string" }, "a": { "type": "integer" } } } },
            "capability_required": null
        }]
    });
    let second = json!({
        "procedures": [{
            "capability_required": null,
            "output_schema": { "inline": { "properties": { "a": { "type": "integer" }, "b": { "type": "string" } }, "type": "object" } },
            "input_schema": { "inline": { "type": "object", "a": false, "z": true } },
            "mutability": "read",
            "tier": "graph",
            "name": "demo_pack.echo"
        }],
        "content_hash": SENTINEL,
        "pack_version": "1.2.3",
        "pack_name": "demo_pack",
        "schema_version": 1
    });

    assert_eq!(hash_manifest_value(first), hash_manifest_value(second));
}

#[test]
fn bad_procedure_entry_surfaces_specific_error_not_content_hash_mismatch() {
    let mut value = value_with_declared_content_hash(&wrong_blake3_hash());
    value["procedures"][0]["capability_required"] = json!("BadCapability");

    let err = parse_value(value).expect_err("procedure error wins before hash mismatch");

    assert!(matches!(
        err,
        ManifestError::ProcedureCapabilityMalformed { .. }
    ));
    assert_eq!(err.gate(), Gate::ProcedureCapabilityFormat);
}

#[test]
fn uppercase_hex_rejected_as_noncanonical() {
    let err = parse_value(value_with_declared_content_hash(&format!(
        "blake3:{}",
        "A".repeat(64)
    )))
    .expect_err("uppercase hex fails");

    assert!(matches!(
        err,
        ManifestError::InvalidContentHashFormat { .. }
    ));
    assert_eq!(err.gate(), Gate::ContentHashCanonical);
}

#[test]
fn platform_builtins_still_register_after_hash_migration() {
    let registry = ProcedurePackRegistry::with_builtins().expect("built-ins register");
    let names = [
        ["selene", "health"].as_slice(),
        ["selene", "create_index"].as_slice(),
        ["selene", "drop_index"].as_slice(),
    ];

    for name in names {
        let interned = name
            .iter()
            .map(|part| selene_core::intern(part).expect("test name interns"))
            .collect::<Vec<_>>();
        assert!(registry.lookup(&interned).is_some());
    }
}

#[test]
fn build_manifest_with_canonical_hash_round_trips_through_parser() {
    let bytes = bytes_with_canonical_hash(build_manifest_value_with_canonical_hash("demo_pack"));

    parse_manifest(&bytes).expect("helper output parses");
}

#[test]
fn omitted_option_field_and_null_option_field_hash_identically() {
    let mut omitted = build_manifest_value_with_canonical_hash("demo_pack");
    omitted["procedures"][0]
        .as_object_mut()
        .expect("procedure object")
        .remove("capability_required");
    let explicit_null = build_manifest_value_with_canonical_hash("demo_pack");

    assert_eq!(
        hash_manifest_value(omitted),
        hash_manifest_value(explicit_null)
    );
}

#[test]
fn declared_hash_is_lowercase_blake3_prefixed_hex() {
    let value = build_manifest_value_with_canonical_hash("demo_pack");
    let hash = declared_hash(&value);

    assert!(hash.starts_with("blake3:"));
    assert_eq!(hash.len(), "blake3:".len() + 64);
    assert!(
        hash["blake3:".len()..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
}
