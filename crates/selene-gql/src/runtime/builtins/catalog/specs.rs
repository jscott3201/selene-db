//! Static registration table for native `selene.*` built-ins.

use super::BuiltinKind;

/// Static descriptor binding a canonical procedure name to its dispatch kind.
pub(in crate::runtime) struct BuiltinSpec {
    /// Canonical multipart CALL name, e.g. `["selene", "health"]`.
    pub(in crate::runtime) name: &'static [&'static str],
    /// Human-readable summary used by `SHOW PROCEDURES`.
    pub(in crate::runtime) description: &'static str,
    /// Version where this procedure became available (carried on the signature).
    pub(in crate::runtime) since_version: &'static str,
    /// Dispatch kind.
    pub(in crate::runtime) kind: BuiltinKind,
}

/// The native platform built-ins in registration order. The first five entries
/// preserve the historical pack registration order (`health`,
/// `feature_status`, `verify`, `create_index`, `drop_index`; the former
/// `pack_history` built-in is not relocated). Vector built-ins are appended so
/// legacy handles keep their relative ordering.
pub(in crate::runtime) const BUILTIN_SPECS: [BuiltinSpec; 33] = [
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
        name: &["selene", "text_index_stats"],
        description: "Report text index memory and cardinality statistics.",
        since_version: "1.1.0",
        kind: BuiltinKind::TextIndexStats,
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
        name: &["selene", "create_text_index"],
        description: "Create a text index.",
        since_version: "1.1.0",
        kind: BuiltinKind::CreateTextIndex,
    },
    BuiltinSpec {
        name: &["selene", "drop_text_index"],
        description: "Drop a text index.",
        since_version: "1.1.0",
        kind: BuiltinKind::DropTextIndex,
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
        name: &["selene", "vector_search_candidate_state_expanded_ann"],
        description: "ANN-root expanded vector search composed with maintained candidate state.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorSearchCandidateStateExpandedAnn,
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
        name: &["selene", "vector_score_candidate_state_expanded"],
        description: "Score a maintained candidate-state set composed with graph-expanded roots.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreCandidateStateExpanded,
    },
    BuiltinSpec {
        name: &["selene", "vector_score_candidate_state_expanded_batch"],
        description: "Batched scoring for maintained candidate-state expanded roots.",
        since_version: "1.1.0",
        kind: BuiltinKind::VectorScoreCandidateStateExpandedBatch,
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
    BuiltinSpec {
        name: &["selene", "text_search_nodes"],
        description: "Exact BM25 full-text search over string node properties.",
        since_version: "1.1.0",
        kind: BuiltinKind::TextSearchNodes,
    },
    BuiltinSpec {
        name: &["selene", "text_score_nodes"],
        description: "Score explicit node candidates with BM25 full-text search.",
        since_version: "1.1.0",
        kind: BuiltinKind::TextScoreNodes,
    },
];
