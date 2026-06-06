use selene_core::Value;

use crate::procedure_registry::{ProcedureError, ProcedureHandle};
use crate::{
    GraphContext, MaintenanceContext, MutationContext, ProcedureMetadata, ProcedureMutability,
    ProcedureOutputColumn, ProcedureOutputSchema, ProcedureParameter, ProcedureResult,
    ProcedureSignature, ProcedureTier,
};

use super::{
    create_index, create_text_index, create_vector_index, drop_index, drop_text_index,
    drop_vector_index, feature_status, health, json_contains_nodes, json_path_exists_nodes,
    rebuild_vector_indexes, text_index_stats, text_search, vector_candidate_states,
    vector_index_stats, vector_score_candidate_state, vector_score_candidate_state_expanded,
    vector_score_candidate_state_expanded_batch, vector_score_candidate_state_nodes,
    vector_score_expanded_candidates, vector_score_expanded_candidates_batch,
    vector_score_neighbors, vector_score_neighbors_batch, vector_score_nodes,
    vector_score_nodes_batch, vector_search, vector_search_ann, vector_search_ann_batch,
    vector_search_batch, vector_search_candidate_state_expanded_ann,
    vector_search_expanded_candidates_ann, vector_search_expanded_candidates_ann_batch, verify,
};

mod specs;

pub(in crate::runtime) use specs::BUILTIN_SPECS;

/// One native platform built-in procedure, identified by its dispatch kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BuiltinKind {
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
    /// `selene.vector_score_candidate_state_expanded` — exact scoring for composed maintained state and expanded roots.
    VectorScoreCandidateStateExpanded,
    /// `selene.vector_score_candidate_state_expanded_batch` — batched exact scoring for composed maintained state and expanded roots.
    VectorScoreCandidateStateExpandedBatch,
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
    /// `selene.vector_search_candidate_state_expanded_ann` — state-gated ANN-root expanded search.
    VectorSearchCandidateStateExpandedAnn,
    /// `selene.vector_search_expanded_candidates_ann_batch` — batched ANN-root graph-expanded search.
    VectorSearchExpandedCandidatesAnnBatch,
    /// `selene.vector_index_stats` — vector index memory/cardinality stats.
    VectorIndexStats,
    /// `selene.text_index_stats` — text index memory/cardinality stats.
    TextIndexStats,
    /// `selene.json_contains_nodes` — exact JSON containment over node properties.
    JsonContainsNodes,
    /// `selene.json_path_exists_nodes` — exact JSON path-existence over node properties.
    JsonPathExistsNodes,
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
    /// `selene.create_text_index` — mutation-tier text-index creation.
    CreateTextIndex,
    /// `selene.drop_text_index` — mutation-tier text-index drop.
    DropTextIndex,
    /// `selene.text_search_nodes` — exact BM25 text search over node properties.
    TextSearchNodes,
    /// `selene.text_score_nodes` — BM25 scoring for explicit node candidates.
    TextScoreNodes,
    /// `selene.text_score_nodes_batch` — batched BM25 scoring for explicit node candidates.
    TextScoreNodesBatch,
    /// `selene.text_score_candidate_state_expanded_batch` — batched BM25 scoring for maintained state composed with expanded roots.
    TextScoreCandidateStateExpandedBatch,
}

impl BuiltinKind {
    /// Build the planner-visible metadata for this built-in.
    ///
    /// Static parameter / output columns are converted with the
    /// `with_description` / `with_default_doc` / `with_default` rules, the
    /// `since_version` rides on the [`ProcedureSignature`], and tier/mutability
    /// come from the procedure.
    pub(in crate::runtime) fn metadata(
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
    pub(in crate::runtime) const fn tier(self) -> ProcedureTier {
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
            | Self::VectorScoreCandidateStateExpanded
            | Self::VectorScoreCandidateStateExpandedBatch
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchCandidateStateExpandedAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats
            | Self::TextIndexStats
            | Self::JsonContainsNodes
            | Self::JsonPathExistsNodes
            | Self::TextSearchNodes
            | Self::TextScoreNodes
            | Self::TextScoreNodesBatch
            | Self::TextScoreCandidateStateExpandedBatch => ProcedureTier::Graph,
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                ProcedureTier::Maintenance
            }
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex
            | Self::CreateTextIndex
            | Self::DropTextIndex => ProcedureTier::Mutation,
        }
    }

    /// Declared mutability.
    pub(in crate::runtime) const fn mutability(self) -> ProcedureMutability {
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
            | Self::VectorScoreCandidateStateExpanded
            | Self::VectorScoreCandidateStateExpandedBatch
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchCandidateStateExpandedAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats
            | Self::TextIndexStats
            | Self::JsonContainsNodes
            | Self::JsonPathExistsNodes
            | Self::TextSearchNodes
            | Self::TextScoreNodes
            | Self::TextScoreNodesBatch
            | Self::TextScoreCandidateStateExpandedBatch => ProcedureMutability::Read,
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                ProcedureMutability::MaintenanceWrite
            }
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex
            | Self::CreateTextIndex
            | Self::DropTextIndex => ProcedureMutability::SchemaWrite,
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
            Self::VectorScoreCandidateStateExpanded => {
                vector_score_candidate_state_expanded::signature()
            }
            Self::VectorScoreCandidateStateExpandedBatch => {
                vector_score_candidate_state_expanded_batch::signature()
            }
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
            Self::VectorSearchCandidateStateExpandedAnn => {
                vector_search_candidate_state_expanded_ann::signature()
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::signature()
            }
            Self::VectorIndexStats => vector_index_stats::signature(),
            Self::TextIndexStats => text_index_stats::signature(),
            Self::JsonContainsNodes => json_contains_nodes::signature(),
            Self::JsonPathExistsNodes => json_path_exists_nodes::signature(),
            Self::RebuildVectorIndexes => rebuild_vector_indexes::signature(),
            Self::RebuildRecommendedVectorIndexes => {
                rebuild_vector_indexes::recommended_signature()
            }
            Self::CreateIndex => create_index::signature(),
            Self::DropIndex => drop_index::signature(),
            Self::CreateVectorIndex => create_vector_index::signature(),
            Self::DropVectorIndex => drop_vector_index::signature(),
            Self::CreateTextIndex => create_text_index::signature(),
            Self::DropTextIndex => drop_text_index::signature(),
            Self::TextSearchNodes => text_search::signature(),
            Self::TextScoreNodes => text_search::score_signature(),
            Self::TextScoreNodesBatch => text_search::score_batch_signature(),
            Self::TextScoreCandidateStateExpandedBatch => {
                text_search::score_state_expanded_batch_signature()
            }
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
            Self::VectorScoreCandidateStateExpanded => {
                vector_score_candidate_state_expanded::output_columns()
            }
            Self::VectorScoreCandidateStateExpandedBatch => {
                vector_score_candidate_state_expanded_batch::output_columns()
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
            Self::VectorSearchCandidateStateExpandedAnn => {
                vector_search_candidate_state_expanded_ann::output_columns()
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::output_columns()
            }
            Self::VectorIndexStats => vector_index_stats::output_columns(),
            Self::TextIndexStats => text_index_stats::output_columns(),
            Self::JsonContainsNodes => json_contains_nodes::output_columns(),
            Self::JsonPathExistsNodes => json_path_exists_nodes::output_columns(),
            Self::RebuildVectorIndexes | Self::RebuildRecommendedVectorIndexes => {
                rebuild_vector_indexes::output_columns()
            }
            Self::CreateIndex => create_index::output_columns(),
            Self::DropIndex => drop_index::output_columns(),
            Self::CreateVectorIndex => create_vector_index::output_columns(),
            Self::DropVectorIndex => drop_vector_index::output_columns(),
            Self::CreateTextIndex => create_text_index::output_columns(),
            Self::DropTextIndex => drop_text_index::output_columns(),
            Self::TextSearchNodes | Self::TextScoreNodes => text_search::output_columns(),
            Self::TextScoreNodesBatch | Self::TextScoreCandidateStateExpandedBatch => {
                text_search::score_batch_output_columns()
            }
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
    pub(in crate::runtime) fn execute_graph(
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
            Self::VectorScoreCandidateStateExpanded => {
                vector_score_candidate_state_expanded::execute(ctx, args)
            }
            Self::VectorScoreCandidateStateExpandedBatch => {
                vector_score_candidate_state_expanded_batch::execute(ctx, args)
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
            Self::VectorSearchCandidateStateExpandedAnn => {
                vector_search_candidate_state_expanded_ann::execute(ctx, args)
            }
            Self::VectorSearchExpandedCandidatesAnnBatch => {
                vector_search_expanded_candidates_ann_batch::execute(ctx, args)
            }
            Self::VectorIndexStats => vector_index_stats::execute(ctx, args),
            Self::TextIndexStats => text_index_stats::execute(ctx, args),
            Self::JsonContainsNodes => json_contains_nodes::execute(ctx, args),
            Self::JsonPathExistsNodes => json_path_exists_nodes::execute(ctx, args),
            Self::TextSearchNodes => text_search::execute(ctx, args),
            Self::TextScoreNodes => text_search::execute_score(ctx, args),
            Self::TextScoreNodesBatch => text_search::execute_score_batch(ctx, args),
            Self::TextScoreCandidateStateExpandedBatch => {
                text_search::execute_score_state_expanded_batch(ctx, args)
            }
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex
            | Self::CreateTextIndex
            | Self::DropTextIndex => Err(ProcedureError::TierMismatch {
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
    pub(in crate::runtime) fn execute_mutation(
        self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        match self {
            Self::CreateIndex => create_index::execute(ctx, args),
            Self::DropIndex => drop_index::execute(ctx, args),
            Self::CreateVectorIndex => create_vector_index::execute(ctx, args),
            Self::DropVectorIndex => drop_vector_index::execute(ctx, args),
            Self::CreateTextIndex => create_text_index::execute(ctx, args),
            Self::DropTextIndex => drop_text_index::execute(ctx, args),
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
            | Self::VectorScoreCandidateStateExpanded
            | Self::VectorScoreCandidateStateExpandedBatch
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchCandidateStateExpandedAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats
            | Self::TextIndexStats
            | Self::JsonContainsNodes
            | Self::JsonPathExistsNodes
            | Self::TextSearchNodes
            | Self::TextScoreNodes
            | Self::TextScoreNodesBatch
            | Self::TextScoreCandidateStateExpandedBatch => Err(ProcedureError::TierMismatch {
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
    pub(in crate::runtime) fn execute_maintenance(
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
            | Self::VectorScoreCandidateStateExpanded
            | Self::VectorScoreCandidateStateExpandedBatch
            | Self::VectorCandidateStates
            | Self::VectorScoreExpandedCandidates
            | Self::VectorScoreExpandedCandidatesBatch
            | Self::VectorSearchNodesAnn
            | Self::VectorSearchNodesAnnBatch
            | Self::VectorSearchExpandedCandidatesAnn
            | Self::VectorSearchCandidateStateExpandedAnn
            | Self::VectorSearchExpandedCandidatesAnnBatch
            | Self::VectorIndexStats
            | Self::TextIndexStats
            | Self::JsonContainsNodes
            | Self::JsonPathExistsNodes
            | Self::TextSearchNodes
            | Self::TextScoreNodes
            | Self::TextScoreNodesBatch
            | Self::TextScoreCandidateStateExpandedBatch => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Graph,
                actual: ProcedureTier::Maintenance,
            }),
            Self::CreateIndex
            | Self::DropIndex
            | Self::CreateVectorIndex
            | Self::DropVectorIndex
            | Self::CreateTextIndex
            | Self::DropTextIndex => Err(ProcedureError::TierMismatch {
                expected: ProcedureTier::Mutation,
                actual: ProcedureTier::Maintenance,
            }),
        }
    }
}
