//! Target-aware local oMLX corpus for code and symbol alias retrieval rows.

use super::{CorpusInput, Topic};

pub(super) fn inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, docs) in [
        (
            Topic::Gql,
            &[
                (
                    "Graph query anchors collect roots before procedure calls.",
                    None,
                ),
                (
                    "The native procedure catalog registers GQL CALL surfaces.",
                    None,
                ),
                (
                    "Source-string plan cache reuses analyzed GQL statements.",
                    Some("gql-plan-cache"),
                ),
                (
                    "ProcedureRegistry lookup returns metadata for native built-ins.",
                    Some("gql-procedure-registry"),
                ),
                ("Pattern matching binds node rows before aggregation.", None),
                (
                    "Graph type checks validate label and property shapes.",
                    None,
                ),
                (
                    "Planner pipelines lower RETURN items into projections.",
                    None,
                ),
                (
                    "A stale query-plan note is superseded by cache generation checks.",
                    None,
                ),
                (
                    "The parser accepts strict ISO GQL and rejects grammar aliases.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Vector,
            &[
                (
                    "Embedding roots seed graph expansion before exact rerank.",
                    None,
                ),
                ("Vector procedures expose cosine candidate scoring.", None),
                (
                    "score_vector_nodes_batch_checked reranks explicit NodeId lists.",
                    Some("vector-batch-score"),
                ),
                (
                    "approximate_vector_search_expanded_candidates_checked expands ANN roots.",
                    Some("vector-ann-expanded"),
                ),
                (
                    "HNSW search returns approximate nearest-neighbor roots.",
                    None,
                ),
                (
                    "IVF partitions dense vectors by coarse centroid assignment.",
                    None,
                ),
                (
                    "VectorCandidateSet algebra intersects graph-authored candidates.",
                    None,
                ),
                (
                    "A stale vector fact is contradicted by negative evidence.",
                    None,
                ),
                (
                    "Exact cosine top-k scores every component of each candidate vector.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                ("Session roots connect active tasks to memory facts.", None),
                (
                    "Support edges link recalled facts to provenance nodes.",
                    None,
                ),
                (
                    "omlx_current_support_facts excludes outgoing negative evidence.",
                    Some("memory-current-state"),
                ),
                (
                    "Provenance-required state keeps facts with grounded evidence links.",
                    Some("memory-provenance-state"),
                ),
                (
                    "Superseded memories stay linked but should not be retrieved.",
                    None,
                ),
                (
                    "Dependency hints keep task-local memories stable across calls.",
                    None,
                ),
                (
                    "A durable memory graph records why a preference was remembered.",
                    None,
                ),
                (
                    "Contradictory memory notes are unresolved until review.",
                    None,
                ),
                (
                    "Active facts should survive snapshot and WAL recovery.",
                    None,
                ),
            ][..],
        ),
        (
            Topic::Code,
            &[
                (
                    "Benchmark harnesses build graph fixtures before timing rows.",
                    None,
                ),
                (
                    "Rust tests assert deterministic GQL procedure outputs.",
                    None,
                ),
                (
                    "node_id_for_row converts storage row indexes back to stable NodeId values.",
                    Some("code-rowid-map"),
                ),
                (
                    "Criterion benchmark IDs include precision suffixes for comparisons.",
                    Some("code-bench-label"),
                ),
                (
                    "CallPlanCache stores reusable source-string execution plans.",
                    None,
                ),
                (
                    "The oMLX client posts embedding JSON with a blocking TcpStream.",
                    None,
                ),
                (
                    "File-size guards keep tracked Rust modules below the cap.",
                    None,
                ),
                (
                    "A stale helper that mutates callers should be replaced by a wrapper.",
                    None,
                ),
                (
                    "Cargo clippy lints benchmark targets as well as library code.",
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
            text: "Which reusable query compiler avoids reparsing a statement string?",
            target_key: Some("gql-plan-cache"),
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Where does built-in procedure metadata get resolved?",
            target_key: Some("gql-procedure-registry"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which vector routine reranks many caller-provided graph ids?",
            target_key: Some("vector-batch-score"),
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which ANN path grows nearest-neighbor roots through support topology?",
            target_key: Some("vector-ann-expanded"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which maintained set removes contradicted memories before recall?",
            target_key: Some("memory-current-state"),
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which state keeps only memories with grounding evidence?",
            target_key: Some("memory-provenance-state"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which helper turns internal storage slots into external node ids?",
            target_key: Some("code-rowid-map"),
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where do benchmark names carry quality measurements?",
            target_key: Some("code-bench-label"),
        },
    ]);
    inputs
}
