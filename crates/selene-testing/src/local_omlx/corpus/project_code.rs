//! Target-aware corpus shaped like current selene-db source files.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, docs) in [
        (
            Topic::Gql,
            &[
                (
                    "Parser and analyzer stages keep ISO GQL syntax strict before executor procedure calls run.",
                    None,
                ),
                (
                    "ProcedureRegistry is the planner seam for metadata lookup and native handle dispatch.",
                    None,
                ),
                (
                    "crates/selene-gql/src/runtime/session.rs exposes Session::with_plan_cache for source-string plan reuse.",
                    Some("gql-session-plan-cache"),
                ),
                (
                    "crates/selene-gql/src/runtime/call_plan_cache.rs stores reusable native CALL execution plans in CallPlanCache.",
                    Some("gql-call-plan-cache"),
                ),
                (
                    "crates/selene-gql/src/runtime/builtins/catalog.rs maps BuiltinKind values to native selene.* procedure dispatch.",
                    Some("gql-builtin-catalog"),
                ),
                (
                    "crates/selene-gql/src/runtime/builtins/text_search.rs implements selene.text_score_candidate_state_expanded_batch.",
                    Some("gql-text-state-batch"),
                ),
                (
                    "Statement execution disables cached call plans when active schema writes change procedure-visible state.",
                    None,
                ),
                (
                    "Native built-in surface tests pin procedure names, signatures, tiers, and mutability.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Vector,
            &[
                (
                    "HNSW and IVF graph indexes are rebuildable derived state over first-class vector values.",
                    None,
                ),
                (
                    "Exact rerank keeps full f64 score semantics after candidate production narrows the node set.",
                    None,
                ),
                (
                    "crates/selene-core/src/value.rs defines Value::Vector(VectorValue) and MAX_VECTOR_DIMENSION.",
                    Some("vector-value-type"),
                ),
                (
                    "crates/selene-core/src/vector.rs scores squared_euclidean, cosine, and negative_inner_product metrics.",
                    Some("vector-core-metrics"),
                ),
                (
                    "crates/selene-graph/src/vector_search/types.rs stores sorted deduplicated NodeId values in VectorCandidateSet.",
                    Some("vector-candidate-set"),
                ),
                (
                    "crates/selene-graph/src/vector_search/score.rs provides score_vector_expanded_candidate_sets_batch_checked.",
                    Some("vector-expanded-batch"),
                ),
                (
                    "Approximate vector roots can be expanded through graph topology before final cosine reranking.",
                    None,
                ),
                (
                    "Vector index registrations survive WAL and snapshot recovery while accelerators rebuild in memory.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                (
                    "omlx_current_support_facts filters stale, superseded, and contradicted memory facts in benchmarks.",
                    None,
                ),
                (
                    "OmlxGroundedBy and OmlxSupports edges preserve provenance and support paths for recalled facts.",
                    None,
                ),
                (
                    "crates/selene-graph/src/candidate_state.rs owns MaintainedCandidateStateProvider for graph-derived memory sets.",
                    Some("memory-state-provider"),
                ),
                (
                    "CandidateStateSpec::exclude_outgoing removes nodes with negative evidence edges from current support.",
                    Some("memory-exclude-outgoing"),
                ),
                (
                    "CandidateStateSpec::require_outgoing and require_incoming model grounded provenance requirements.",
                    Some("memory-required-edges"),
                ),
                (
                    "vector_score_candidate_state_expanded_batch composes maintained memory state with graph-expanded roots.",
                    Some("memory-vector-state-batch"),
                ),
                (
                    "Provider replay prunes node deletes and recomputes surviving candidate-state memberships.",
                    None,
                ),
                (
                    "Agent memory retrieval stays policy-neutral by composing graph state, text scoring, and vectors.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Code,
            &[
                (
                    "AGENTS.md records repo workflow rules for cargo clean, branch cleanup, and local goal logs.",
                    None,
                ),
                (
                    "The pre-push hook runs cargo clippy --workspace -- -D warnings before publishing branches.",
                    None,
                ),
                (
                    "crates/selene-testing/src/local_omlx/client.rs uses a curl-backed OpenRouterClient for setup-time embeddings.",
                    Some("code-openrouter-client"),
                ),
                (
                    "crates/selene-testing/src/local_omlx/config.rs reads SELENE_EMBEDDING_PROVIDER and SELENE_EMBEDDING_CORPUS.",
                    Some("code-embedding-config"),
                ),
                (
                    "scripts/run-benches.sh serializes Criterion bench binaries and rejects concurrent cargo bench processes.",
                    Some("code-run-benches"),
                ),
                (
                    ".github/scripts/check-benchmarks-doc.sh verifies committed benchmark targets are documented.",
                    Some("code-bench-doc-check"),
                ),
                (
                    "Local _goalslogs notes are ignored and must not be committed with benchmark evidence.",
                    None,
                ),
                (
                    "apply_patch is the preferred manual edit path for Codex source changes in this repository.",
                    None,
                ),
            ][..],
        ),
    ] {
        inputs.extend(docs.iter().map(|(text, target_key)| CorpusInput {
            topic,
            is_document: true,
            text,
            target_key: *target_key,
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which runtime session API caches parsed source statements for repeated GQL execution?",
            target_key: Some("gql-session-plan-cache"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which cache stores reusable execution plans for native CALL procedures?",
            target_key: Some("gql-call-plan-cache"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which catalog dispatches BuiltinKind entries to selene procedure implementations?",
            target_key: Some("gql-builtin-catalog"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which GQL built-in batches BM25 over maintained state and expanded roots?",
            target_key: Some("gql-text-state-batch"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which core value variant stores first-class embedding vectors?",
            target_key: Some("vector-value-type"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Where are cosine and negative inner product vector metrics scored?",
            target_key: Some("vector-core-metrics"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which graph type canonicalizes candidate NodeId sets for vector scoring?",
            target_key: Some("vector-candidate-set"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which graph API scores batched query vectors after expanding root sets?",
            target_key: Some("vector-expanded-batch"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which provider owns maintained graph-derived memory candidate sets?",
            target_key: Some("memory-state-provider"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which candidate-state rule removes memories with outgoing negative evidence?",
            target_key: Some("memory-exclude-outgoing"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which candidate-state rules require incoming or outgoing provenance edges?",
            target_key: Some("memory-required-edges"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which vector procedure composes maintained memory state with expanded graph roots?",
            target_key: Some("memory-vector-state-batch"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which local embedding client sends OpenRouter setup requests through curl?",
            target_key: Some("code-openrouter-client"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which config module reads the embedding provider and corpus environment variables?",
            target_key: Some("code-embedding-config"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which script keeps Criterion benchmark binaries from running at the same time?",
            target_key: Some("code-run-benches"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which script checks that benchmark targets are represented in BENCHMARKS.md?",
            target_key: Some("code-bench-doc-check"),
        },
    ]);
    inputs
}
