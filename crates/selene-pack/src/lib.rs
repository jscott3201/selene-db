//! Built-in procedure pack registry for selene-db.
//!
//! `selene-pack` owns concrete built-in registration and implements the
//! procedure-registry boundary consumed by `selene-gql`. Registration is
//! construct-time-only in v1.0; callers receive a frozen registry that supports
//! plan-time lookup and runtime dispatch.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod activation;
mod builtin;
mod error;
mod manifest;
mod registry;
mod reserved;

pub use activation::{
    ActivationEntry, ActivationError, ActivationRegistry, ActivationStatus, Active, ContentHash,
    Deprecated, Disabled, GraphCommitSink, LifecycleEvent, LifecycleSink, NoopSink, Principal,
    Staged, Uploaded, Validating,
};
pub use error::RegistryError;
pub use manifest::{
    ACTIVATION_SEAL_COVERAGE, DEFERRED_GATES, Gate, MANIFEST_LEVEL_GATES, MANIFEST_SCHEMA_DRAFT,
    MANIFEST_VALIDATION_COVERAGE, MAX_INLINE_SCHEMA_SIZE_BYTES, MAX_PROCEDURE_NAME_LENGTH,
    MAX_PROCEDURES_PER_PACK, ManifestError, ManifestMutability, ManifestProcedureEntry,
    ManifestSchemaRef, ManifestTier, PLACEHOLDER_CONTENT_HASH, PROCEDURE_LEVEL_GATES,
    ProcedurePackManifest, SCHEMA_VERSION_SUPPORTED, WAL_AUDIT_COVERAGE, manifest_json_schema,
    parse_manifest,
};
pub use registry::ProcedurePackRegistry;
pub use reserved::{RESERVED_LABEL_PREFIX, RESERVED_PACK_NAMESPACE};
pub use selene_core::IStr;
pub use selene_gql::{
    GraphContext, MutationContext, ProcedureContext, ProcedureError, ProcedureHandle,
    ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema,
    ProcedureParameter, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Value,
};
