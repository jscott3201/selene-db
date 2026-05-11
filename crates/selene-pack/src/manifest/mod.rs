//! Procedure-pack manifest parsing and JSON Schema generation.

pub(crate) mod canonical;
mod error;
mod gates;
mod parser;
mod procedures;
mod schema;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use canonical::{canonical_bytes_for_hashing, hex_lower};
pub use error::ManifestError;
pub use gates::{
    ACTIVATION_SEAL_COVERAGE, DEFERRED_GATES, FINAL_VALIDATION_COVERAGE, Gate,
    MANIFEST_LEVEL_GATES, MANIFEST_VALIDATION_COVERAGE, MAX_INLINE_SCHEMA_SIZE_BYTES,
    MAX_PROCEDURE_NAME_LENGTH, MAX_PROCEDURES_PER_PACK, PROCEDURE_LEVEL_GATES, WAL_AUDIT_COVERAGE,
};
pub use parser::parse_manifest;
pub use procedures::{ManifestMutability, ManifestProcedureEntry, ManifestSchemaRef, ManifestTier};
pub use schema::{MANIFEST_SCHEMA_DRAFT, manifest_json_schema};

/// Procedure-pack manifest schema version supported by this binary.
pub const SCHEMA_VERSION_SUPPORTED: u32 = 1;

/// Sentinel content hash used while canonicalizing manifests for hashing.
pub(crate) const BLAKE3_HASH_SENTINEL: &str =
    "blake3:0000000000000000000000000000000000000000000000000000000000000000";

/// Typed procedure-pack manifest.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePackManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Canonical single-segment package name.
    pub pack_name: String,
    /// Semver package version string.
    pub pack_version: String,
    /// Manifest content hash as `blake3:<64 lowercase hex characters>`.
    pub content_hash: String,
    /// Declared procedures.
    pub procedures: Vec<ManifestProcedureEntry>,
}
