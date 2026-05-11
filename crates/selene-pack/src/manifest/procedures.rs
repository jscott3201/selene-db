//! Procedure entries in a procedure-pack manifest.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Procedure execution tier declared by a manifest.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestTier {
    /// Read-only graph-tier procedure.
    Graph,
    /// Mutation-tier procedure.
    Mutation,
    /// Persistence-tier procedure, rejected in v1.0 structural validation.
    Persist,
}

/// Procedure mutability declared by a manifest.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestMutability {
    /// Procedure cannot mutate graph or catalog state.
    Read,
    /// Procedure may mutate graph data.
    GraphWrite,
    /// Procedure may mutate catalog/schema state.
    SchemaWrite,
    /// Procedure performs administrative registry or operational work.
    Admin,
}

/// Reference to a procedure input or output JSON Schema.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum ManifestSchemaRef {
    /// Schema is loaded from a path relative to the containing manifest.
    Path {
        /// Relative path to the JSON Schema file.
        relative_to: String,
    },
    /// Schema is embedded directly in the manifest.
    Inline(Value),
}

/// One procedure declaration in a procedure-pack manifest.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestProcedureEntry {
    /// Canonical dot-joined procedure name, prefixed by the pack name.
    pub name: String,
    /// Declared execution tier.
    pub tier: ManifestTier,
    /// Declared procedure mutability.
    pub mutability: ManifestMutability,
    /// Procedure input schema.
    pub input_schema: ManifestSchemaRef,
    /// Procedure output schema.
    pub output_schema: ManifestSchemaRef,
    /// Optional opaque capability token.
    pub capability_required: Option<String>,
}
