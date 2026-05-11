//! Procedure-pack manifest parser.

use std::collections::HashSet;

use serde_json::Value;

use super::{
    ManifestError, ManifestMutability, ManifestSchemaRef, ManifestTier, PLACEHOLDER_CONTENT_HASH,
    ProcedurePackManifest, SCHEMA_VERSION_SUPPORTED, manifest_json_schema,
};

/// Parse and validate a procedure-pack manifest from JSON bytes.
///
/// # Errors
///
/// Returns [`ManifestError`] for syntax, schema, typed-deserialization, or
/// structural-invariant failures.
pub fn parse_manifest(bytes: &[u8]) -> Result<ProcedurePackManifest, ManifestError> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|source| ManifestError::InvalidJson { source })?;
    validate_manifest_schema(&value)?;
    let manifest = serde_json::from_value::<ProcedurePackManifest>(value)
        .map_err(|source| ManifestError::DeserializeError { source })?;
    validate_manifest_invariants(&manifest)?;
    Ok(manifest)
}

fn validate_manifest_schema(value: &Value) -> Result<(), ManifestError> {
    let validator = jsonschema::draft202012::options()
        .build(manifest_json_schema())
        .map_err(|error| ManifestError::SchemaViolation {
            errors: vec![error.to_string()],
        })?;
    let errors = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::SchemaViolation { errors })
    }
}

fn validate_manifest_invariants(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    if manifest.schema_version != SCHEMA_VERSION_SUPPORTED {
        return Err(ManifestError::UnsupportedSchemaVersion {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION_SUPPORTED,
        });
    }
    if manifest.content_hash != PLACEHOLDER_CONTENT_HASH {
        return Err(ManifestError::UnsupportedContentHash {
            content_hash: manifest.content_hash.clone(),
        });
    }
    semver::Version::parse(&manifest.pack_version).map_err(|source| {
        ManifestError::InvalidPackVersion {
            pack_version: manifest.pack_version.clone(),
            detail: source.to_string(),
        }
    })?;
    if !valid_segment(&manifest.pack_name) {
        return Err(ManifestError::InvalidPackName {
            pack_name: manifest.pack_name.clone(),
            detail: "pack_name must match ^[A-Za-z_][A-Za-z0-9_]*$",
        });
    }

    let mut seen = HashSet::new();
    for procedure in &manifest.procedures {
        let segments = validate_procedure_name(&procedure.name)?;
        if segments.first().copied() == Some("selene") {
            return Err(ManifestError::ReservedNamespaceConflict {
                procedure_name: procedure.name.clone(),
            });
        }
        let expected_prefix = format!("{}.", manifest.pack_name);
        if !procedure.name.starts_with(&expected_prefix) {
            return Err(ManifestError::ProcedureNameOutsidePack {
                procedure_name: procedure.name.clone(),
                expected_prefix,
            });
        }
        if !seen.insert(procedure.name.as_str()) {
            return Err(ManifestError::DuplicateProcedureName {
                procedure_name: procedure.name.clone(),
            });
        }
        if procedure.tier == ManifestTier::Persist {
            return Err(ManifestError::PersistTierInManifest {
                procedure_name: procedure.name.clone(),
            });
        }
        validate_tier_mutability(
            procedure.name.as_str(),
            procedure.tier,
            procedure.mutability,
        )?;
        validate_schema_ref(
            procedure.name.as_str(),
            "input_schema",
            &procedure.input_schema,
        )?;
        validate_schema_ref(
            procedure.name.as_str(),
            "output_schema",
            &procedure.output_schema,
        )?;
    }
    Ok(())
}

fn validate_procedure_name(name: &str) -> Result<Vec<&str>, ManifestError> {
    let segments = name.split('.').collect::<Vec<_>>();
    if segments.len() < 2 {
        return Err(ManifestError::InvalidProcedureName {
            procedure_name: name.to_owned(),
            detail: "procedure name must contain at least two dot-separated segments",
        });
    }
    if segments.iter().any(|segment| !valid_segment(segment)) {
        return Err(ManifestError::InvalidProcedureName {
            procedure_name: name.to_owned(),
            detail: "procedure name segments must match ^[A-Za-z_][A-Za-z0-9_]*$",
        });
    }
    Ok(segments)
}

fn valid_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn validate_tier_mutability(
    procedure_name: &str,
    declared_tier: ManifestTier,
    declared_mutability: ManifestMutability,
) -> Result<(), ManifestError> {
    let expected_tier = match declared_mutability {
        ManifestMutability::Read => ManifestTier::Graph,
        ManifestMutability::GraphWrite
        | ManifestMutability::SchemaWrite
        | ManifestMutability::Admin => ManifestTier::Mutation,
    };
    if declared_tier == expected_tier {
        Ok(())
    } else {
        Err(ManifestError::MutabilityTierMismatch {
            procedure_name: procedure_name.to_owned(),
            declared_tier,
            declared_mutability,
            expected_tier,
        })
    }
}

fn validate_schema_ref(
    procedure_name: &str,
    field: &'static str,
    schema_ref: &ManifestSchemaRef,
) -> Result<(), ManifestError> {
    match schema_ref {
        ManifestSchemaRef::Inline(schema) => validate_inline_schema(procedure_name, field, schema),
        ManifestSchemaRef::Path { relative_to } => {
            validate_schema_path(procedure_name, field, relative_to)
        }
    }
}

fn validate_inline_schema(
    procedure_name: &str,
    field: &'static str,
    schema: &Value,
) -> Result<(), ManifestError> {
    let validator = jsonschema::draft202012::meta::validator();
    let errors = validator
        .iter_errors(schema)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::InvalidInlineSchema {
            procedure_name: procedure_name.to_owned(),
            field,
            errors,
        })
    }
}

fn validate_schema_path(
    procedure_name: &str,
    field: &'static str,
    path: &str,
) -> Result<(), ManifestError> {
    let detail = if path.is_empty() {
        Some("path must be non-empty")
    } else if path.starts_with('/') || path.starts_with('\\') {
        Some("path must be relative")
    } else if path.as_bytes().get(1) == Some(&b':') {
        Some("windows drive prefixes are not allowed")
    } else if path.split(['/', '\\']).any(|component| component == "..") {
        Some("parent directory components are not allowed")
    } else if path.split(['/', '\\']).any(str::is_empty) {
        Some("empty path components are not allowed")
    } else {
        None
    };
    if let Some(detail) = detail {
        Err(ManifestError::InvalidSchemaPath {
            procedure_name: procedure_name.to_owned(),
            field,
            path: path.to_owned(),
            detail,
        })
    } else {
        Ok(())
    }
}
