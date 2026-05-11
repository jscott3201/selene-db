//! Procedure-pack manifest parsing and JSON Schema generation.

mod error;
mod gates;
mod parser;
mod procedures;
mod schema;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use error::ManifestError;
pub use gates::{
    ACTIVATION_SEAL_COVERAGE, DEFERRED_GATES, Gate, MANIFEST_LEVEL_GATES,
    MANIFEST_VALIDATION_COVERAGE, MAX_INLINE_SCHEMA_SIZE_BYTES, MAX_PROCEDURE_NAME_LENGTH,
    MAX_PROCEDURES_PER_PACK, PROCEDURE_LEVEL_GATES,
};
pub use parser::parse_manifest;
pub use procedures::{ManifestMutability, ManifestProcedureEntry, ManifestSchemaRef, ManifestTier};
pub use schema::{MANIFEST_SCHEMA_DRAFT, manifest_json_schema};

/// Procedure-pack manifest schema version supported by this binary.
pub const SCHEMA_VERSION_SUPPORTED: u32 = 1;

/// Placeholder content hash accepted until BRIEF-48 lands canonical hashing.
pub const PLACEHOLDER_CONTENT_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

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
    /// Manifest content hash placeholder.
    pub content_hash: String,
    /// Declared procedures.
    pub procedures: Vec<ManifestProcedureEntry>,
}
