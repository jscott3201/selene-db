//! Manifest parsing and validation errors.

/// Failure while parsing or validating a procedure-pack manifest.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The input bytes were not valid JSON.
    #[error("manifest is not valid JSON: {source}")]
    InvalidJson {
        /// JSON parser error.
        source: serde_json::Error,
    },
    /// The JSON value failed the generated manifest JSON Schema.
    #[error("manifest violates JSON Schema: {}", errors.join("; "))]
    SchemaViolation {
        /// Schema validation errors.
        errors: Vec<String>,
    },
    /// The JSON value could not deserialize into the typed manifest shape.
    #[error("manifest could not deserialize into typed structure: {source}")]
    DeserializeError {
        /// Serde deserialization error.
        source: serde_json::Error,
    },
    /// The manifest schema version is not supported by this binary.
    #[error("unsupported manifest schema version {found}; supported version is {supported}")]
    UnsupportedSchemaVersion {
        /// Version found in the manifest.
        found: u32,
        /// Version supported by this binary.
        supported: u32,
    },
    /// The manifest content hash is not the v1.0 placeholder.
    #[error("unsupported manifest content hash {content_hash}")]
    UnsupportedContentHash {
        /// Manifest content hash value.
        content_hash: String,
    },
    /// The manifest package version is not valid semver.
    #[error("invalid manifest package version {pack_version}: {detail}")]
    InvalidPackVersion {
        /// Raw package version string.
        pack_version: String,
        /// Stable detail string.
        detail: String,
    },
    /// The manifest package name is not a canonical ASCII identifier.
    #[error("invalid manifest package name {pack_name}: {detail}")]
    InvalidPackName {
        /// Raw package name.
        pack_name: String,
        /// Stable detail string.
        detail: &'static str,
    },
    /// A procedure name is not a canonical dot-joined identifier path.
    #[error("invalid manifest procedure name {procedure_name}: {detail}")]
    InvalidProcedureName {
        /// Raw procedure name.
        procedure_name: String,
        /// Stable detail string.
        detail: &'static str,
    },
    /// A procedure name does not begin with the package-name prefix.
    #[error("procedure {procedure_name} is outside pack prefix {expected_prefix}")]
    ProcedureNameOutsidePack {
        /// Raw procedure name.
        procedure_name: String,
        /// Required `pack_name.` prefix.
        expected_prefix: String,
    },
    /// A manifest attempted to declare a platform-reserved procedure name.
    #[error("procedure {procedure_name} conflicts with the reserved selene namespace")]
    ReservedNamespaceConflict {
        /// Raw procedure name.
        procedure_name: String,
    },
    /// Two manifest entries declared the same canonical procedure name.
    #[error("duplicate manifest procedure name {procedure_name}")]
    DuplicateProcedureName {
        /// Duplicate procedure name.
        procedure_name: String,
    },
    /// A procedure declared a tier inconsistent with its mutability.
    #[error(
        "procedure {procedure_name} declares tier {declared_tier:?} with mutability \
         {declared_mutability:?}, but mutability implies tier {expected_tier:?}"
    )]
    MutabilityTierMismatch {
        /// Raw procedure name.
        procedure_name: String,
        /// Tier declared by the manifest.
        declared_tier: crate::ManifestTier,
        /// Mutability declared by the manifest.
        declared_mutability: crate::ManifestMutability,
        /// Tier implied by the declared mutability.
        expected_tier: crate::ManifestTier,
    },
    /// Persist-tier procedures are deferred out of v1.0.
    #[error("persist-tier manifest procedure is not implemented in v1.0 for {procedure_name}")]
    PersistTierInManifest {
        /// Raw procedure name.
        procedure_name: String,
    },
    /// An inline procedure schema is not a valid JSON Schema 2020-12 document.
    #[error(
        "inline schema for procedure {procedure_name} field {field} violates JSON Schema \
         2020-12: {}",
        errors.join("; ")
    )]
    InvalidInlineSchema {
        /// Raw procedure name.
        procedure_name: String,
        /// Field containing the inline schema.
        field: &'static str,
        /// Meta-schema validation errors.
        errors: Vec<String>,
    },
    /// A path schema reference is not a safe relative path.
    #[error("invalid schema path {path} for procedure {procedure_name} field {field}: {detail}")]
    InvalidSchemaPath {
        /// Raw procedure name.
        procedure_name: String,
        /// Field containing the path schema reference.
        field: &'static str,
        /// Raw path string.
        path: String,
        /// Stable detail string.
        detail: &'static str,
    },
}
