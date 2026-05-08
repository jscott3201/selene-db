#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Planner-facing procedure registry boundary for `selene-gql`.
//!
//! D16 places the single `ProcedureRegistry` trait in `selene-gql` because the
//! planner and executor are the upstream consumers of procedure metadata and
//! dispatch. `selene-pack` implements this trait for its concrete registry, and
//! the embedder injects `&dyn ProcedureRegistry` into plan and execute calls.
//! See `_spec/08-iso-gql-planner-and-executor.md` §7.

use selene_core::IStr;

use crate::GqlType;

/// Registry interface consumed by the GQL planner and executor.
///
/// Registration is intentionally not part of this trait. Concrete registries
/// own their startup-time loading APIs; `selene-gql` only needs plan-time
/// metadata lookup and runtime dispatch through an opaque handle.
pub trait ProcedureRegistry: Send + Sync {
    /// Look up procedure metadata by canonical CALL-time name.
    fn lookup(&self, name: &[IStr]) -> Option<ProcedureMetadata>;

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
#[derive(Clone, Debug)]
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

impl ProcedureHandle {
    /// Construct an opaque handle from a registry-defined raw value.
    ///
    /// `selene-gql` does not interpret the raw value. The concrete registry
    /// chooses the encoding and receives the handle back unchanged through
    /// [`ProcedureRegistry::execute`].
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the opaque raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Static signature used for plan-time argument validation.
#[derive(Clone, Debug, Default)]
pub struct ProcedureSignature {
    /// Positional parameters in declaration order.
    pub parameters: Vec<ProcedureParameter>,
}

/// One declared procedure parameter.
#[derive(Clone, Debug)]
pub struct ProcedureParameter {
    /// Parameter name. Diagnostic-only; arguments are positional in v1.0.
    pub name: IStr,
    /// Expected static type for the corresponding positional argument.
    pub ty: GqlType,
    /// Whether a statically resolved `NULL` argument is accepted.
    pub nullable: bool,
}

/// Output schema as a relation of named columns.
#[derive(Clone, Debug, Default)]
pub struct ProcedureOutputSchema {
    /// Output columns in declaration order.
    pub columns: Vec<ProcedureOutputColumn>,
}

/// One output column from a procedure call.
#[derive(Clone, Debug)]
pub struct ProcedureOutputColumn {
    /// Column name matched against `YIELD col` references.
    pub name: IStr,
    /// Static type assigned to the YIELD binding.
    pub ty: GqlType,
}

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

/// Registry with no registered procedures.
///
/// Use this for analyzer call sites that do not exercise CALL. Any procedure
/// lookup returns `None`, and runtime execution remains out of scope until
/// M5c defines the concrete execution value flow.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyProcedureRegistry;

impl ProcedureRegistry for EmptyProcedureRegistry {
    fn lookup(&self, _name: &[IStr]) -> Option<ProcedureMetadata> {
        None
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        // Why: D17's per-tier runtime execution model lands in M5c; BRIEF-23
        // only consumes lookup() for analyzer-time signature metadata.
        Err(ProcedureError::M2Placeholder)
    }
}
