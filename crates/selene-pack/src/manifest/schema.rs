//! Generated JSON Schema for procedure-pack manifests.

use std::sync::OnceLock;

use schemars::generate::SchemaSettings;
use serde_json::Value;

use super::ProcedurePackManifest;

/// JSON Schema draft emitted for procedure-pack manifests.
pub const MANIFEST_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Return the generated JSON Schema 2020-12 document for procedure-pack manifests.
#[must_use]
pub fn manifest_json_schema() -> &'static Value {
    static CACHE: OnceLock<Value> = OnceLock::new();
    CACHE.get_or_init(|| {
        SchemaSettings::draft2020_12()
            .into_generator()
            .into_root_schema_for::<ProcedurePackManifest>()
            .to_value()
    })
}
