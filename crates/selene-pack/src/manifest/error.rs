//! Manifest parsing and validation errors.

use super::Gate;

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
    /// Manifest procedure count exceeds the v1.0 bound.
    #[error("manifest declares {count} procedures, exceeding limit {limit}")]
    PackProcedureCountExceedsBound {
        /// Procedure count found in the manifest.
        count: usize,
        /// Maximum accepted procedure count.
        limit: usize,
    },
    /// Inline schema serialized size exceeds the v1.0 bound.
    #[error(
        "inline schema for procedure {procedure_name} field {field} is {size} bytes, exceeding \
         limit {limit}"
    )]
    InlineSchemaSizeExceedsBound {
        /// Raw procedure name.
        procedure_name: String,
        /// Field containing the inline schema.
        field: &'static str,
        /// Serialized size in bytes.
        size: usize,
        /// Maximum accepted serialized size.
        limit: usize,
    },
    /// Inline procedure schema failed JSON Schema compilation.
    #[error(
        "inline schema for procedure {procedure_name} field {field} failed to compile: {}",
        errors.join("; ")
    )]
    ProcedureSchemaCompileFailed {
        /// Raw procedure name.
        procedure_name: String,
        /// Field containing the inline schema.
        field: &'static str,
        /// Compilation errors.
        errors: Vec<String>,
    },
    /// Capability declaration does not follow the v1.0 lexical shape.
    #[error("malformed capability {capability} for procedure {procedure_name}: {detail}")]
    ProcedureCapabilityMalformed {
        /// Raw procedure name.
        procedure_name: String,
        /// Raw capability declaration.
        capability: String,
        /// Stable detail string.
        detail: &'static str,
    },
    /// Procedure name exceeds the v1.0 byte-length bound.
    #[error("procedure name {procedure_name} is {length} bytes, exceeding limit {limit}")]
    ProcedureNameTooLong {
        /// Raw procedure name.
        procedure_name: String,
        /// Procedure name byte length.
        length: usize,
        /// Maximum accepted byte length.
        limit: usize,
    },
}

impl ManifestError {
    /// Validation gate that owns this manifest error variant.
    #[must_use]
    pub fn gate(&self) -> Gate {
        match self {
            Self::InvalidJson { .. } | Self::SchemaViolation { .. } => {
                Gate::ManifestSyntaxAndSchema
            }
            Self::DeserializeError { .. } => Gate::ManifestTypedShape,
            Self::UnsupportedSchemaVersion { .. } => Gate::ManifestSchemaVersionSupported,
            Self::UnsupportedContentHash { .. } => Gate::ContentHashPlaceholderRecognized,
            Self::InvalidPackVersion { .. } => Gate::PackVersionWellFormed,
            Self::InvalidPackName { .. } => Gate::PackNameLexical,
            Self::InvalidProcedureName { .. } => Gate::ProcedureNameLexical,
            Self::ProcedureNameOutsidePack { .. } => Gate::ProcedureWithinPack,
            Self::ReservedNamespaceConflict { .. } => Gate::ReservedNamespace,
            Self::DuplicateProcedureName { .. } => Gate::ProcedureNamesUnique,
            Self::MutabilityTierMismatch { .. } => Gate::TierMutabilityConsistency,
            Self::PersistTierInManifest { .. } => Gate::PersistTierRejected,
            Self::InvalidInlineSchema { .. } => Gate::InlineSchemaMetaValid,
            Self::InvalidSchemaPath { .. } => Gate::PathSchemaSafety,
            Self::PackProcedureCountExceedsBound { .. } => Gate::PackProcedureCountBounded,
            Self::InlineSchemaSizeExceedsBound { .. } => Gate::InlineSchemaSizeBounded,
            Self::ProcedureSchemaCompileFailed { field, .. } => match *field {
                "input_schema" => Gate::ProcedureInputSchemaCompiles,
                "output_schema" => Gate::ProcedureOutputSchemaCompiles,
                _ => Gate::ProcedureInputSchemaCompiles,
            },
            Self::ProcedureCapabilityMalformed { .. } => Gate::ProcedureCapabilityFormat,
            Self::ProcedureNameTooLong { .. } => Gate::ProcedureNameLengthBounded,
        }
    }
}
