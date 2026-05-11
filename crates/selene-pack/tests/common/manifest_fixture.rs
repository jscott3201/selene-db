//! Procedure-pack manifest fixtures with canonical content hashes.

use selene_pack::{
    ContentHash, ManifestMutability, ManifestProcedureEntry, ManifestSchemaRef, ManifestTier,
    ProcedurePackManifest, SCHEMA_VERSION_SUPPORTED, parse_manifest,
};
use serde_json::{Value, json};

const BLAKE3_HASH_SENTINEL: &str =
    "blake3:0000000000000000000000000000000000000000000000000000000000000000";

pub fn default_procedure(pack_name: &str) -> ManifestProcedureEntry {
    ManifestProcedureEntry {
        name: format!("{pack_name}.echo"),
        tier: ManifestTier::Graph,
        mutability: ManifestMutability::Read,
        input_schema: ManifestSchemaRef::Inline(json!({ "type": "object" })),
        output_schema: ManifestSchemaRef::Inline(json!({ "type": "object" })),
        capability_required: None,
    }
}

pub fn build_manifest_with_canonical_hash(
    pack_name: &str,
    pack_version: &str,
    procedures: Vec<ManifestProcedureEntry>,
) -> ProcedurePackManifest {
    let mut manifest = ProcedurePackManifest {
        schema_version: SCHEMA_VERSION_SUPPORTED,
        pack_name: pack_name.to_owned(),
        pack_version: pack_version.to_owned(),
        content_hash: BLAKE3_HASH_SENTINEL.to_owned(),
        procedures,
    };
    let hash = ContentHash::from_validated_manifest(&manifest);
    manifest.content_hash = prefixed_hash(&hash.as_bytes());
    manifest
}

pub fn build_default_manifest(pack_name: &str) -> ProcedurePackManifest {
    build_manifest_with_canonical_hash(pack_name, "1.2.3", vec![default_procedure(pack_name)])
}

pub fn build_manifest_json_with_canonical_hash(pack_name: &str) -> Vec<u8> {
    serde_json::to_vec(&build_default_manifest(pack_name)).expect("test manifest serializes")
}

pub fn build_manifest_value_with_canonical_hash(pack_name: &str) -> Value {
    serde_json::to_value(build_default_manifest(pack_name)).expect("test manifest serializes")
}

pub fn value_with_canonical_hash(mut value: Value) -> Value {
    value["content_hash"] = json!(BLAKE3_HASH_SENTINEL);
    let manifest = serde_json::from_value::<ProcedurePackManifest>(value.clone())
        .expect("test manifest deserializes");
    let hash = ContentHash::from_validated_manifest(&manifest);
    value["content_hash"] = json!(prefixed_hash(&hash.as_bytes()));
    value
}

pub fn bytes_with_canonical_hash(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value_with_canonical_hash(value)).expect("test manifest serializes")
}

pub fn parse_value(value: Value) -> Result<ProcedurePackManifest, selene_pack::ManifestError> {
    let bytes = serde_json::to_vec(&value).expect("test JSON serializes");
    parse_manifest(&bytes)
}

pub fn expected_hash_for_manifest(manifest: &ProcedurePackManifest) -> String {
    let hash = ContentHash::from_validated_manifest(manifest);
    prefixed_hash(&hash.as_bytes())
}

pub fn prefixed_hash(hash: &[u8; 32]) -> String {
    let mut out = String::with_capacity("blake3:".len() + hash.len() * 2);
    out.push_str("blake3:");
    out.push_str(&hex_lower(hash));
    out
}

pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
