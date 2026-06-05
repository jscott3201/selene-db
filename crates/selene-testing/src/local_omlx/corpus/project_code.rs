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

pub(super) fn alias_inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, docs) in [
        (
            Topic::Gql,
            &[
                (
                    "GQL runtime root lookups, CALL dispatch, and text scoring share source-shaped retrieval context.",
                    None,
                ),
                (
                    "Procedure surfaces stay native and ISO GQL compliant while planner caches reuse repeated work.",
                    None,
                ),
                (
                    "Session::with_plan_cache attaches PlanCache so repeated execute_source calls reuse parsed statement plans.",
                    Some("gql-source-plan-cache"),
                ),
                (
                    "CallPlanCache keys native CALL argument shapes, yield lists, registry generation, and graph id.",
                    Some("gql-call-plan-cache"),
                ),
                (
                    "BuiltinKind dispatch in runtime/builtins/catalog.rs routes selene procedure names to execute functions.",
                    Some("gql-builtin-dispatch"),
                ),
                (
                    "selene.text_score_candidate_state_expanded_batch batches BM25 over maintained state and expanded roots.",
                    Some("gql-state-text-batch"),
                ),
                (
                    "Repeated root lookup statements often ask similar graph questions and need parser work avoided.",
                    None,
                ),
                (
                    "Native procedure invocation metadata lists argument signatures, output columns, and yield names.",
                    None,
                ),
                (
                    "Text scoring after graph root expansion can be expressed as a CALL without grammar changes.",
                    None,
                ),
                (
                    "Catalog tests verify procedure names and mutability but do not execute retrieval policy.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Vector,
            &[
                (
                    "Vector retrieval context covers first-class values, metrics, canonical candidates, and graph expansion.",
                    None,
                ),
                (
                    "Embedding search narrows nodes through graph-authored roots before exact rerank computes distances.",
                    None,
                ),
                (
                    "Value::Vector(VectorValue) stores finite f32 embedding components as a native graph property.",
                    Some("vector-value-variant"),
                ),
                (
                    "VectorMetric evaluates cosine, squared_euclidean, and negative_inner_product as lower-is-better scores.",
                    Some("vector-metric-dispatch"),
                ),
                (
                    "VectorCandidateSet canonicalizes caller-provided NodeId inputs into sorted deduplicated candidates.",
                    Some("vector-candidate-canonical"),
                ),
                (
                    "score_vector_expanded_candidate_sets_batch_checked expands root sets and scores many query vectors.",
                    Some("vector-expanded-batch-api"),
                ),
                (
                    "Dense vectors live on nodes as embedding data and can be indexed by HNSW or IVF structures.",
                    None,
                ),
                (
                    "Semantic distance options compare direction, magnitude, and dot-product style similarity.",
                    None,
                ),
                (
                    "Caller supplied graph identifiers should be cleaned up before rerank loops walk candidates.",
                    None,
                ),
                (
                    "Many query embeddings can share one batched graph expansion and exact scoring boundary.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                (
                    "Agent memory retrieval composes graph state, provenance requirements, negative evidence, and vectors.",
                    None,
                ),
                (
                    "Current support facts are graph-authored candidates for semantic and lexical memory ranking.",
                    None,
                ),
                (
                    "MaintainedCandidateStateProvider owns named graph-derived candidate sets for memory retrieval.",
                    Some("memory-state-owner"),
                ),
                (
                    "CandidateStateSpec::exclude_outgoing removes candidates that carry disqualifying outgoing edges.",
                    Some("memory-negative-edge-filter"),
                ),
                (
                    "CandidateStateSpec::require_incoming and require_outgoing enforce provenance edge evidence.",
                    Some("memory-provenance-rules"),
                ),
                (
                    "vector_score_candidate_state_expanded_batch intersects maintained memory state with expanded roots.",
                    Some("memory-state-vector-batch"),
                ),
                (
                    "A memory candidate registry can keep active facts ready for repeated agent recall.",
                    None,
                ),
                (
                    "Negative evidence should prevent stale observations from appearing in current memory answers.",
                    None,
                ),
                (
                    "Grounding rules require evidence links before a remembered fact should be trusted.",
                    None,
                ),
                (
                    "Expanded graph roots and current memory sets can be combined before vector reranking.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Code,
            &[
                (
                    "Local benchmark workflow uses provider config, remote embedding setup, and sequential Criterion runs.",
                    None,
                ),
                (
                    "Repository guard scripts document benchmark targets and prevent accidental secret or artifact churn.",
                    None,
                ),
                (
                    "OpenRouterClient shells out to curl for setup-time embedding requests without adding an HTTP dependency.",
                    Some("code-openrouter-curl"),
                ),
                (
                    "EmbeddingBenchConfig reads SELENE_EMBEDDING_PROVIDER, corpus, models, batch size, and graph hint envs.",
                    Some("code-embedding-env-config"),
                ),
                (
                    "scripts/run-benches.sh serializes Criterion bench binaries and blocks concurrent cargo bench processes.",
                    Some("code-sequential-runner"),
                ),
                (
                    "check-benchmarks-doc.sh requires every registered benchmark target to appear in BENCHMARKS.md.",
                    Some("code-benchmark-doc-guard"),
                ),
                (
                    "Remote embedding setup should happen before timing loops so network latency does not pollute Criterion.",
                    None,
                ),
                (
                    "Environment variables select provider, model list, corpus profile, and graph hint fanout.",
                    None,
                ),
                (
                    "Benchmark runners should avoid parallel cargo bench execution because medians become noisy.",
                    None,
                ),
                (
                    "Documentation guards keep newly added benchmark binaries visible in the tracked benchmark guide.",
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
            text: "What should repeated root lookup execution enable to avoid rebuilding statement plans?",
            target_key: Some("gql-source-plan-cache"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Where are native procedure argument shapes remembered for reusable CALL execution?",
            target_key: Some("gql-call-plan-cache"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which runtime table sends a selene procedure name to implementation code?",
            target_key: Some("gql-builtin-dispatch"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which procedure ranks text after maintained memory state is intersected with expanded roots?",
            target_key: Some("gql-state-text-batch"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which value type lets a graph node carry embedding data directly?",
            target_key: Some("vector-value-variant"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Where do distance choices become lower-is-better semantic scores?",
            target_key: Some("vector-metric-dispatch"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which structure cleans duplicate node identifiers before exact reranking?",
            target_key: Some("vector-candidate-canonical"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which API expands graph roots once while scoring many query embeddings?",
            target_key: Some("vector-expanded-batch-api"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which component owns named active memory candidate sets?",
            target_key: Some("memory-state-owner"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which rule removes memories with disqualifying negative evidence links?",
            target_key: Some("memory-negative-edge-filter"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which rules keep only memories with incoming and outgoing evidence links?",
            target_key: Some("memory-provenance-rules"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which batch procedure combines current memory state with expanded roots before vector scoring?",
            target_key: Some("memory-state-vector-batch"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which helper sends remote embedding setup requests through the command-line HTTP tool?",
            target_key: Some("code-openrouter-curl"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which configuration reads provider, corpus, model, batch, and graph hint environment settings?",
            target_key: Some("code-embedding-env-config"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which script prevents multiple Criterion binaries from timing concurrently?",
            target_key: Some("code-sequential-runner"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which guard forces new benchmark targets to be written in the benchmark guide?",
            target_key: Some("code-benchmark-doc-guard"),
        },
    ]);
    inputs
}
