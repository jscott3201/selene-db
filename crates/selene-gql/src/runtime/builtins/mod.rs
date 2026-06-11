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
//! vector-search, vector-index stats, vector-index procedures, maintained
//! text-index stats/procedures, BM25 text-search, candidate BM25 scoring,
//! batched candidate BM25 scoring, maintained-state graph-expanded BM25 batch
//! scoring, Reciprocal Rank Fusion over ranked node lists, JSON containment
//! node search, JSON path search, and candidate-scoped JSON search are new
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
//!   `selene.vector_score_candidate_state_expanded`,
//!   `selene.vector_score_candidate_state_expanded_batch`,
//!   `selene.vector_candidate_states`,
//!   `selene.vector_score_expanded_candidates`,
//!   `selene.vector_score_expanded_candidates_batch`,
//!   `selene.vector_search_nodes_ann`, `selene.vector_search_nodes_ann_batch`,
//!   `selene.vector_search_expanded_candidates_ann`,
//!   `selene.vector_search_candidate_state_expanded_ann`,
//!   `selene.vector_search_expanded_candidates_ann_batch`,
//!   `selene.vector_index_stats`, `selene.text_index_stats`,
//!   `selene.json_contains_nodes`, `selene.json_path_exists_nodes`,
//!   `selene.json_path_contains_nodes`, `selene.json_path_value_nodes`,
//!   `selene.json_contains_candidate_nodes`,
//!   `selene.json_path_exists_candidate_nodes`,
//!   `selene.json_path_contains_candidate_nodes`,
//!   `selene.json_path_value_candidate_nodes`,
//!   `selene.compaction_stats`,
//!   `selene.text_search_nodes`, `selene.text_score_nodes`,
//!   `selene.text_score_nodes_batch`, and
//!   `selene.text_score_candidate_state_expanded_batch`,
//!   `selene.reciprocal_rank_fusion` are read-only
//!   graph-tier ([`ProcedureTier::Graph`] + [`ProcedureMutability::Read`]);
//!   they never mutate and never re-enter `begin_write`.
//! - `selene.create_index`, `selene.drop_index`, `selene.create_vector_index`,
//!   `selene.drop_vector_index`, `selene.create_text_index`, and
//!   `selene.drop_text_index` are mutation-tier
//!   ([`ProcedureTier::Mutation`] + [`ProcedureMutability::SchemaWrite`]); they
//!   route every write through [`MutationContext::mutator`] — emitting index
//!   schema changes through the single mutation funnel (Hard Rule 11). They
//!   never bypass the funnel and never re-enter `begin_write`.
//! - `selene.rebuild_vector_indexes`,
//!   `selene.rebuild_recommended_vector_indexes`, and `selene.compact` are
//!   maintenance-tier
//!   ([`ProcedureTier::Maintenance`] +
//!   [`ProcedureMutability::MaintenanceWrite`]); they rebuild derived vector
//!   index state or compact dead graph rows through [`MaintenanceContext`]
//!   without graph changes, WAL entries, or schema-version bumps.
//!
//! `pack_history` is **not** relocated: it read the pack-lifecycle audit, which
//! is removed in the teardown.

mod catalog;
mod compaction;
mod create_index;
mod create_text_index;
mod create_vector_index;
mod drop_index;
mod drop_text_index;
mod drop_vector_index;
mod feature_status;
mod health;
mod json_candidate_nodes;
mod json_contains_nodes;
mod json_path_common;
mod json_path_contains_nodes;
mod json_path_exists_nodes;
mod json_path_value_nodes;
mod meta;
mod rebuild_vector_indexes;
mod reciprocal_rank_fusion;
mod text_index_stats;
mod text_search;
mod vector_candidate_state_common;
mod vector_candidate_states;
mod vector_common;
mod vector_index_stats;
mod vector_score_candidate_state;
mod vector_score_candidate_state_expanded;
mod vector_score_candidate_state_expanded_batch;
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
mod vector_search_candidate_state_expanded_ann;
mod vector_search_expanded_candidates_ann;
mod vector_search_expanded_candidates_ann_batch;
mod verify;

pub(super) use catalog::{BUILTIN_SPECS, BuiltinKind};

use crate::ProcedureResult;

/// Build a unit (single empty-row) result, mirroring the pack built-ins' return
/// shape for procedures with no output columns.
pub(super) fn unit_result() -> ProcedureResult {
    ProcedureResult {
        rows: vec![Vec::new()],
    }
}
