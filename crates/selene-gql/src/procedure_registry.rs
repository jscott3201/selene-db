#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Planner-facing procedure registry boundary for `selene-gql`.
//!
//! D16 places the single `ProcedureRegistry` trait in `selene-gql` because the
//! planner and executor are the upstream consumers of procedure metadata and
//! dispatch. `selene-pack` implements this trait for its concrete registry, and
//! the embedder injects `&dyn ProcedureRegistry` into plan and execute calls.
//! See `_spec/08-iso-gql-planner-and-executor.md` §7.

/// Registry interface consumed by the GQL planner and executor.
///
/// Registration is intentionally not part of this trait. Concrete registries
/// own their startup-time loading APIs; `selene-gql` only needs plan-time
/// metadata lookup and runtime dispatch through an opaque handle.
pub trait ProcedureRegistry: Send + Sync {
    /// Look up procedure metadata by canonical CALL-time name.
    fn lookup(&self, name: &str) -> Option<ProcedureMetadata>;

    /// Execute a previously planned procedure handle with evaluated arguments.
    fn execute(
        &self,
        handle: ProcedureHandle,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

/// Planner-visible metadata for a registered procedure.
///
/// M2 will define the exact signature, tier, mutability, and capability fields.
/// The important D16 boundary is that this type is owned by `selene-gql` and
/// carries only data the planner can consume without importing `selene-pack`.
pub struct ProcedureMetadata {
    /// Opaque handle returned to the executor after successful planning.
    pub handle: ProcedureHandle,
    /// Input signature used for argument type-checking.
    pub signature: ProcedureSignature,
    /// Output schema used for YIELD validation and binding-table construction.
    pub output_schema: ProcedureOutputSchema,
    /// Tier selected by the concrete registry for execution.
    pub tier: ProcedureTier,
    /// Side-effect class declared by the procedure manifest.
    pub mutability: ProcedureMutability,
    /// Caller-owned capability string, if the manifest requires one.
    pub capability_required: Option<String>,
}

/// Opaque procedure handle.
///
/// The planner treats this as an uninterpreted token. `selene-pack` defines the
/// internal handle encoding in M2.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcedureHandle(u64);

/// Input signature placeholder for M2.
pub struct ProcedureSignature;

/// Output schema placeholder for M2.
pub struct ProcedureOutputSchema;

/// Execution tier advertised by a procedure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcedureTier {
    /// Read-only graph-tier procedure.
    Graph,
    /// Mutation-tier procedure running inside a write transaction.
    Mutation,
    /// Persistence-aware procedure with an explicit persist handle.
    Persist,
}

/// Procedure mutability declared by the pack manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcedureMutability {
    /// Procedure cannot mutate graph or catalog state.
    Read,
    /// Procedure may mutate graph data.
    GraphWrite,
    /// Procedure may mutate catalog/schema state.
    SchemaWrite,
    /// Procedure performs administrative registry or operational work.
    Admin,
}

/// Result returned by procedure execution.
///
/// M2 will define whether this is a binding table, scalar value, unit value, or
/// tagged union of those shapes.
pub struct ProcedureResult;

/// Procedure dispatch failure.
#[derive(Debug, thiserror::Error)]
pub enum ProcedureError {
    /// Placeholder until M2 defines concrete lookup, validation, and runtime
    /// failure variants.
    #[error("M2 work")]
    M2Placeholder,
}

/// Placeholder value type; the final type lives in `selene-core`.
pub struct Value;
