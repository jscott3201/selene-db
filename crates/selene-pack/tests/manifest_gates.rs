//! Manifest gate behavior tests.

mod common;

use common::manifest_fixture::{
    build_manifest_value_with_canonical_hash, parse_value, value_with_canonical_hash,
};
use selene_pack::{
    Gate, MAX_INLINE_SCHEMA_SIZE_BYTES, MAX_PROCEDURE_NAME_LENGTH, MAX_PROCEDURES_PER_PACK,
    ManifestError,
};
use serde_json::{Value, json};

fn valid_manifest() -> Value {
    build_manifest_value_with_canonical_hash("demo_pack")
}

fn valid_procedure(name: &str) -> Value {
    json!({
        "name": name,
        "tier": "graph",
        "mutability": "read",
        "input_schema": { "inline": { "type": "object" } },
        "output_schema": { "inline": { "type": "object" } },
        "capability_required": null
    })
}

fn only_procedure_mut(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    value["procedures"][0]
        .as_object_mut()
        .expect("procedure is an object")
}

fn schema_with_serialized_size(target_size: usize) -> Value {
    let base = json!({ "type": "object", "description": "" });
    let base_len = serde_json::to_vec(&base)
        .expect("test schema serializes")
        .len();
    assert!(target_size >= base_len);

    let filler = "x".repeat(target_size - base_len);
    let schema = json!({ "type": "object", "description": filler });
    assert_eq!(
        serde_json::to_vec(&schema)
            .expect("test schema serializes")
            .len(),
        target_size
    );
    schema
}

fn procedure_name_with_length(length: usize) -> String {
    let prefix = "demo_pack.";
    assert!(length > prefix.len());
    let segment_len = length - prefix.len();
    format!("{prefix}p{}", "a".repeat(segment_len - 1))
}

#[test]
fn procedure_count_at_max_succeeds() {
    let mut value = valid_manifest();
    let procedures = (0..MAX_PROCEDURES_PER_PACK)
        .map(|idx| valid_procedure(&format!("demo_pack.proc_{idx}")))
        .collect::<Vec<_>>();
    value["procedures"] = Value::Array(procedures);
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("max procedure count parses");

    assert_eq!(manifest.procedures.len(), MAX_PROCEDURES_PER_PACK);
}

#[test]
fn inline_schema_at_max_size_succeeds() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": schema_with_serialized_size(MAX_INLINE_SCHEMA_SIZE_BYTES) }),
    );
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("max inline schema size parses");

    assert_eq!(manifest.procedures.len(), 1);
}

#[test]
fn procedure_name_at_max_length_succeeds() {
    let mut value = valid_manifest();
    let name = procedure_name_with_length(MAX_PROCEDURE_NAME_LENGTH);
    only_procedure_mut(&mut value).insert("name".to_owned(), json!(name));
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("max procedure name length parses");

    assert_eq!(manifest.procedures[0].name.len(), MAX_PROCEDURE_NAME_LENGTH);
}

#[test]
fn valid_capability_format_parses() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("capability_required".to_owned(), json!("read:journal"));
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("valid capability parses");

    assert_eq!(
        manifest.procedures[0].capability_required.as_deref(),
        Some("read:journal")
    );
}

#[test]
fn valid_inline_schemas_compile() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": { "type": "string" } }),
    );
    only_procedure_mut(&mut value).insert(
        "output_schema".to_owned(),
        json!({ "inline": { "type": "integer" } }),
    );
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("valid inline schemas compile");

    assert_eq!(manifest.procedures[0].name, "demo_pack.echo");
}

#[test]
fn path_schema_skips_compile_check() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "path": { "relative_to": "schemas/input.json" } }),
    );
    only_procedure_mut(&mut value).insert(
        "output_schema".to_owned(),
        json!({ "path": { "relative_to": "schemas/output.json" } }),
    );
    let value = value_with_canonical_hash(value);

    let manifest = parse_value(value).expect("path schemas are not loaded in BRIEF-44");

    assert_eq!(manifest.procedures[0].name, "demo_pack.echo");
}

#[test]
fn procedure_count_above_max_is_rejected() {
    let mut value = valid_manifest();
    let procedures = (0..=MAX_PROCEDURES_PER_PACK)
        .map(|idx| valid_procedure(&format!("demo_pack.proc_{idx}")))
        .collect::<Vec<_>>();
    value["procedures"] = Value::Array(procedures);

    let err = parse_value(value).expect_err("too many procedures fail");

    assert!(matches!(
        err,
        ManifestError::PackProcedureCountExceedsBound { count, limit }
            if count == MAX_PROCEDURES_PER_PACK + 1 && limit == MAX_PROCEDURES_PER_PACK
    ));
    assert_eq!(err.gate(), Gate::PackProcedureCountBounded);
}

#[test]
fn inline_schema_above_max_size_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": schema_with_serialized_size(MAX_INLINE_SCHEMA_SIZE_BYTES + 1) }),
    );

    let err = parse_value(value).expect_err("oversize inline schema fails");

    assert!(matches!(
        err,
        ManifestError::InlineSchemaSizeExceedsBound {
            field: "input_schema",
            size,
            limit,
            ..
        } if size == MAX_INLINE_SCHEMA_SIZE_BYTES + 1 && limit == MAX_INLINE_SCHEMA_SIZE_BYTES
    ));
    assert_eq!(err.gate(), Gate::InlineSchemaSizeBounded);
}

#[test]
fn procedure_name_above_max_length_is_rejected() {
    let mut value = valid_manifest();
    let name = procedure_name_with_length(MAX_PROCEDURE_NAME_LENGTH + 1);
    only_procedure_mut(&mut value).insert("name".to_owned(), json!(name));

    let err = parse_value(value).expect_err("too-long procedure name fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureNameTooLong { length, limit, .. }
            if length == MAX_PROCEDURE_NAME_LENGTH + 1 && limit == MAX_PROCEDURE_NAME_LENGTH
    ));
    assert_eq!(err.gate(), Gate::ProcedureNameLengthBounded);
}

#[test]
fn malformed_capability_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("capability_required".to_owned(), json!("BadCapability"));

    let err = parse_value(value).expect_err("malformed capability fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureCapabilityMalformed { ref capability, .. }
            if capability == "BadCapability"
    ));
    assert_eq!(err.gate(), Gate::ProcedureCapabilityFormat);
}

#[test]
fn multi_colon_capability_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "capability_required".to_owned(),
        json!("read:journal:graph"),
    );

    let err = parse_value(value).expect_err("multi-colon capability fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureCapabilityMalformed { ref capability, .. }
            if capability == "read:journal:graph"
    ));
    assert_eq!(err.gate(), Gate::ProcedureCapabilityFormat);
}

#[test]
fn input_schema_compile_failure_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": { "type": "string", "pattern": "[" } }),
    );

    let err = parse_value(value).expect_err("input schema compile failure fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureSchemaCompileFailed {
            field: "input_schema",
            ..
        }
    ));
    assert_eq!(err.gate(), Gate::ProcedureInputSchemaCompiles);
}

#[test]
fn output_schema_compile_failure_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "output_schema".to_owned(),
        json!({ "inline": { "type": "string", "pattern": "[" } }),
    );

    let err = parse_value(value).expect_err("output schema compile failure fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureSchemaCompileFailed {
            field: "output_schema",
            ..
        }
    ));
    assert_eq!(err.gate(), Gate::ProcedureOutputSchemaCompiles);
}

#[test]
fn oversized_schema_short_circuits_before_meta_validation() {
    let mut value = valid_manifest();
    let invalid_large_schema = json!({
        "properties": "not-an-object",
        "description": "x".repeat(MAX_INLINE_SCHEMA_SIZE_BYTES),
    });
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": invalid_large_schema }),
    );

    let err = parse_value(value).expect_err("oversize invalid schema fails on size");

    assert!(matches!(
        err,
        ManifestError::InlineSchemaSizeExceedsBound {
            field: "input_schema",
            ..
        }
    ));
    assert_eq!(err.gate(), Gate::InlineSchemaSizeBounded);
}

#[test]
fn existing_invalid_pack_name_owned_by_pack_name_lexical() {
    let mut value = valid_manifest();
    value["pack_name"] = json!("demo pack");

    let err = parse_value(value).expect_err("invalid pack name fails");

    assert!(matches!(err, ManifestError::InvalidPackName { .. }));
    assert_eq!(err.gate(), Gate::PackNameLexical);
}

#[test]
fn existing_persist_tier_owned_by_persist_tier_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("tier".to_owned(), json!("persist"));

    let err = parse_value(value).expect_err("persist tier fails");

    assert!(matches!(err, ManifestError::PersistTierInManifest { .. }));
    assert_eq!(err.gate(), Gate::PersistTierRejected);
}
