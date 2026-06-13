//! File-level corpus built from selected selene-db project files.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    push_docs(
        &mut inputs,
        Topic::Gql,
        &[
            (
                "GQL file-level roots cover runtime sessions, native built-in dispatch, \
                 procedure scoring rows, and parser/analyzer boundaries.",
                None,
            ),
            (
                "The current GQL engine keeps ISO syntax strict while exposing native selene \
                 procedures through registry-backed CALL execution.",
                None,
            ),
            (
                concat!(
                    "crates/selene-gql/src/runtime/session.rs\n",
                    include_str!("../../../../../crates/selene-gql/src/runtime/session.rs")
                ),
                Some("file-gql-session"),
            ),
            (
                concat!(
                    "crates/selene-gql/src/runtime/builtins/catalog.rs\n",
                    include_str!(
                        "../../../../../crates/selene-gql/src/runtime/builtins/catalog.rs"
                    )
                ),
                Some("file-gql-catalog"),
            ),
            (
                "crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/text_score_rows.rs \
                 benchmarks BM25 and vector/text scoring labels for oMLX query roots.",
                None,
            ),
            (
                "crates/selene-gql/src/analyzer lowers graph patterns and procedure calls before \
                 runtime sessions execute a plan.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::Vector,
        &[
            (
                "Vector file-level roots cover candidate sets, exact scoring, HNSW/IVF build \
                 helpers, and maintained graph-derived candidates.",
                None,
            ),
            (
                "First-class vector values are stored on graph nodes while derived ANN indexes \
                 remain rebuildable in-memory accelerators.",
                None,
            ),
            (
                concat!(
                    "crates/selene-graph/src/vector_search/types.rs\n",
                    include_str!("../../../../../crates/selene-graph/src/vector_search/types.rs")
                ),
                Some("file-vector-candidate-types"),
            ),
            (
                concat!(
                    "crates/selene-graph/src/vector_index/build.rs\n",
                    include_str!("../../../../../crates/selene-graph/src/vector_index/build.rs")
                ),
                Some("file-vector-index-build"),
            ),
            (
                "crates/selene-graph/src/vector_search/score.rs scores explicit, neighbor, and \
                 expanded vector candidates with exact metrics.",
                None,
            ),
            (
                "crates/selene-core/src/vector.rs owns the lower-is-better vector metric kernels \
                 used by graph reranking.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::AgentMemory,
        &[
            (
                "Agent-memory file-level roots cover maintained candidate-state ownership, \
                 required provenance edges, and batch current-state scoring rows.",
                None,
            ),
            (
                "Policy-neutral graph state lets memory retrieval compose support, currentness, \
                 provenance, text, and vector scoring without a hard-coded policy.",
                None,
            ),
            (
                concat!(
                    "crates/selene-graph/src/candidate_state.rs\n",
                    include_str!("../../../../../crates/selene-graph/src/candidate_state.rs")
                ),
                Some("file-memory-candidate-state"),
            ),
            (
                concat!(
                    "crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/state_batch_rows.rs\n",
                    include_str!(
                        "../../../../../crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/state_batch_rows.rs"
                    )
                ),
                Some("file-memory-state-batch-rows"),
            ),
            (
                "crates/selene-graph/src/candidate_state_shared.rs exposes shared candidate-state \
                 views for graph readers.",
                None,
            ),
            (
                "crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/fixture/text_exec.rs \
                 measures text scoring over maintained current-state candidates.",
                None,
            ),
        ],
    );
    push_docs(
        &mut inputs,
        Topic::Code,
        &[
            (
                "Workflow file-level roots cover local embedding setup, provider configuration, \
                 benchmark hygiene, and repository guard scripts.",
                None,
            ),
            (
                "Local benchmark evidence stays opt-in and provider-free for CI while tracked \
                 docs record stable commands and benchmark IDs.",
                None,
            ),
            (
                concat!(
                    "crates/selene-testing/src/local_omlx/client.rs\n",
                    include_str!("../../../../../crates/selene-testing/src/local_omlx/client.rs")
                ),
                Some("file-code-embedding-client"),
            ),
            (
                concat!(
                    "crates/selene-testing/src/local_omlx/config.rs\n",
                    include_str!("../../../../../crates/selene-testing/src/local_omlx/config.rs")
                ),
                Some("file-code-embedding-config"),
            ),
            (
                "scripts/run-benches.sh serializes Criterion binaries, selects profiles, and \
                 documents vector-scale presets.",
                None,
            ),
            (
                ".github/scripts/check-benchmarks-doc.sh verifies every registered benchmark is \
                 represented in BENCHMARKS.md.",
                None,
            ),
        ],
    );
    inputs.extend(queries());
    inputs
}

fn push_docs(
    inputs: &mut Vec<CorpusInput>,
    topic: Topic,
    docs: &[(&'static str, Option<&'static str>)],
) {
    inputs.extend(
        docs.iter()
            .map(|(text, target_key)| CorpusInput::document(topic, *text, *target_key)),
    );
}

fn queries() -> [CorpusInput; 8] {
    [
        query(
            Topic::Gql,
            "Which runtime file configures sessions, plan cache capacity, and GQL execution entry points?",
            "file-gql-session",
        ),
        query(
            Topic::Gql,
            "Which catalog file maps native selene built-in procedure metadata to dispatch kinds?",
            "file-gql-catalog",
        ),
        query(
            Topic::Vector,
            "Which vector search file defines canonical candidate sets and search option structs?",
            "file-vector-candidate-types",
        ),
        query(
            Topic::Vector,
            "Which vector index file builds derived Flat, HNSW, and IVF accelerators from graph values?",
            "file-vector-index-build",
        ),
        query(
            Topic::AgentMemory,
            "Which graph file owns maintained candidate-state specs, edge rules, and provider rebuilds?",
            "file-memory-candidate-state",
        ),
        query(
            Topic::AgentMemory,
            "Which GQL benchmark file creates labels for current-state and provenance-state vector batch rows?",
            "file-memory-state-batch-rows",
        ),
        query(
            Topic::Code,
            "Which local embedding file implements the curl-backed OpenRouter and oMLX clients?",
            "file-code-embedding-client",
        ),
        query(
            Topic::Code,
            "Which local embedding config file reads provider, model, corpus, batch-size, and API-key environment variables?",
            "file-code-embedding-config",
        ),
    ]
}

fn query(topic: Topic, text: &'static str, target_key: &'static str) -> CorpusInput {
    CorpusInput::query(topic, text, Some(target_key))
}
