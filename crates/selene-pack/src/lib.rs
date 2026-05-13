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
mod external;
mod history;
mod manifest;
mod registry;
mod reserved;
#[cfg(any(test, feature = "test-harness"))]
mod snapshot_summary;

pub use activation::{
    ActivationEntry, ActivationError, ActivationRegistry, ActivationStatus, Active, ContentHash,
    Deprecated, Disabled, GraphCommitSink, LifecycleEvent, LifecycleSink, NoopSink, Principal,
    Staged, Uploaded, Validating,
};
pub use error::RegistryError;
pub use external::{
    ExternalGraphProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
    ExternalProcedurePack,
};
pub use history::PackHistorySource;
pub use manifest::{
    ACTIVATION_SEAL_COVERAGE, DEFERRED_GATES, FINAL_VALIDATION_COVERAGE, Gate,
    MANIFEST_LEVEL_GATES, MANIFEST_SCHEMA_DRAFT, MANIFEST_VALIDATION_COVERAGE,
    MAX_INLINE_SCHEMA_SIZE_BYTES, MAX_PROCEDURE_NAME_LENGTH, MAX_PROCEDURES_PER_PACK,
    ManifestError, ManifestMutability, ManifestProcedureEntry, ManifestSchemaRef, ManifestTier,
    PROCEDURE_LEVEL_GATES, ProcedurePackManifest, SCHEMA_VERSION_SUPPORTED, WAL_AUDIT_COVERAGE,
    manifest_json_schema, parse_manifest,
};
pub use registry::{ProcedurePackRegistry, ProcedurePackRegistryBuilder};
pub use reserved::{RESERVED_LABEL_PREFIX, RESERVED_PACK_NAMESPACE};
pub use selene_core::IStr;
pub use selene_gql::{
    GraphContext, MutationContext, ProcedureContext, ProcedureError, ProcedureHandle,
    ProcedureMetadata, ProcedureMutability, ProcedureOutputColumn, ProcedureOutputSchema,
    ProcedureParameter, ProcedureRegistry, ProcedureResult, ProcedureSignature, ProcedureTier,
    Value,
};
#[cfg(any(test, feature = "test-harness"))]
pub use snapshot_summary::{
    PackFixtureKind, PackSnapshot, PackSnapshotInput, PackSnapshotSection, pack_summary,
};
