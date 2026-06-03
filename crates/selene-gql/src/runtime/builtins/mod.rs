//! Native platform built-in procedures (`selene.*`).
//!
//! The first five procedures were relocated verbatim from the historical
//! procedure-pack built-ins into `selene-gql` and registered directly in the
//! native [`BuiltinProcedureRegistry`](crate::BuiltinProcedureRegistry) (STEP 3).
//! The historical `BuiltInMetadata` / `GraphProcedureBuiltIn` /
//! `MutationProcedureBuiltIn` trait indirection and the
//! `UNSTABLE_BUILTIN_CONTENT_HASH` sentinel are dropped — each procedure is a
//! concrete dispatch arm, with planner-visible metadata built from static
//! parameter/output tables (the same `StaticParameter`/`StaticOutputColumn` →
//! `ProcedureMetadata` conversion the pack registry performed). The vector
//! search, approximate vector-search, batched approximate vector-search,
//! vector-index stats, and vector-index
//! procedures are new native engine functionality on the same concrete built-in
//! dispatch path.
//!
//! Tiers and mutability are preserved exactly:
//! - `selene.health`, `selene.feature_status`, `selene.verify`, and
//!   `selene.vector_search_nodes`, `selene.vector_search_nodes_ann`,
//!   `selene.vector_search_nodes_ann_batch`, and `selene.vector_index_stats`
//!   are read-only graph-tier
//!   ([`ProcedureTier::Graph`] + [`ProcedureMutability::Read`]); they never
//!   mutate and never re-enter `begin_write`.
//! - `selene.create_index`, `selene.drop_index`, `selene.create_vector_index`,
//!   and `selene.drop_vector_index` are mutation-tier
//!   ([`ProcedureTier::Mutation`] + [`ProcedureMutability::SchemaWrite`]); they
//!   route every write through [`MutationContext::mutator`] — emitting index
//!   schema changes through the single mutation funnel (Hard Rule 11). They
//!   never bypass the funnel and never re-enter `begin_write`.
//! - `selene.rebuild_vector_indexes` is maintenance-tier
//!   ([`ProcedureTier::Maintenance`] +
//!   [`ProcedureMutability::MaintenanceWrite`]); it rebuilds derived vector
//!   index state through [`MaintenanceContext`] without graph changes, WAL
//!   entries, or schema-version bumps.
//!
//! `pack_history` is **not** relocated: it read the pack-lifecycle audit, which
//! is removed in the teardown.

mod create_index;
mod create_vector_index;
mod drop_index;
mod drop_vector_index;
mod feature_status;
mod health;
mod meta;
mod rebuild_vector_indexes;
mod vector_index_stats;
mod vector_search;
mod vector_search_ann;
mod vector_search_ann_batch;
mod verify;

use selene_core::Value;

use crate::procedure_registry::{ProcedureError, ProcedureHandle};
use crate::{
    GraphContext, MaintenanceContext, MutationContext, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputColumn, ProcedureOutputSchema, ProcedureParameter, ProcedureResult,
    ProcedureSignature, ProcedureTier,
};

/// One native platform built-in procedure, identified by its dispatch kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BuiltinKind {
    /// `selene.health` — read-only graph health counters.
    Health,
    /// `selene.feature_status` — read-only ISO GQL feature support listing.
    FeatureStatus,
    /// `selene.verify` — read-only graph integrity check.
    Verify,
    /// `selene.vector_search_nodes` — exact vector node search.
    VectorSearchNodes,
    /// `selene.vector_search_nodes_ann` — HNSW approximate vector node search.
    VectorSearchNodesAnn,
    /// `selene.vector_search_nodes_ann_batch` — batched HNSW vector node search.
    VectorSearchNodesAnnBatch,
    /// `selene.vector_index_stats` — vector index memory/cardinality stats.
    VectorIndexStats,
    /// `selene.rebuild_vector_indexes` — vector index derived-state rebuild.
    RebuildVectorIndexes,
    /// `selene.create_index` — mutation-tier property-index creation.
    CreateIndex,
    /// `selene.drop_index` — mutation-tier property-index drop.
    DropIndex,
    /// `selene.create_vector_index` — mutation-tier vector-index creation.
    CreateVectorIndex,
    /// `selene.drop_vector_index` — mutation-tier vector-index drop.
    DropVectorIndex,
}

/// Static descriptor binding a canonical procedure name to its dispatch kind.
pub(super) struct BuiltinSpec {
    /// Canonical multipart CALL name, e.g. `["selene", "health"]`.
    pub(super) name: &'static [&'static str],
    /// Human-readable summary used by `SHOW PROCEDURES`.
    pub(super) description: &'static str,
    /// Version where this procedure became available (carried on the signature).
    pub(super) since_version: &'static str,
    /// Dispatch kind.
    pub(super) kind: BuiltinKind,
}

/// The native platform built-ins in registration order. The first five entries
/// preserve the historical pack registration order (`health`,
/// `feature_status`, `verify`, `create_index`, `drop_index`; the former
/// `pack_history` built-in is not relocated). Vector built-ins are appended so
/// legacy handles keep their relative ordering.
pub(super) const BUILTIN_SPECS: [BuiltinSpec; 12] = [
    BuiltinSpec {
        name: &["selene", "health"],
        description: "Report basic graph health counters.",
        since_version: "1.0.0",
        kind: BuiltinKind::Health,
    },
    BuiltinSpec {
        name: &["selene", "feature_status"],
        description: "List ISO GQL feature support status.",
        since_version: "1.1.0",
        kind: BuiltinKind::FeatureStatus,
    },
    BuiltinSpec {
        name: &["selene", "verify"],
        description: "Integrity check against graph invariants.",
        since_version: "1.1.0",
        kind: BuiltinKind::Verify,
    },
    BuiltinSpec {
        name: &["selene", "create_index"],
        description: "Create a property index.",
        since_version: "1.0.0",
        kind: BuiltinKind::CreateIndex,
    },
    BuiltinSpec {
        name: &["selene", "drop_index"],
        description: "Drop a property index.",
        since_version: "1.0.0",
        kind: BuiltinKind::DropIndex,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_nodes"],
        description: "Exact vector search over node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchNodes,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_nodes_ann"],
        description: "Approximate HNSW vector search over node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchNodesAnn,
    },
    BuiltinSpec {
        name: &["selene", "vector_index_stats"],
        description: "Report vector index memory and cardinality statistics.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorIndexStats,
    },
    BuiltinSpec {
        name: &["selene", "rebuild_vector_indexes"],
        description: "Rebuild vector indexes from primary graph values.",
        since_version: "1.1.0",
        kind: BuiltinKind::RebuildVectorIndexes,
    },
    BuiltinSpec {
        name: &["selene", "create_vector_index"],
        description: "Create a vector index.",
        since_version: "1.1.0",
        kind: BuiltinKind::CreateVectorIndex,
    },
    BuiltinSpec {
        name: &["selene", "drop_vector_index"],
        description: "Drop a vector index.",
        since_version: "1.1.0",
        kind: BuiltinKind::DropVectorIndex,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_nodes_ann_batch"],
        description: "Batched approximate HNSW vector search over node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchNodesAnnBatch,
    },
];

impl BuiltinKind {
    /// Build the planner-visible metadata for this built-in.
    ///
    /// Static parameter / output columns are converted with the
    /// `with_description` / `with_default_doc` / `with_default` rules, the
    /// `since_version` rides on the [`ProcedureSignature`], and tier/mutability
    /// come from the procedure.
    pub(super) fn metadata(
        self,
        handle: ProcedureHandle,
        description: &'static str,
        since_version: &'static str,
    ) -> ProcedureMetadata {
        let signature = ProcedureSignature::new(self.signature()).with_since_version(since_version);
        ProcedureMetadata::new(
            handle,
            signature,
            ProcedureOutputSchema {
                columns: self.output_columns(),
            },
            self.tier(),
            self.mutability(),
        )
        .with_description(description)
    }

    /// Declared execution tier.
    pub(super) const fn tier(self) -> ProcedureTier {
        match self {
            Self::Health
            | Self::FeatureStatus
            | Self::Verify
            | Self::VectorSearchNodes
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorIndexStats => ProcedureTier::Graph,
            Self::RebuildVectorIndexes => ProcedureTier::Maintenance,
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex => ProcedureTier::Mutation,
        }
    }

    /// Declared mutability.
    pub(super) const fn mutability(self) -> ProcedureMutability {
        match self {
            Self::Health
            | Self::FeatureStatus
            | Self::Verify
            | Self::VectorSearchNodes
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorIndexStats => ProcedureMutability::Read,
            Self::RebuildVectorIndexes => ProcedureMutability::MaintenanceWrite,
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex => ProcedureMutability::SchemaWrite,
        }
    }

    fn signature(self) -> Vec<ProcedureParameter> {
        match self {
            Self::Health => health::signature(),
            Self::FeatureStatus => feature_status::signature(),
            Self::Verify => verify::signature(),
            Self::VectorSearchNodes => vector_search::signature(),
            Self::VectorSearchNodesAnn => vector_search_ann::signature(),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::signature(),
            Self::VectorIndexStats => vector_index_stats::signature(),
            Self::RebuildVectorIndexes => rebuild_vector_indexes::signature(),
            Self::CreateIndex => create_index::signature(),
            Self::DropIndex => drop_index::signature(),
            Self::CreateVectorIndex => create_vector_index::signature(),
            Self::DropVectorIndex => drop_vector_index::signature(),
        }
    }

    fn output_columns(self) -> Vec<ProcedureOutputColumn> {
        match self {
            Self::Health => health::output_columns(),
            Self::FeatureStatus => feature_status::output_columns(),
            Self::Verify => verify::output_columns(),
            Self::VectorSearchNodes => vector_search::output_columns(),
            Self::VectorSearchNodesAnn => vector_search_ann::output_columns(),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::output_columns(),
            Self::VectorIndexStats => vector_index_stats::output_columns(),
            Self::RebuildVectorIndexes => rebuild_vector_indexes::output_columns(),
            Self::CreateIndex => create_index::output_columns(),
            Self::DropIndex => drop_index::output_columns(),
            Self::CreateVectorIndex => create_vector_index::output_columns(),
            Self::DropVectorIndex => drop_vector_index::output_columns(),
        }
    }

    /// Execute a read-only graph-tier built-in.
    ///
    /// # Panics
    ///
    /// Never panics for the mutation-tier kinds: the registry routes
    /// [`CreateIndex`](Self::CreateIndex)/[`DropIndex`](Self::DropIndex) through
    /// [`execute_mutation`](Self::execute_mutation). This method is only invoked
    /// for graph-tier kinds, mirroring the pack's `GraphProcedureBuiltIn` split.
    pub(super) fn execute_graph(
        self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        match self {
            Self::Health => health::execute(ctx, args),
            Self::FeatureStatus => feature_status::execute(ctx, args),
            Self::Verify => verify::execute(ctx, args),
            Self::VectorSearchNodes => vector_search::execute(ctx, args),
            Self::VectorSearchNodesAnn => vector_search_ann::execute(ctx, args),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::execute(ctx, args),
            Self::VectorIndexStats => vector_index_stats::execute(ctx, args),
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Mutation,
                actual: ProcedureTier::Graph,
            }),
            Self::RebuildVectorIndexes => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Maintenance,
                actual: ProcedureTier::Graph,
            }),
        }
    }

    /// Execute a mutation-tier built-in, routing every write through the
    /// [`MutationContext`] mutation funnel (Hard Rule 11).
    pub(super) fn execute_mutation(
        self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        match self {
            Self::CreateIndex => create_index::execute(ctx, args),
            Self::DropIndex => drop_index::execute(ctx, args),
            Self::CreateVectorIndex => create_vector_index::execute(ctx, args),
            Self::DropVectorIndex => drop_vector_index::execute(ctx, args),
            Self::Health
            | Self::FeatureStatus
            | Self::Verify
            | Self::VectorSearchNodes
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorIndexStats => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Mutation,
            }),
            Self::RebuildVectorIndexes => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Maintenance,
                actual: ProcedureTier::Mutation,
            }),
        }
    }

    /// Execute a maintenance-tier built-in against shared engine state.
    pub(super) fn execute_maintenance(
        self,
        ctx: &MaintenanceContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        match self {
            Self::RebuildVectorIndexes => rebuild_vector_indexes::execute(ctx, args),
            Self::Health
            | Self::FeatureStatus
            | Self::Verify
            | Self::VectorSearchNodes
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorIndexStats => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Maintenance,
            }),
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Mutation,
                actual: ProcedureTier::Maintenance,
            }),
        }
    }
}

/// Build a unit (single empty-row) result, mirroring the pack built-ins' return
/// shape for procedures with no output columns.
pub(super) fn unit_result() -> ProcedureResult {
    ProcedureResult {
        rows: vec![Vec::new()],
    }
}
