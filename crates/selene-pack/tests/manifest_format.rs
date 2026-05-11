//! Procedure-pack manifest parsing and schema tests.

use selene_pack::{
    MANIFEST_SCHEMA_DRAFT, ManifestError, ManifestMutability, ManifestSchemaRef, ManifestTier,
    ProcedurePackManifest, manifest_json_schema, parse_manifest,
};
use serde_json::{Value, json};

const PLACEHOLDER_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn valid_manifest() -> Value {
    json!({
        "schema_version": 1,
        "pack_name": "demo_pack",
        "pack_version": "1.2.3",
        "content_hash": PLACEHOLDER_HASH,
        "procedures": [
            {
                "name": "demo_pack.echo",
                "tier": "graph",
                "mutability": "read",
                "input_schema": { "inline": { "type": "object" } },
                "output_schema": { "inline": { "type": "object" } },
                "capability_required": null
            }
        ]
    })
}

fn parse_value(value: Value) -> Result<ProcedurePackManifest, ManifestError> {
    let bytes = serde_json::to_vec(&value).expect("test JSON serializes");
    parse_manifest(&bytes)
}

fn only_procedure_mut(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    value["procedures"][0]
        .as_object_mut()
        .expect("procedure is an object")
}

#[test]
fn minimal_valid_manifest_parses_and_round_trips() {
    let manifest = parse_value(valid_manifest()).expect("valid manifest parses");
    let value = serde_json::to_value(&manifest).expect("manifest serializes");
    let reparsed = serde_json::from_value::<ProcedurePackManifest>(value).expect("round-trips");

    assert_eq!(manifest, reparsed);
    assert_eq!(manifest.pack_name, "demo_pack");
    assert_eq!(manifest.procedures[0].name, "demo_pack.echo");
}

#[test]
fn manifest_json_schema_is_cache_stable() {
    let first = manifest_json_schema();
    let second = manifest_json_schema();

    assert!(std::ptr::eq(first, second));
    assert_eq!(first, second);
}

#[test]
fn generated_schema_declares_json_schema_2020_12() {
    assert_eq!(
        manifest_json_schema()
            .get("$schema")
            .and_then(Value::as_str),
        Some(MANIFEST_SCHEMA_DRAFT)
    );
}

#[test]
fn generated_schema_passes_meta_validation() {
    jsonschema::draft202012::meta::validate(manifest_json_schema())
        .expect("generated schema is valid JSON Schema 2020-12");
}

#[test]
fn unknown_top_level_field_is_schema_violation() {
    let mut value = valid_manifest();
    value["unknown_field"] = json!("x");

    let err = parse_value(value).expect_err("unknown field fails schema");

    assert!(matches!(err, ManifestError::SchemaViolation { .. }));
}

#[test]
fn missing_required_field_is_schema_violation() {
    let mut value = valid_manifest();
    value.as_object_mut().unwrap().remove("pack_name");

    let err = parse_value(value).expect_err("missing field fails schema");

    assert!(matches!(err, ManifestError::SchemaViolation { .. }));
}

#[test]
fn wrong_field_type_is_schema_violation() {
    let mut value = valid_manifest();
    value["schema_version"] = json!("one");

    let err = parse_value(value).expect_err("wrong type fails schema");

    assert!(matches!(err, ManifestError::SchemaViolation { .. }));
}

#[test]
fn invalid_json_is_invalid_json_error() {
    let err = parse_manifest(br#"{ "schema_version": 1, "#).expect_err("invalid JSON fails");

    assert!(matches!(err, ManifestError::InvalidJson { .. }));
}

#[test]
fn unsupported_schema_version_is_structural_error() {
    let mut value = valid_manifest();
    value["schema_version"] = json!(2);

    let err = parse_value(value).expect_err("unsupported version fails");

    assert!(matches!(
        err,
        ManifestError::UnsupportedSchemaVersion {
            found: 2,
            supported: 1
        }
    ));
}

#[test]
fn procedure_name_outside_pack_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("name".to_owned(), json!("other_pack.echo"));

    let err = parse_value(value).expect_err("outside-pack name fails");

    assert!(matches!(
        err,
        ManifestError::ProcedureNameOutsidePack {
            procedure_name,
            expected_prefix,
        } if procedure_name == "other_pack.echo" && expected_prefix == "demo_pack."
    ));
}

#[test]
fn reserved_selene_namespace_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("name".to_owned(), json!("selene.health"));

    let err = parse_value(value).expect_err("reserved name fails");

    assert!(matches!(
        err,
        ManifestError::ReservedNamespaceConflict { procedure_name }
            if procedure_name == "selene.health"
    ));
}

#[test]
fn selene_prefix_is_allowed_when_first_segment_differs() {
    let mut value = valid_manifest();
    value["pack_name"] = json!("selene_extensions");
    only_procedure_mut(&mut value).insert("name".to_owned(), json!("selene_extensions.health"));

    let manifest = parse_value(value).expect("non-reserved selene prefix parses");

    assert_eq!(manifest.pack_name, "selene_extensions");
    assert_eq!(manifest.procedures[0].name, "selene_extensions.health");
}

#[test]
fn duplicate_procedure_names_are_rejected() {
    let mut value = valid_manifest();
    let duplicate = value["procedures"][0].clone();
    value["procedures"].as_array_mut().unwrap().push(duplicate);

    let err = parse_value(value).expect_err("duplicate procedure fails");

    assert!(matches!(
        err,
        ManifestError::DuplicateProcedureName { procedure_name }
            if procedure_name == "demo_pack.echo"
    ));
}

#[test]
fn persist_tier_is_rejected_at_manifest_layer() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("tier".to_owned(), json!("persist"));

    let err = parse_value(value).expect_err("persist tier fails");

    assert!(matches!(
        err,
        ManifestError::PersistTierInManifest { procedure_name }
            if procedure_name == "demo_pack.echo"
    ));
}

#[test]
fn inline_schema_ref_parses_and_round_trips() {
    let manifest = parse_value(valid_manifest()).expect("valid manifest parses");

    assert!(matches!(
        &manifest.procedures[0].input_schema,
        ManifestSchemaRef::Inline(schema) if schema == &json!({ "type": "object" })
    ));

    let value = serde_json::to_value(&manifest).expect("manifest serializes");
    assert_eq!(
        value["procedures"][0]["input_schema"],
        json!({ "inline": { "type": "object" } })
    );
}

#[test]
fn path_schema_ref_parses_without_checking_existence() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "path": { "relative_to": "schemas/input.json" } }),
    );

    let manifest = parse_value(value).expect("path schema ref parses");

    assert!(matches!(
        &manifest.procedures[0].input_schema,
        ManifestSchemaRef::Path { relative_to } if relative_to == "schemas/input.json"
    ));
}

#[test]
fn capability_declaration_parses_when_present() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("capability_required".to_owned(), json!("read:journal"));

    let manifest = parse_value(value).expect("capability declaration parses");

    assert_eq!(
        manifest.procedures[0].capability_required.as_deref(),
        Some("read:journal")
    );
}

#[test]
fn capability_declaration_is_optional() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).remove("capability_required");

    let manifest = parse_value(value).expect("absent capability parses");

    assert_eq!(manifest.procedures[0].capability_required, None);
}

#[test]
fn non_placeholder_content_hash_is_rejected() {
    let mut value = valid_manifest();
    value["content_hash"] =
        json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");

    let err = parse_value(value).expect_err("non-placeholder hash fails");

    assert!(matches!(
        err,
        ManifestError::UnsupportedContentHash { content_hash }
            if content_hash.starts_with("sha256:ffff")
    ));
}

#[test]
fn invalid_pack_version_is_rejected() {
    let mut value = valid_manifest();
    value["pack_version"] = json!("not-semver");

    let err = parse_value(value).expect_err("invalid semver fails");

    assert!(matches!(
        err,
        ManifestError::InvalidPackVersion { pack_version, .. } if pack_version == "not-semver"
    ));
}

#[test]
fn invalid_pack_name_is_rejected() {
    let mut value = valid_manifest();
    value["pack_name"] = json!("demo pack");

    let err = parse_value(value).expect_err("invalid pack name fails");

    assert!(matches!(
        err,
        ManifestError::InvalidPackName { pack_name, .. } if pack_name == "demo pack"
    ));
}

#[test]
fn one_segment_procedure_name_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("name".to_owned(), json!("echo"));

    let err = parse_value(value).expect_err("one-segment procedure name fails");

    assert!(matches!(
        err,
        ManifestError::InvalidProcedureName { procedure_name, .. } if procedure_name == "echo"
    ));
}

#[test]
fn tier_mutability_mismatch_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert("mutability".to_owned(), json!("graph_write"));

    let err = parse_value(value).expect_err("tier/mutability mismatch fails");

    assert!(matches!(
        err,
        ManifestError::MutabilityTierMismatch {
            procedure_name,
            declared_tier: ManifestTier::Graph,
            declared_mutability: ManifestMutability::GraphWrite,
            expected_tier: ManifestTier::Mutation,
        } if procedure_name == "demo_pack.echo"
    ));
}

#[test]
fn malformed_inline_schema_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "inline": { "type": "definitely_not_a_json_schema_type" } }),
    );

    let err = parse_value(value).expect_err("invalid inline schema fails");

    assert!(matches!(
        err,
        ManifestError::InvalidInlineSchema {
            procedure_name,
            field: "input_schema",
            ..
        } if procedure_name == "demo_pack.echo"
    ));
}

#[test]
fn parent_directory_schema_path_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "input_schema".to_owned(),
        json!({ "path": { "relative_to": "../escape.json" } }),
    );

    let err = parse_value(value).expect_err("parent path fails");

    assert!(matches!(
        err,
        ManifestError::InvalidSchemaPath {
            procedure_name,
            field: "input_schema",
            path,
            ..
        } if procedure_name == "demo_pack.echo" && path == "../escape.json"
    ));
}

#[test]
fn absolute_schema_path_is_rejected() {
    let mut value = valid_manifest();
    only_procedure_mut(&mut value).insert(
        "output_schema".to_owned(),
        json!({ "path": { "relative_to": "/tmp/schema.json" } }),
    );

    let err = parse_value(value).expect_err("absolute path fails");

    assert!(matches!(
        err,
        ManifestError::InvalidSchemaPath {
            field: "output_schema",
            path,
            ..
        } if path == "/tmp/schema.json"
    ));
}

#[test]
fn unsafe_schema_paths_are_rejected() {
    for path in [
        "",
        r"C:\schemas\input.json",
        r"\\server\share\input.json",
        "schemas//input.json",
    ] {
        let mut value = valid_manifest();
        only_procedure_mut(&mut value).insert(
            "input_schema".to_owned(),
            json!({ "path": { "relative_to": path } }),
        );

        let err = parse_value(value).expect_err("unsafe path fails");

        assert!(matches!(
            err,
            ManifestError::InvalidSchemaPath {
                field: "input_schema",
                path: rejected,
                ..
            } if rejected == path
        ));
    }
}
