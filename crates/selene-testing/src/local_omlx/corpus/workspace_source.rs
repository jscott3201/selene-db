//! Workspace-source corpus extracted from the current selene-db checkout.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::{CorpusInput, Topic};

const CONTEXT_LINES: usize = 7;

struct SourceDoc {
    topic: Topic,
    path: &'static str,
    target_key: &'static str,
    needles: &'static [&'static str],
    query: &'static str,
}

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    push_root_docs(&mut inputs);
    for doc in SOURCE_DOCS {
        inputs.push(CorpusInput::document(
            doc.topic,
            source_excerpt(doc),
            Some(doc.target_key),
        ));
    }
    inputs.extend(
        SOURCE_DOCS
            .iter()
            .map(|doc| CorpusInput::query(doc.topic, doc.query, Some(doc.target_key))),
    );
    inputs
}

fn push_root_docs(inputs: &mut Vec<CorpusInput>) {
    for (topic, roots) in [
        (
            Topic::Gql,
            &[
                "Workspace GQL roots cover runtime source execution, shared CALL plan caches, native built-in dispatch, and maintained-state BM25 procedure scoring.",
                "Source-derived GQL retrieval should distinguish planner/cache files from parser, analyzer, and benchmark-only fixtures.",
            ][..],
        ),
        (
            Topic::Vector,
            &[
                "Workspace vector roots cover first-class Value::Vector storage, exact metric kernels, candidate canonicalization, and graph-expanded batch reranking.",
                "Source-derived vector retrieval should separate core metric semantics from graph index and scoring adapters.",
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                "Workspace memory roots cover maintained candidate-state specs, provider-owned current sets, generation checks, and graph/state composition.",
                "Source-derived memory retrieval should separate policy-neutral state ownership from GQL benchmark rows that consume the state.",
            ][..],
        ),
        (
            Topic::Code,
            &[
                "Workspace code roots cover local embedding providers, source-corpus configuration, Criterion runner hygiene, and benchmark documentation guards.",
                "Source-derived workflow retrieval should distinguish live benchmark setup helpers from scripts that enforce local and CI hygiene.",
            ][..],
        ),
    ] {
        inputs.extend(
            roots
                .iter()
                .map(|text| CorpusInput::document(topic, *text, None)),
        );
    }
}

fn source_excerpt(doc: &SourceDoc) -> String {
    let source_path = workspace_root().join(doc.path);
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|error| panic!("read {} failed: {error}", source_path.display()));
    let lines = source.lines().collect::<Vec<_>>();
    let mut selected = BTreeSet::new();
    for needle in doc.needles {
        let index = lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("missing needle `{needle}` in {}", doc.path));
        let start = index.saturating_sub(CONTEXT_LINES);
        let end = index.saturating_add(CONTEXT_LINES + 1).min(lines.len());
        selected.extend(start..end);
    }

    let mut text = format!("{}\n", doc.path);
    let mut previous = None;
    for index in selected {
        if previous.is_some_and(|last| index > last + 1) {
            text.push_str("---\n");
        }
        writeln!(&mut text, "{:04}: {}", index + 1, lines[index])
            .expect("writing to String cannot fail");
        previous = Some(index);
    }
    text
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const SOURCE_DOCS: &[SourceDoc] = &[
    SourceDoc {
        topic: Topic::Gql,
        path: "crates/selene-gql/src/runtime/session.rs",
        target_key: "workspace-gql-session-plan-cache",
        needles: &["pub fn with_plan_cache", "pub fn with_call_plan_cache"],
        query: "Which live runtime file enables source-string plan cache reuse and shared CALL plan cache reuse for GQL sessions?",
    },
    SourceDoc {
        topic: Topic::Gql,
        path: "crates/selene-gql/src/runtime/call_plan_cache.rs",
        target_key: "workspace-gql-call-plan-cache",
        needles: &["pub struct CallPlanCache", "pub(crate) fn get_source"],
        query: "Which source file owns the shared native CALL plan cache and its source-string fast path?",
    },
    SourceDoc {
        topic: Topic::Gql,
        path: "crates/selene-gql/src/runtime/builtins/catalog.rs",
        target_key: "workspace-gql-builtin-catalog",
        needles: &[
            "TextScoreCandidateStateExpandedBatch",
            "ProcedureTier::Graph",
        ],
        query: "Which built-in catalog source maps maintained-state BM25 procedure kinds to graph-tier read dispatch?",
    },
    SourceDoc {
        topic: Topic::Gql,
        path: "crates/selene-gql/src/runtime/builtins/text_search.rs",
        target_key: "workspace-gql-state-bm25-batch",
        needles: &[
            "SCORE_STATE_EXPANDED_BATCH_PROC_NAME",
            "execute_score_state_expanded_batch",
        ],
        query: "Which built-in source executes batched BM25 scoring over maintained candidate state and expanded roots?",
    },
    SourceDoc {
        topic: Topic::Vector,
        path: "crates/selene-core/src/value.rs",
        target_key: "workspace-vector-value-type",
        needles: &["pub const MAX_VECTOR_DIMENSION", "Vector(VectorValue)"],
        query: "Which core source file makes dense embeddings a bounded first-class Value variant?",
    },
    SourceDoc {
        topic: Topic::Vector,
        path: "crates/selene-core/src/vector.rs",
        target_key: "workspace-vector-metric-kernels",
        needles: &[
            "pub enum VectorMetric",
            "VectorMetric::NegativeInnerProduct => -dot",
        ],
        query: "Which core source defines lower-is-better vector metrics and the negative-inner-product adapter?",
    },
    SourceDoc {
        topic: Topic::Vector,
        path: "crates/selene-graph/src/vector_search/types.rs",
        target_key: "workspace-vector-candidate-set",
        needles: &["pub struct VectorCandidateSet", "pub fn from_nodes"],
        query: "Which graph vector source canonicalizes NodeId candidates before exact reranking?",
    },
    SourceDoc {
        topic: Topic::Vector,
        path: "crates/selene-graph/src/vector_search/score.rs",
        target_key: "workspace-vector-expanded-batch",
        needles: &["score_vector_expanded_candidate_sets_batch_checked"],
        query: "Which graph vector source expands root candidate sets before batched exact vector scoring?",
    },
    SourceDoc {
        topic: Topic::AgentMemory,
        path: "crates/selene-graph/src/candidate_state.rs",
        target_key: "workspace-memory-candidate-state-spec",
        needles: &["pub struct CandidateStateSpec", "pub fn exclude_outgoing"],
        query: "Which graph source defines maintained candidate-state specs and negative-edge exclusion rules?",
    },
    SourceDoc {
        topic: Topic::AgentMemory,
        path: "crates/selene-graph/src/candidate_state.rs",
        target_key: "workspace-memory-provider-candidate-set",
        needles: &["pub fn candidate_set", "pub fn candidate_set_at_generation"],
        query: "Which provider source returns named maintained state as generation-checked vector candidate sets?",
    },
    SourceDoc {
        topic: Topic::AgentMemory,
        path: "crates/selene-graph/src/candidate_state_shared.rs",
        target_key: "workspace-memory-shared-generation",
        needles: &["pub fn vector_candidate_set", "snapshot.meta.generation"],
        query: "Which shared graph source checks snapshot generation before exposing a maintained candidate state?",
    },
    SourceDoc {
        topic: Topic::AgentMemory,
        path: "crates/selene-gql/benches/procedure_call_repeat/vector_omlx_query_roots/state_batch_rows.rs",
        target_key: "workspace-memory-state-batch-bench",
        needles: &["bench_current_state_batch", "basecurbp"],
        query: "Which GQL benchmark source labels current-state vector batch rows for oMLX query roots?",
    },
    SourceDoc {
        topic: Topic::Code,
        path: "crates/selene-testing/src/local_omlx/config.rs",
        target_key: "workspace-code-embedding-config",
        needles: &["pub struct EmbeddingBenchConfig", "SELENE_EMBEDDING_CORPUS"],
        query: "Which local embedding config source reads provider, model, corpus, batch-size, and graph-hint settings?",
    },
    SourceDoc {
        topic: Topic::Code,
        path: "crates/selene-testing/src/local_omlx/client.rs",
        target_key: "workspace-code-openrouter-client",
        needles: &["pub struct OpenRouterClient", "parse_embedding_response"],
        query: "Which local embedding client source owns the curl-backed OpenRouter setup path and response parsing?",
    },
    SourceDoc {
        topic: Topic::Code,
        path: "scripts/run-benches.sh",
        target_key: "workspace-code-bench-runner",
        needles: &["SELENE_BENCH_FORCE_CONFLICT", "pgrep -f \"cargo bench\""],
        query: "Which benchmark runner script rejects concurrent cargo bench processes before timing Criterion rows?",
    },
    SourceDoc {
        topic: Topic::Code,
        path: ".github/scripts/check-benchmarks-doc.sh",
        target_key: "workspace-code-bench-doc-guard",
        needles: &["Registry", "registered bench(es) undocumented"],
        query: "Which repository guard checks that registered benchmark targets are documented in BENCHMARKS.md?",
    },
];
