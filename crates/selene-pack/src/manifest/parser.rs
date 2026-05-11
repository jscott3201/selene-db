//! Procedure-pack manifest parser.

use std::collections::HashSet;

use serde_json::Value;

use super::{
    Gate, MANIFEST_LEVEL_GATES, MAX_INLINE_SCHEMA_SIZE_BYTES, MAX_PROCEDURE_NAME_LENGTH,
    MAX_PROCEDURES_PER_PACK, ManifestError, ManifestMutability, ManifestProcedureEntry,
    ManifestSchemaRef, ManifestTier, PLACEHOLDER_CONTENT_HASH, PROCEDURE_LEVEL_GATES,
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
    for gate in MANIFEST_LEVEL_GATES {
        enforce_manifest_gate(*gate, manifest)?;
    }
    for procedure in &manifest.procedures {
        for gate in PROCEDURE_LEVEL_GATES {
            enforce_procedure_gate(*gate, procedure, manifest)?;
        }
    }
    Ok(())
}

fn enforce_manifest_gate(
    gate: Gate,
    manifest: &ProcedurePackManifest,
) -> Result<(), ManifestError> {
    match gate {
        Gate::ManifestSyntaxAndSchema | Gate::ManifestTypedShape => Ok(()),
        Gate::ManifestSchemaVersionSupported => validate_schema_version(manifest),
        Gate::ContentHashPlaceholderRecognized => validate_content_hash(manifest),
        Gate::PackVersionWellFormed => validate_pack_version(manifest),
        Gate::PackNameLexical => validate_pack_name(manifest),
        Gate::PackProcedureCountBounded => validate_procedure_count(manifest),
        Gate::ProcedureNamesUnique => validate_unique_procedure_names(manifest),
        Gate::ProcedureNameLexical
        | Gate::ProcedureWithinPack
        | Gate::ReservedNamespace
        | Gate::PersistTierRejected
        | Gate::TierMutabilityConsistency
        | Gate::InlineSchemaSizeBounded
        | Gate::InlineSchemaMetaValid
        | Gate::PathSchemaSafety
        | Gate::ProcedureInputSchemaCompiles
        | Gate::ProcedureOutputSchemaCompiles
        | Gate::ProcedureCapabilityFormat
        | Gate::ProcedureNameLengthBounded
        | Gate::ContentHashCanonical
        | Gate::ContentHashConsistency
        | Gate::ActivationLifecycleAtomicity
        | Gate::RegistryConflictDetection => {
            unreachable!("non-manifest gate in manifest-level validation slice")
        }
    }
}

fn enforce_procedure_gate(
    gate: Gate,
    procedure: &ManifestProcedureEntry,
    manifest: &ProcedurePackManifest,
) -> Result<(), ManifestError> {
    match gate {
        Gate::ProcedureNameLexical => validate_procedure_name(&procedure.name).map(|_| ()),
        Gate::ProcedureWithinPack => validate_procedure_within_pack(procedure, manifest),
        Gate::ReservedNamespace => validate_reserved_namespace(procedure),
        Gate::PersistTierRejected => validate_persist_tier(procedure),
        Gate::TierMutabilityConsistency => validate_tier_mutability(
            procedure.name.as_str(),
            procedure.tier,
            procedure.mutability,
        ),
        Gate::ProcedureNameLengthBounded => validate_procedure_name_length(procedure),
        Gate::InlineSchemaSizeBounded => validate_inline_schema_sizes(procedure),
        Gate::InlineSchemaMetaValid => validate_inline_schema_metas(procedure),
        Gate::PathSchemaSafety => validate_schema_paths(procedure),
        Gate::ProcedureInputSchemaCompiles => validate_inline_schema_compiles(
            procedure.name.as_str(),
            "input_schema",
            &procedure.input_schema,
        ),
        Gate::ProcedureOutputSchemaCompiles => validate_inline_schema_compiles(
            procedure.name.as_str(),
            "output_schema",
            &procedure.output_schema,
        ),
        Gate::ProcedureCapabilityFormat => validate_capability_format(procedure),
        Gate::ManifestSyntaxAndSchema
        | Gate::ManifestTypedShape
        | Gate::ManifestSchemaVersionSupported
        | Gate::ContentHashPlaceholderRecognized
        | Gate::PackVersionWellFormed
        | Gate::PackNameLexical
        | Gate::PackProcedureCountBounded
        | Gate::ProcedureNamesUnique
        | Gate::ContentHashCanonical
        | Gate::ContentHashConsistency
        | Gate::ActivationLifecycleAtomicity
        | Gate::RegistryConflictDetection => {
            unreachable!("non-procedure gate in procedure-level validation slice")
        }
    }
}

fn validate_schema_version(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    if manifest.schema_version != SCHEMA_VERSION_SUPPORTED {
        return Err(ManifestError::UnsupportedSchemaVersion {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION_SUPPORTED,
        });
    }
    Ok(())
}

fn validate_content_hash(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    if manifest.content_hash != PLACEHOLDER_CONTENT_HASH {
        return Err(ManifestError::UnsupportedContentHash {
            content_hash: manifest.content_hash.clone(),
        });
    }
    Ok(())
}

fn validate_pack_version(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    semver::Version::parse(&manifest.pack_version).map_err(|source| {
        ManifestError::InvalidPackVersion {
            pack_version: manifest.pack_version.clone(),
            detail: source.to_string(),
        }
    })?;
    Ok(())
}

fn validate_pack_name(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    if !valid_segment(&manifest.pack_name) {
        return Err(ManifestError::InvalidPackName {
            pack_name: manifest.pack_name.clone(),
            detail: "pack_name must match ^[A-Za-z_][A-Za-z0-9_]*$",
        });
    }
    Ok(())
}

fn validate_procedure_count(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    let count = manifest.procedures.len();
    if count > MAX_PROCEDURES_PER_PACK {
        Err(ManifestError::PackProcedureCountExceedsBound {
            count,
            limit: MAX_PROCEDURES_PER_PACK,
        })
    } else {
        Ok(())
    }
}

fn validate_unique_procedure_names(manifest: &ProcedurePackManifest) -> Result<(), ManifestError> {
    let mut seen = HashSet::new();
    for procedure in &manifest.procedures {
        if !seen.insert(procedure.name.as_str()) {
            return Err(ManifestError::DuplicateProcedureName {
                procedure_name: procedure.name.clone(),
            });
        }
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

fn validate_procedure_within_pack(
    procedure: &ManifestProcedureEntry,
    manifest: &ProcedurePackManifest,
) -> Result<(), ManifestError> {
    let expected_prefix = format!("{}.", manifest.pack_name);
    if procedure.name.starts_with(&expected_prefix) {
        Ok(())
    } else {
        Err(ManifestError::ProcedureNameOutsidePack {
            procedure_name: procedure.name.clone(),
            expected_prefix,
        })
    }
}

fn validate_reserved_namespace(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    if procedure.name.split('.').next() == Some("selene") {
        Err(ManifestError::ReservedNamespaceConflict {
            procedure_name: procedure.name.clone(),
        })
    } else {
        Ok(())
    }
}

fn validate_persist_tier(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    if procedure.tier == ManifestTier::Persist {
        Err(ManifestError::PersistTierInManifest {
            procedure_name: procedure.name.clone(),
        })
    } else {
        Ok(())
    }
}

fn validate_procedure_name_length(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    // `str::len` is byte length; lexical gates guarantee ASCII-only names.
    let length = procedure.name.len();
    if length > MAX_PROCEDURE_NAME_LENGTH {
        Err(ManifestError::ProcedureNameTooLong {
            procedure_name: procedure.name.clone(),
            length,
            limit: MAX_PROCEDURE_NAME_LENGTH,
        })
    } else {
        Ok(())
    }
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

fn validate_inline_schema_sizes(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    validate_inline_schema_size(
        procedure.name.as_str(),
        "input_schema",
        &procedure.input_schema,
    )?;
    validate_inline_schema_size(
        procedure.name.as_str(),
        "output_schema",
        &procedure.output_schema,
    )
}

fn validate_inline_schema_size(
    procedure_name: &str,
    field: &'static str,
    schema_ref: &ManifestSchemaRef,
) -> Result<(), ManifestError> {
    let ManifestSchemaRef::Inline(schema) = schema_ref else {
        return Ok(());
    };
    let size = serde_json::to_vec(schema).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_INLINE_SCHEMA_SIZE_BYTES {
        Err(ManifestError::InlineSchemaSizeExceedsBound {
            procedure_name: procedure_name.to_owned(),
            field,
            size,
            limit: MAX_INLINE_SCHEMA_SIZE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn validate_inline_schema_metas(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    validate_inline_schema_meta(
        procedure.name.as_str(),
        "input_schema",
        &procedure.input_schema,
    )?;
    validate_inline_schema_meta(
        procedure.name.as_str(),
        "output_schema",
        &procedure.output_schema,
    )
}

fn validate_inline_schema_meta(
    procedure_name: &str,
    field: &'static str,
    schema_ref: &ManifestSchemaRef,
) -> Result<(), ManifestError> {
    match schema_ref {
        ManifestSchemaRef::Inline(schema) => validate_inline_schema(procedure_name, field, schema),
        ManifestSchemaRef::Path { .. } => Ok(()),
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

fn validate_schema_paths(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    validate_schema_path_ref(
        procedure.name.as_str(),
        "input_schema",
        &procedure.input_schema,
    )?;
    validate_schema_path_ref(
        procedure.name.as_str(),
        "output_schema",
        &procedure.output_schema,
    )
}

fn validate_schema_path_ref(
    procedure_name: &str,
    field: &'static str,
    schema_ref: &ManifestSchemaRef,
) -> Result<(), ManifestError> {
    match schema_ref {
        ManifestSchemaRef::Path { relative_to } => {
            validate_schema_path(procedure_name, field, relative_to)
        }
        ManifestSchemaRef::Inline(_) => Ok(()),
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
    } else if path.contains('\\') {
        Some("windows-style separators are not allowed; use '/'")
    } else if path.split('/').any(|component| component == "..") {
        Some("parent directory components are not allowed")
    } else if path.split('/').any(str::is_empty) {
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

fn validate_inline_schema_compiles(
    procedure_name: &str,
    field: &'static str,
    schema_ref: &ManifestSchemaRef,
) -> Result<(), ManifestError> {
    let ManifestSchemaRef::Inline(schema) = schema_ref else {
        return Ok(());
    };
    jsonschema::draft202012::options()
        .build(schema)
        .map(|_| ())
        .map_err(|error| ManifestError::ProcedureSchemaCompileFailed {
            procedure_name: procedure_name.to_owned(),
            field,
            errors: vec![error.to_string()],
        })
}

fn validate_capability_format(procedure: &ManifestProcedureEntry) -> Result<(), ManifestError> {
    let Some(capability) = procedure.capability_required.as_deref() else {
        return Ok(());
    };
    if is_well_formed_capability(capability) {
        Ok(())
    } else {
        Err(ManifestError::ProcedureCapabilityMalformed {
            procedure_name: procedure.name.clone(),
            capability: capability.to_owned(),
            detail: "capability must match ^[a-z][a-z0-9_]*:[a-z][a-z0-9_]*$",
        })
    }
}

fn is_well_formed_capability(value: &str) -> bool {
    if value.matches(':').count() != 1 {
        return false;
    }
    let Some((resource, action)) = value.split_once(':') else {
        return false;
    };
    is_well_formed_capability_segment(resource) && is_well_formed_capability_segment(action)
}

fn is_well_formed_capability_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}
