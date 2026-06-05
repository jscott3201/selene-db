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
//! search, batched exact vector-search, candidate vector scoring, batched
//! candidate vector scoring, neighbor candidate vector scoring, batched neighbor
//! candidate vector scoring, expanded-candidate vector scoring, batched
//! expanded-candidate vector scoring, approximate vector-search, batched
//! approximate vector-search, ANN-expanded vector-search, batched ANN-expanded
//! vector-search, vector-index stats, and vector-index procedures are new
//! native engine functionality on the same concrete built-in dispatch path.
//!
//! Tiers and mutability are preserved exactly:
//! - `selene.health`, `selene.feature_status`, `selene.verify`, and
//!   `selene.vector_search_nodes`, `selene.vector_search_nodes_batch`,
//!   `selene.vector_score_nodes`, `selene.vector_score_nodes_batch`,
//!   `selene.vector_score_neighbors`,
//!   `selene.vector_score_neighbors_batch`,
//!   `selene.vector_score_candidate_state`,
//!   `selene.vector_score_candidate_state_nodes`,
//!   `selene.vector_candidate_states`,
//!   `selene.vector_score_expanded_candidates`,
//!   `selene.vector_score_expanded_candidates_batch`,
//!   `selene.vector_search_nodes_ann`, `selene.vector_search_nodes_ann_batch`,
//!   `selene.vector_search_expanded_candidates_ann`,
//!   `selene.vector_search_expanded_candidates_ann_batch`,
//!   and `selene.vector_index_stats` are read-only graph-tier
//!   ([`ProcedureTier::Graph`] + [`ProcedureMutability::Read`]); they never
//!   mutate and never re-enter `begin_write`.
//! - `selene.create_index`, `selene.drop_index`, `selene.create_vector_index`,
//!   and `selene.drop_vector_index` are mutation-tier
//!   ([`ProcedureTier::Mutation`] + [`ProcedureMutability::SchemaWrite`]); they
//!   route every write through [`MutationContext::mutator`] — emitting index
//!   schema changes through the single mutation funnel (Hard Rule 11). They
//!   never bypass the funnel and never re-enter `begin_write`.
//! - `selene.rebuild_vector_indexes` and
//!   `selene.rebuild_recommended_vector_indexes` are maintenance-tier
//!   ([`ProcedureTier::Maintenance`] +
//!   [`ProcedureMutability::MaintenanceWrite`]); they rebuild derived vector
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
mod vector_candidate_states;
mod vector_common;
mod vector_index_stats;
mod vector_score_candidate_state;
mod vector_score_candidate_state_nodes;
mod vector_score_expanded_candidates;
mod vector_score_expanded_candidates_batch;
mod vector_score_neighbors;
mod vector_score_neighbors_batch;
mod vector_score_nodes;
mod vector_score_nodes_batch;
mod vector_search;
mod vector_search_ann;
mod vector_search_ann_batch;
mod vector_search_ann_defaults;
mod vector_search_batch;
mod vector_search_expanded_candidates_ann;
mod vector_search_expanded_candidates_ann_batch;
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
    /// `selene.vector_search_nodes_batch` — batched exact vector node search.
    VectorSearchNodesBatch,
    /// `selene.vector_score_nodes` — exact scoring for explicit node candidates.
    VectorScoreNodes,
    /// `selene.vector_score_nodes_batch` — batched exact scoring for explicit node candidates.
    VectorScoreNodesBatch,
    /// `selene.vector_score_neighbors` — exact scoring for one-hop graph neighbors.
    VectorScoreNeighbors,
    /// `selene.vector_score_neighbors_batch` — batched exact scoring for one-hop graph neighbors.
    VectorScoreNeighborsBatch,
    /// `selene.vector_score_candidate_state` — exact scoring for maintained candidate state.
    VectorScoreCandidateState,
    /// `selene.vector_score_candidate_state_nodes` — exact scoring for composed maintained state and node candidates.
    VectorScoreCandidateStateNodes,
    /// `selene.vector_candidate_states` — maintained candidate-state metadata.
    VectorCandidateStates,
    /// `selene.vector_score_expanded_candidates` — exact scoring for graph-expanded candidates.
    VectorScoreExpandedCandidates,
    /// `selene.vector_score_expanded_candidates_batch` — batched graph-expanded scoring.
    VectorScoreExpandedCandidatesBatch,
    /// `selene.vector_search_nodes_ann` — approximate vector node search.
    VectorSearchNodesAnn,
    /// `selene.vector_search_nodes_ann_batch` — batched approximate vector node search.
    VectorSearchNodesAnnBatch,
    /// `selene.vector_search_expanded_candidates_ann` — ANN-root graph-expanded search.
    VectorSearchExpandedCandidatesAnn,
    /// `selene.vector_search_expanded_candidates_ann_batch` — batched ANN-root graph-expanded search.
    VectorSearchExpandedCandidatesAnnBatch,
    /// `selene.vector_index_stats` — vector index memory/cardinality stats.
    VectorIndexStats,
    /// `selene.rebuild_vector_indexes` — vector index derived-state rebuild.
    RebuildVectorIndexes,
    /// `selene.rebuild_recommended_vector_indexes` — recommended vector-index rebuild.
    RebuildRecommendedVectorIndexes,
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
pub(super) const BUILTIN_SPECS: [BuiltinSpec; 25] = [
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
        description: "Approximate vector search over node properties.",
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
        name: &["selene", "rebuild_recommended_vector_indexes"],
        description: "Rebuild vector indexes whose diagnostics recommend maintenance.",
        since_version: "1.1.0",
        kind: BuiltinKind::RebuildRecommendedVectorIndexes,
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
        description: "Batched approximate vector search over node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchNodesAnnBatch,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_expanded_candidates_ann"],
        description: "Approximate vector search over graph-expanded node candidates.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchExpandedCandidatesAnn,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_expanded_candidates_ann_batch"],
        description: "Batched approximate vector search over graph-expanded node candidates.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchExpandedCandidatesAnnBatch,
    },
    BuiltinSpec {
        name: &["selene", "vector_search_nodes_batch"],
        description: "Batched exact vector search over node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchNodesBatch,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_nodes"],
        description: "Score explicit node candidates by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreNodes,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_nodes_batch"],
        description: "Batched scoring for explicit node candidates by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreNodesBatch,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_neighbors"],
        description: "Score one-hop graph neighbors by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreNeighbors,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_neighbors_batch"],
        description: "Batched scoring for one-hop graph neighbors by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreNeighborsBatch,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_candidate_state"],
        description: "Score a maintained graph candidate-state set by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreCandidateState,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_candidate_state_nodes"],
        description: "Score a maintained candidate-state set composed with explicit nodes.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreCandidateStateNodes,
    },
    BuiltinSpec {
        name: &["selene", "vector_candidate_states"],
        description: "List maintained graph candidate-state metadata.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorCandidateStates,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_expanded_candidates"],
        description: "Score graph-expanded node candidates by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreExpandedCandidates,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_expanded_candidates_batch"],
        description: "Batched scoring for graph-expanded node candidates by a vector property.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreExpandedCandidatesBatch,
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
            | Self::VectorSearchNodesBatch
            | Self::VectorScoreNodes
            | Self::VectorScoreNodesBatch
            | Self::VectorScoreNeighbors
            | Self::VectorScoreNeighborsBatch
            | Self::VectorScoreCandidateState
            | Self::VectorScoreCandidateStateNodes
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats => ProcedureTier::Graph,
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                ProcedureTier::Maintenance
            }
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
            | Self::VectorSearchNodesBatch
            | Self::VectorScoreNodes
            | Self::VectorScoreNodesBatch
            | Self::VectorScoreNeighbors
            | Self::VectorScoreNeighborsBatch
            | Self::VectorScoreCandidateState
            | Self::VectorScoreCandidateStateNodes
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats => ProcedureMutability::Read,
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                ProcedureMutability::MaintenanceWrite
            }
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
            Self::VectorSearchNodesBatch => vector_search_batch::signature(),
            Self::VectorScoreNodes => vector_score_nodes::signature(),
            Self::VectorScoreNodesBatch => vector_score_nodes_batch::signature(),
            Self::VectorScoreNeighbors => vector_score_neighbors::signature(),
            Self::VectorScoreNeighborsBatch => vector_score_neighbors_batch::signature(),
            Self::VectorScoreCandidateState => vector_score_candidate_state::signature(),
            Self::VectorScoreCandidateStateNodes => vector_score_candidate_state_nodes::signature(),
            Self::VectorCandidateStates => vector_candidate_states::signature(),
            Self::VectorScoreExpandedCandidates => vector_score_expanded_candidates::signature(),
            Self::VectorScoreExpandedCandidatesBatch => {
                vector_score_expanded_candidates_batch::signature()
            }
            Self::VectorSearchNodesAnn => vector_search_ann::signature(),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::signature(),
            Self::VectorSearchExpandedCandidatesAnn => {
                vector_search_expanded_candidates_ann::signature()
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::signature()
            }
            Self::VectorIndexStats => vector_index_stats::signature(),
            Self::RebuildVectorIndexes => rebuild_vector_indexes::signature(),
            Self::RebuildRecommendedVectorIndexes => {
                rebuild_vector_indexes::recommended_signature()
            }
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
            Self::VectorSearchNodesBatch => vector_search_batch::output_columns(),
            Self::VectorScoreNodes => vector_score_nodes::output_columns(),
            Self::VectorScoreNodesBatch => vector_score_nodes_batch::output_columns(),
            Self::VectorScoreNeighbors => vector_score_neighbors::output_columns(),
            Self::VectorScoreNeighborsBatch => vector_score_neighbors_batch::output_columns(),
            Self::VectorScoreCandidateState => vector_score_candidate_state::output_columns(),
            Self::VectorScoreCandidateStateNodes => {
                vector_score_candidate_state_nodes::output_columns()
            }
            Self::VectorCandidateStates => vector_candidate_states::output_columns(),
            Self::VectorScoreExpandedCandidates => {
                vector_score_expanded_candidates::output_columns()
            }
            Self::VectorScoreExpandedCandidatesBatch => {
                vector_score_expanded_candidates_batch::output_columns()
            }
            Self::VectorSearchNodesAnn => vector_search_ann::output_columns(),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::output_columns(),
            Self::VectorSearchExpandedCandidatesAnn => {
                vector_search_expanded_candidates_ann::output_columns()
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::output_columns()
            }
            Self::VectorIndexStats => vector_index_stats::output_columns(),
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                rebuild_vector_indexes::output_columns()
            }
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
            Self::VectorSearchNodesBatch => vector_search_batch::execute(ctx, args),
            Self::VectorScoreNodes => vector_score_nodes::execute(ctx, args),
            Self::VectorScoreNodesBatch => vector_score_nodes_batch::execute(ctx, args),
            Self::VectorScoreNeighbors => vector_score_neighbors::execute(ctx, args),
            Self::VectorScoreNeighborsBatch => vector_score_neighbors_batch::execute(ctx, args),
            Self::VectorScoreCandidateState => vector_score_candidate_state::execute(ctx, args),
            Self::VectorScoreCandidateStateNodes => {
                vector_score_candidate_state_nodes::execute(ctx, args)
            }
            Self::VectorCandidateStates => vector_candidate_states::execute(ctx, args),
            Self::VectorScoreExpandedCandidates => {
                vector_score_expanded_candidates::execute(ctx, args)
            }
            Self::VectorScoreExpandedCandidatesBatch => {
                vector_score_expanded_candidates_batch::execute(ctx, args)
            }
            Self::VectorSearchNodesAnn => vector_search_ann::execute(ctx, args),
            Self::VectorSearchNodesAnnBatch => vector_search_ann_batch::execute(ctx, args),
            Self::VectorSearchExpandedCandidatesAnn => {
                vector_search_expanded_candidates_ann::execute(ctx, args)
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::execute(ctx, args)
            }
            Self::VectorIndexStats => vector_index_stats::execute(ctx, args),
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Mutation,
                actual: ProcedureTier::Graph,
            }),
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                Err(ProcedureError::TierMismatch {
                    expected: ProcedureTier::Maintenance,
                    actual: ProcedureTier::Graph,
                })
            }
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
            | Self::VectorSearchNodesBatch
            | Self::VectorScoreNodes
            | Self::VectorScoreNodesBatch
            | Self::VectorScoreNeighbors
            | Self::VectorScoreNeighborsBatch
            | Self::VectorScoreCandidateState
            | Self::VectorScoreCandidateStateNodes
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Mutation,
            }),
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                Err(ProcedureError::TierMismatch {
                    expected: ProcedureTier::Maintenance,
                    actual: ProcedureTier::Mutation,
                })
            }
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
            Self::RebuildRecommendedVectorIndexes => {
                rebuild_vector_indexes::execute_recommended(ctx, args)
            }
            Self::Health
            | Self::FeatureStatus
            | Self::Verify
            | Self::VectorSearchNodes
            | Self::VectorSearchNodesBatch
            | Self::VectorScoreNodes
            | Self::VectorScoreNodesBatch
            | Self::VectorScoreNeighbors
            | Self::VectorScoreNeighborsBatch
            | Self::VectorScoreCandidateState
            | Self::VectorScoreCandidateStateNodes
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
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
