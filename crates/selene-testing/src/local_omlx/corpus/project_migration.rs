//! Legacy-alias corpus for archived-code to current-engine retrieval.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    push_docs(&mut inputs, Topic::Gql, gql_docs());
    push_docs(&mut inputs, Topic::Vector, vector_docs());
    push_docs(&mut inputs, Topic::AgentMemory, memory_docs());
    push_docs(&mut inputs, Topic::Code, code_docs());
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

fn gql_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Migration GQL roots connect archived prototype terminology to current ISO GQL \
             runtime sessions, native procedure dispatch, and text/vector scoring surfaces.",
            None,
        ),
        (
            "A second migration GQL root mentions old extension-pack vocabulary only as an \
             alias for current in-tree built-ins and planner cache behavior.",
            None,
        ),
        (
            "crates/selene-gql/src/runtime/session.rs now owns Session::with_plan_cache for \
             repeated source-string GQL execution without relying on archived service caches.",
            Some("migration-gql-session-plan-cache"),
        ),
        (
            "crates/selene-gql/src/runtime/builtins/catalog.rs maps BuiltinKind variants to \
             native selene.* procedure metadata and graph/mutation/maintenance tiers.",
            Some("migration-gql-builtin-catalog"),
        ),
        (
            "crates/selene-gql/src/runtime/builtins/text_search.rs executes \
             selene.text_score_candidate_state_expanded_batch over maintained state, expanded \
             roots, and BM25 text scoring.",
            Some("migration-gql-text-state-batch"),
        ),
        (
            "crates/selene-gql/src/runtime/builtins/vector_score_nodes.rs handles native vector \
             scoring procedure arguments before exact candidate rerank returns node hits.",
            Some("migration-gql-vector-score"),
        ),
        (
            "stale archived extension-pack manifest documentation talked about loadable \
             procedure packs and plugin registry dispatch; current selene-db keeps built-ins \
             in-tree instead.",
            None,
        ),
        (
            "superseded prototype query-cache notes described service-local replay handles, \
             not the current Session::with_plan_cache runtime API.",
            None,
        ),
    ]
}

fn vector_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Migration vector roots connect the old vector index experiment to current \
             first-class Value::Vector storage, graph indexes, and exact rerank kernels.",
            None,
        ),
        (
            "A second migration vector root treats archive HNSW/PQ naming as an alias for \
             current rebuildable in-memory accelerators over graph-owned vector values.",
            None,
        ),
        (
            "crates/selene-core/src/value.rs defines Value::Vector(VectorValue) and \
             MAX_VECTOR_DIMENSION, making embeddings a first-class engine value.",
            Some("migration-vector-value-type"),
        ),
        (
            "crates/selene-core/src/vector.rs defines VectorMetric and safe exact distance \
             kernels for squared_euclidean, cosine, and negative_inner_product scoring.",
            Some("migration-vector-core-metrics"),
        ),
        (
            "crates/selene-graph/src/vector_search/types.rs provides VectorCandidateSet for \
             sorted, deduplicated NodeId candidates before scoring.",
            Some("migration-vector-candidate-set"),
        ),
        (
            "crates/selene-graph/src/vector_index.rs registers Flat, HNSW, and IVF vector \
             indexes as graph indexes over label/property vector values.",
            Some("migration-vector-index-registration"),
        ),
        (
            "stale archived vector-store code kept embeddings beside graph nodes and exposed \
             separate vector APIs; current selene-db stores vectors as Value data.",
            None,
        ),
        (
            "superseded PQ-only prototype notes compressed vectors for experiments but did \
             not define the current production index registration surface.",
            None,
        ),
    ]
}

fn memory_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Migration memory roots map old agent-memory notes to current graph-derived \
             candidate states, negative evidence, provenance, and current support facts.",
            None,
        ),
        (
            "A second migration memory root treats vector memory filters as aliases for \
             policy-neutral graph state composed with text and vector scoring.",
            None,
        ),
        (
            "crates/selene-graph/src/candidate_state.rs defines CandidateStateSpec with \
             required labels, required edge labels, and excluded edge labels.",
            Some("migration-memory-candidate-state-spec"),
        ),
        (
            "crates/selene-graph/src/candidate_state.rs owns MaintainedCandidateStateProvider, \
             provider replay, and graph-derived candidate membership.",
            Some("migration-memory-provider"),
        ),
        (
            "crates/selene-graph/src/candidate_state_shared.rs exposes named candidate states \
             through generation-checked SharedGraph accessors.",
            Some("migration-memory-shared-generation"),
        ),
        (
            "crates/selene-gql/src/runtime/builtins/vector_score_candidate_state_expanded.rs \
             composes maintained candidate state with graph-expanded roots before vector rerank.",
            Some("migration-memory-gql-state-expanded-vector"),
        ),
        (
            "stale archived memory-vector filter notes kept currentness as an external agent \
             policy; current selene-db models negative evidence inside graph state.",
            None,
        ),
        (
            "contradictory prototype memory rows can remain useful for audit, but maintained \
             current-state retrieval should exclude them from answer candidates.",
            None,
        ),
    ]
}

fn code_docs() -> &'static [(&'static str, Option<&'static str>)] {
    &[
        (
            "Migration workflow roots connect old MCP/service setup terminology to current \
             local embedding configuration, benchmark runners, hooks, and ignored goal logs.",
            None,
        ),
        (
            "A second migration workflow root mentions archived endpoint setup only as context \
             for current oMLX/OpenRouter corpus and benchmark configuration.",
            None,
        ),
        (
            "crates/selene-testing/src/local_omlx/config.rs reads SELENE_EMBEDDING_PROVIDER, \
             SELENE_EMBEDDING_CORPUS, model lists, batch size, and graph hint settings.",
            Some("migration-code-embedding-config"),
        ),
        (
            "crates/selene-testing/src/local_omlx/client.rs owns the oMLX and OpenRouter \
             embedding clients, request batching, and embedding response parsing.",
            Some("migration-code-embedding-client"),
        ),
        (
            "scripts/run-benches.sh is the sanctioned Criterion runner and serializes bench \
             binaries to avoid polluted wall-clock medians.",
            Some("migration-code-bench-runner"),
        ),
        (
            "AGENTS.md records selene-db workflow rules for cargo clean after merged PRs, \
             branch cleanup, local-only goal logs, and CI-safe embedding rows.",
            Some("migration-code-agent-workflow"),
        ),
        (
            "stale archived MCP service configuration was built from old code and is not \
             authoritative for current selene-db benchmark or API behavior.",
            None,
        ),
        (
            "superseded local endpoint notes can explain historical setup, but current corpus \
             selection flows through SELENE_EMBEDDING_CORPUS.",
            None,
        ),
    ]
}

fn queries() -> [CorpusInput; 16] {
    [
        query(
            Topic::Gql,
            "The archived service reused query work; which current runtime API caches repeated source GQL statements?",
            "migration-gql-session-plan-cache",
        ),
        query(
            Topic::Gql,
            "What replaced the old extension-pack registry for native selene procedure dispatch?",
            "migration-gql-builtin-catalog",
        ),
        query(
            Topic::Gql,
            "Which current built-in executes BM25 scoring over maintained memory state and expanded roots?",
            "migration-gql-text-state-batch",
        ),
        query(
            Topic::Gql,
            "Where does current GQL parse native vector scoring procedure arguments before exact rerank?",
            "migration-gql-vector-score",
        ),
        query(
            Topic::Vector,
            "The prototype treated embeddings as side data; where are vectors now represented as a real Value?",
            "migration-vector-value-type",
        ),
        query(
            Topic::Vector,
            "Which current core file defines cosine and negative-inner-product vector metric kernels?",
            "migration-vector-core-metrics",
        ),
        query(
            Topic::Vector,
            "Which graph vector type deduplicates NodeId candidates before scoring?",
            "migration-vector-candidate-set",
        ),
        query(
            Topic::Vector,
            "Where are Flat, HNSW, and IVF vector indexes registered over graph label/property pairs?",
            "migration-vector-index-registration",
        ),
        query(
            Topic::AgentMemory,
            "What current graph primitive describes required labels and negative-evidence edges for memory candidates?",
            "migration-memory-candidate-state-spec",
        ),
        query(
            Topic::AgentMemory,
            "Which provider owns graph-derived memory candidate membership after archived vector filters?",
            "migration-memory-provider",
        ),
        query(
            Topic::AgentMemory,
            "Which shared graph accessor checks candidate-state generation before exposing named memory sets?",
            "migration-memory-shared-generation",
        ),
        query(
            Topic::AgentMemory,
            "Which GQL vector procedure combines maintained memory state with graph-expanded roots?",
            "migration-memory-gql-state-expanded-vector",
        ),
        query(
            Topic::Code,
            "Where does the current embedding benchmark read provider, corpus, models, batch size, and graph hints?",
            "migration-code-embedding-config",
        ),
        query(
            Topic::Code,
            "Which current client code handles local oMLX and OpenRouter embedding requests?",
            "migration-code-embedding-client",
        ),
        query(
            Topic::Code,
            "Which runner should be used instead of direct cargo bench for local retrieval benchmark rows?",
            "migration-code-bench-runner",
        ),
        query(
            Topic::Code,
            "Which repo guide records cargo clean, branch cleanup, and ignored goal-log workflow rules?",
            "migration-code-agent-workflow",
        ),
    ]
}

fn query(topic: Topic, text: &'static str, target_key: &'static str) -> CorpusInput {
    CorpusInput::query(topic, text, Some(target_key))
}
