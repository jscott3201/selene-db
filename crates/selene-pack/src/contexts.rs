#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Dyn-compatible procedure contexts for `selene-pack`.
//!
//! D4 established procedure-pack extensions with per-tier contexts. D17 makes
//! that concrete: three context structs and three object-safe Procedure traits,
//! with no generic execute methods and no trait-object context hierarchy. See
//! `_spec/05-extension-architecture.md` §3.

use std::marker::PhantomData;

/// Read-only graph procedure context.
pub struct GraphContext<'a> {
    /// Graph snapshot or handle available to graph-tier procedures.
    pub graph: &'a Graph,
}

/// Mutation procedure context.
///
/// The mutator is borrowed from an enclosing write transaction that already
/// holds the single graph write-lock settled by D10.
pub struct MutationContext<'a> {
    /// Graph handle available for read context during mutation execution.
    pub graph: &'a Graph,
    /// Borrowed mutation funnel for graph and catalog writes.
    pub mutator: &'a mut Mutator<'a, 'a>,
}

/// Persistence-aware procedure context.
pub struct PersistContext<'a> {
    /// Graph handle associated with the persistence operation.
    pub graph: &'a Graph,
    /// Persistence surface exposed to persist-tier procedures.
    pub persist: &'a dyn Persist,
}

/// Read-only graph-tier procedure.
pub trait GraphProcedure: Send + Sync + 'static {
    /// Execute with graph-only access.
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

/// Mutation-tier procedure.
pub trait MutationProcedure: Send + Sync + 'static {
    /// Execute inside an existing write transaction.
    fn execute(
        &self,
        ctx: &mut MutationContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

/// Persistence-tier procedure.
pub trait PersistProcedure: Send + Sync + 'static {
    /// Execute with read access to graph state and explicit persistence access.
    fn execute(
        &self,
        ctx: &PersistContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError>;
}

/// Placeholder graph type; the final type lives in `selene-graph`.
pub struct Graph;

/// Placeholder write mutator; the final type lives in `selene-graph`.
pub struct Mutator<'tx, 'g> {
    _marker: PhantomData<(&'tx (), &'g ())>,
}

/// Placeholder persistence trait; the final type lives in `selene-persist`.
pub trait Persist: Send + Sync {}

/// Placeholder value type; the final type lives in `selene-core`.
pub struct Value;

/// Placeholder procedure result; the final type is shared with `selene-gql`.
pub struct ProcedureResult;

/// Placeholder procedure error; the final type is shared with `selene-gql`.
#[derive(Debug)]
pub enum ProcedureError {
    /// Placeholder until M2 defines concrete failure variants.
    M2Placeholder,
}
