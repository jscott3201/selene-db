//! Manifest validation gate coverage tests.

use selene_pack::{
    ACTIVATION_SEAL_COVERAGE, DEFERRED_GATES, Gate, MANIFEST_LEVEL_GATES,
    MANIFEST_VALIDATION_COVERAGE, ManifestError, ManifestMutability, ManifestTier,
    PLACEHOLDER_CONTENT_HASH, PROCEDURE_LEVEL_GATES, ProcedurePackManifest, WAL_AUDIT_COVERAGE,
    parse_manifest,
};
use serde_json::{Value, json};

fn valid_manifest() -> Value {
    json!({
        "schema_version": 1,
        "pack_name": "demo_pack",
        "pack_version": "1.2.3",
        "content_hash": PLACEHOLDER_CONTENT_HASH,
        "procedures": [
            valid_procedure("demo_pack.echo")
        ]
    })
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

fn parse_value(value: Value) -> Result<ProcedurePackManifest, ManifestError> {
    let bytes = serde_json::to_vec(&value).expect("test JSON serializes");
    parse_manifest(&bytes)
}

#[test]
fn every_gate_appears_in_exactly_one_coverage_slice() {
    for gate in Gate::ALL {
        let count = usize::from(MANIFEST_VALIDATION_COVERAGE.contains(gate))
            + usize::from(ACTIVATION_SEAL_COVERAGE.contains(gate))
            + usize::from(WAL_AUDIT_COVERAGE.contains(gate))
            + usize::from(DEFERRED_GATES.contains(gate));

        assert_eq!(count, 1, "gate {} appears {count} times", gate.id());
    }
}

#[test]
fn wal_audit_coverage_contains_activation_lifecycle_atomicity_only() {
    assert_eq!(WAL_AUDIT_COVERAGE, &[Gate::ActivationLifecycleAtomicity]);
    assert!(!DEFERRED_GATES.contains(&Gate::ActivationLifecycleAtomicity));
}

#[test]
fn manifest_coverage_is_concatenation_of_sub_slices() {
    let expected = MANIFEST_LEVEL_GATES
        .iter()
        .chain(PROCEDURE_LEVEL_GATES.iter())
        .copied()
        .collect::<Vec<_>>();

    assert_eq!(MANIFEST_VALIDATION_COVERAGE, expected.as_slice());
}

#[test]
fn manifest_level_gates_run_before_procedure_loop() {
    let mut value = valid_manifest();
    let procedures = (0..=256)
        .map(|idx| valid_procedure(&format!("demo_pack.proc_{idx}")))
        .collect::<Vec<_>>();
    value["procedures"] = Value::Array(procedures);
    value["procedures"][0]["capability_required"] = json!("BadCapability");

    let err = parse_value(value).expect_err("manifest-level count gate wins");

    assert!(matches!(
        err,
        ManifestError::PackProcedureCountExceedsBound { count: 257, .. }
    ));
    assert_eq!(err.gate(), Gate::PackProcedureCountBounded);
}

#[test]
fn procedure_major_within_procedures_is_preserved() {
    let mut value = valid_manifest();
    let mut first = valid_procedure("demo_pack.first");
    first["capability_required"] = json!("BadCapability");
    let second = valid_procedure("demo_pack.bad-name");
    value["procedures"] = json!([first, second]);

    let err = parse_value(value).expect_err("procedure-major gate order wins");

    assert!(matches!(
        err,
        ManifestError::ProcedureCapabilityMalformed {
            ref procedure_name,
            ..
        }
            if procedure_name == "demo_pack.first"
    ));
    assert_eq!(err.gate(), Gate::ProcedureCapabilityFormat);
}

#[test]
fn mapping_integrity_compiles() {
    assert_eq!(
        ManifestError::InvalidJson {
            source: serde_json::from_str::<Value>("{").expect_err("invalid JSON")
        }
        .gate(),
        Gate::ManifestSyntaxAndSchema
    );
    assert_eq!(
        ManifestError::SchemaViolation {
            errors: vec!["schema".to_owned()]
        }
        .gate(),
        Gate::ManifestSyntaxAndSchema
    );
    assert_eq!(
        ManifestError::DeserializeError {
            source: serde_json::from_value::<ProcedurePackManifest>(json!(null))
                .expect_err("typed shape error")
        }
        .gate(),
        Gate::ManifestTypedShape
    );
    assert_eq!(
        ManifestError::UnsupportedSchemaVersion {
            found: 2,
            supported: 1,
        }
        .gate(),
        Gate::ManifestSchemaVersionSupported
    );
    assert_eq!(
        ManifestError::UnsupportedContentHash {
            content_hash: "sha256:bad".to_owned()
        }
        .gate(),
        Gate::ContentHashPlaceholderRecognized
    );
    assert_eq!(
        ManifestError::InvalidPackVersion {
            pack_version: "x".to_owned(),
            detail: "detail".to_owned(),
        }
        .gate(),
        Gate::PackVersionWellFormed
    );
    assert_eq!(
        ManifestError::InvalidPackName {
            pack_name: "bad name".to_owned(),
            detail: "detail",
        }
        .gate(),
        Gate::PackNameLexical
    );
    assert_eq!(
        ManifestError::InvalidProcedureName {
            procedure_name: "bad-name".to_owned(),
            detail: "detail",
        }
        .gate(),
        Gate::ProcedureNameLexical
    );
    assert_eq!(
        ManifestError::ProcedureNameOutsidePack {
            procedure_name: "other.name".to_owned(),
            expected_prefix: "demo_pack.".to_owned(),
        }
        .gate(),
        Gate::ProcedureWithinPack
    );
    assert_eq!(
        ManifestError::ReservedNamespaceConflict {
            procedure_name: "selene.health".to_owned(),
        }
        .gate(),
        Gate::ReservedNamespace
    );
    assert_eq!(
        ManifestError::DuplicateProcedureName {
            procedure_name: "demo_pack.echo".to_owned(),
        }
        .gate(),
        Gate::ProcedureNamesUnique
    );
    assert_eq!(
        ManifestError::MutabilityTierMismatch {
            procedure_name: "demo_pack.echo".to_owned(),
            declared_tier: ManifestTier::Graph,
            declared_mutability: ManifestMutability::GraphWrite,
            expected_tier: ManifestTier::Mutation,
        }
        .gate(),
        Gate::TierMutabilityConsistency
    );
    assert_eq!(
        ManifestError::PersistTierInManifest {
            procedure_name: "demo_pack.echo".to_owned(),
        }
        .gate(),
        Gate::PersistTierRejected
    );
    assert_eq!(
        ManifestError::InvalidInlineSchema {
            procedure_name: "demo_pack.echo".to_owned(),
            field: "input_schema",
            errors: vec!["schema".to_owned()],
        }
        .gate(),
        Gate::InlineSchemaMetaValid
    );
    assert_eq!(
        ManifestError::InvalidSchemaPath {
            procedure_name: "demo_pack.echo".to_owned(),
            field: "input_schema",
            path: "../x.json".to_owned(),
            detail: "detail",
        }
        .gate(),
        Gate::PathSchemaSafety
    );
    assert_eq!(
        ManifestError::PackProcedureCountExceedsBound {
            count: 257,
            limit: 256,
        }
        .gate(),
        Gate::PackProcedureCountBounded
    );
    assert_eq!(
        ManifestError::InlineSchemaSizeExceedsBound {
            procedure_name: "demo_pack.echo".to_owned(),
            field: "input_schema",
            size: 65_537,
            limit: 65_536,
        }
        .gate(),
        Gate::InlineSchemaSizeBounded
    );
    assert_eq!(
        ManifestError::ProcedureSchemaCompileFailed {
            procedure_name: "demo_pack.echo".to_owned(),
            field: "input_schema",
            errors: vec!["compile".to_owned()],
        }
        .gate(),
        Gate::ProcedureInputSchemaCompiles
    );
    assert_eq!(
        ManifestError::ProcedureSchemaCompileFailed {
            procedure_name: "demo_pack.echo".to_owned(),
            field: "output_schema",
            errors: vec!["compile".to_owned()],
        }
        .gate(),
        Gate::ProcedureOutputSchemaCompiles
    );
    assert_eq!(
        ManifestError::ProcedureCapabilityMalformed {
            procedure_name: "demo_pack.echo".to_owned(),
            capability: "BadCapability".to_owned(),
            detail: "detail",
        }
        .gate(),
        Gate::ProcedureCapabilityFormat
    );
    assert_eq!(
        ManifestError::ProcedureNameTooLong {
            procedure_name: "demo_pack.echo".to_owned(),
            length: 256,
            limit: 255,
        }
        .gate(),
        Gate::ProcedureNameLengthBounded
    );
}
