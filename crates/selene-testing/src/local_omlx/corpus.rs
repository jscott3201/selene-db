//! Reusable corpora for opt-in embedding benchmark rows.

use selene_core::{IStr, intern};

mod code_alias;
mod project_code;
mod source_code;

/// Corpus size and ambiguity profile for local embedding benchmarks.
#[derive(Clone, Copy)]
pub enum CorpusProfile {
    /// Four documents per topic plus one query per topic.
    Tiny,
    /// Agent-memory-oriented corpus with two queries per topic.
    AgentMemory,
    /// Corpus with overlapping terminology across topics to stress vector-only retrieval.
    AmbiguousMemory,
    /// Combined ambiguous + agent-memory corpus for larger local rows.
    ScaledAmbiguousMemory,
    /// Code and symbol alias corpus with per-query target facts.
    CodeAliasMemory,
    /// Wider code and symbol alias corpus with more target-hit queries.
    CodeAliasWideMemory,
    /// Target-aware corpus shaped like current selene-db source files.
    ProjectCodeMemory,
    /// Harder source-shaped alias corpus with lexical decoys.
    ProjectCodeAliasMemory,
    /// Source excerpt corpus with target-aware real code snippets.
    ProjectSourceCodeMemory,
}

impl CorpusProfile {
    /// Resolve a profile from `env_name`, defaulting to [`Self::Tiny`].
    pub fn from_env(env_name: &str) -> Self {
        std::env::var(env_name)
            .ok()
            .as_deref()
            .map_or(Self::Tiny, Self::from_value)
    }

    /// Resolve a profile from an environment value.
    pub fn from_value(value: &str) -> Self {
        match value {
            "" | "tiny" => Self::Tiny,
            "agent_memory" | "memory" => Self::AgentMemory,
            "ambiguous_memory" | "ambiguous" => Self::AmbiguousMemory,
            "scaled_ambiguous_memory" | "scaled_ambiguous" => Self::ScaledAmbiguousMemory,
            "code_alias_memory" | "code_alias" => Self::CodeAliasMemory,
            "code_alias_wide_memory" | "code_alias_wide" => Self::CodeAliasWideMemory,
            "project_code_memory" | "project_code" | "selene_project_code" => {
                Self::ProjectCodeMemory
            }
            "project_code_alias_memory" | "project_code_alias" | "selene_project_code_alias" => {
                Self::ProjectCodeAliasMemory
            }
            "project_source_code_memory" | "project_source_code" | "selene_source_code" => {
                Self::ProjectSourceCodeMemory
            }
            other => panic!("unsupported embedding corpus value: {other}"),
        }
    }

    /// Materialize the profile as ordered document and query inputs.
    pub fn inputs(self) -> Vec<CorpusInput> {
        match self {
            Self::Tiny => tiny_inputs(),
            Self::AgentMemory => agent_memory_inputs(),
            Self::AmbiguousMemory => ambiguous_memory_inputs(),
            Self::ScaledAmbiguousMemory => scaled_ambiguous_memory_inputs(),
            Self::CodeAliasMemory => code_alias::inputs(),
            Self::CodeAliasWideMemory => code_alias::wide_inputs(),
            Self::ProjectCodeMemory => project_code::inputs(),
            Self::ProjectCodeAliasMemory => project_code::alias_inputs(),
            Self::ProjectSourceCodeMemory => source_code::inputs(),
        }
    }
}

/// Topic assigned to a corpus document or query.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Topic {
    /// ISO GQL and graph-query language behavior.
    Gql,
    /// Vector storage, search, indexing, and rerank behavior.
    Vector,
    /// Agentic memory retrieval, provenance, and currentness.
    AgentMemory,
    /// Rust implementation and benchmark-code behavior.
    Code,
}

/// One text item sent to the local embedding endpoint.
#[derive(Clone, Copy)]
pub struct CorpusInput {
    /// Semantic topic used by graph labels and precision checks.
    pub topic: Topic,
    /// Whether this item is a searchable document (`true`) or query (`false`).
    pub is_document: bool,
    /// Text submitted to the embedding endpoint.
    pub text: &'static str,
    /// Optional document key or query target key for target-hit benchmark rows.
    pub target_key: Option<&'static str>,
}

/// Return the graph label used for `topic` in benchmark fixtures.
pub fn topic_label(topic: Topic) -> IStr {
    match topic {
        Topic::Gql => istr("OmlxTopicGql"),
        Topic::Vector => istr("OmlxTopicVector"),
        Topic::AgentMemory => istr("OmlxTopicAgentMemory"),
        Topic::Code => istr("OmlxTopicCode"),
    }
}

fn istr(value: &str) -> IStr {
    intern(value).expect("local embedding fixture strings fit the interner")
}

fn tiny_inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, texts) in [
        (
            Topic::Gql,
            &[
                "ISO GQL MATCH over graph patterns returns rows from a property graph.",
                "A GQL CALL procedure can expose implementation-defined graph algorithms.",
                "Graph type constraints validate labels and property value types.",
                "Serializable graph transactions preserve committed mutation ordering.",
            ][..],
        ),
        (
            Topic::Vector,
            &[
                "A dense embedding vector is stored as a first-class graph property value.",
                "HNSW approximate nearest-neighbor search trades recall for latency.",
                "IVF partitions vectors through coarse centroids before reranking candidates.",
                "Candidate-set scoring exact-reranks graph-derived vector candidates.",
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                "Agent memory retrieval should prefer current facts over superseded evidence.",
                "Contradiction edges mark unresolved memory facts that should be filtered.",
                "Session and scope nodes narrow memory retrieval to a task-local subgraph.",
                "Recency windows and dependency edges produce active memory candidate sets.",
            ][..],
        ),
        (
            Topic::Code,
            &[
                "Rust benchmarks should use Criterion groups with stable benchmark IDs.",
                "A VectorCandidateSet stores sorted and deduplicated NodeId values.",
                "The score_vector_candidate_sets_batch_checked API scores many queries.",
                "A safe local HTTP client can post JSON without adding a runtime dependency.",
            ][..],
        ),
    ] {
        inputs.extend(texts.iter().map(|text| CorpusInput {
            topic,
            is_document: true,
            text,
            target_key: None,
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How does GQL execute graph pattern matching and procedure calls?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which vector index should rerank embedding candidates in memory?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Find current task memory while ignoring contradicted facts.",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where is the Rust batch vector candidate scoring API implemented?",
            target_key: None,
        },
    ]);
    inputs
}

fn agent_memory_inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, texts) in [
        (
            Topic::Gql,
            &[
                "MATCH finds active memory nodes linked to the current agent session.",
                "CALL selene.vector_score_nodes reranks graph-derived memory candidates.",
                "A graph type can require memory facts to carry confidence and scope.",
                "Serializable GQL writes keep memory replacement edges ordered.",
                "Path patterns can traverse from a task node to supporting evidence.",
                "Closed graph constraints reject malformed agent memory records.",
                "Projection catalogs let algorithms run over scoped memory subgraphs.",
                "Procedure calls expose graph algorithms without extending GQL grammar.",
            ][..],
        ),
        (
            Topic::Vector,
            &[
                "A first-class embedding vector stores semantic memory content.",
                "HNSW returns approximate candidates before exact vector reranking.",
                "IVF partitions memory vectors into coarse semantic regions.",
                "Candidate-set scoring compares query embeddings to selected nodes.",
                "Vector dimensions differ across local embedding models.",
                "Cosine distance is the default semantic retrieval metric here.",
                "ANN recall can fail when graph hints already identify precise facts.",
                "Batch vector scoring amortizes query setup over multiple memories.",
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                "The current preference says use rustls and avoid native TLS.",
                "A superseded preference should remain linked but not retrieved as current.",
                "Contradictory evidence marks a memory fact unresolved until reviewed.",
                "A session scope narrows retrieval to facts relevant to the task.",
                "Dependency edges identify facts required by the active planning step.",
                "Recent observations can be maintained as graph-authored active hints.",
                "A durable memory graph should preserve why a fact was remembered.",
                "Agent memory retrieval balances semantic similarity with graph currentness.",
            ][..],
        ),
        (
            Topic::Code,
            &[
                "The benchmark runner serializes Criterion invocations by bench name.",
                "VectorCandidateSet intersection uses sorted NodeId arrays.",
                "RowIndex values must be mapped back through node_id_for_row.",
                "The local oMLX HTTP client posts JSON without adding async dependencies.",
                "Criterion benchmark IDs include quality suffixes for comparison.",
                "Graph fixtures build HNSW indexes before committing the write transaction.",
                "Rust clippy runs all targets so benchmark code must lint cleanly.",
                "The file-size script enforces a 700 line cap for tracked Rust files.",
            ][..],
        ),
    ] {
        inputs.extend(texts.iter().map(|text| CorpusInput {
            topic,
            is_document: true,
            text,
            target_key: None,
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How can GQL retrieve active agent memories through graph procedures?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which graph patterns connect a task to supporting memory evidence?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "How should vector candidates be reranked after graph filtering?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "When does ANN help compared with exact scoring over graph candidates?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Find current preferences and ignore superseded or contradictory facts.",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Retrieve session-scoped agent memory with provenance and recency hints.",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where does the Rust benchmark derive graph candidate sets?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which code path converts row indexes back to stable node ids?",
            target_key: None,
        },
    ]);
    inputs
}

fn ambiguous_memory_inputs() -> Vec<CorpusInput> {
    let mut inputs = Vec::new();
    for (topic, texts) in [
        (
            Topic::Gql,
            &[
                "A query anchor matches active facts before vector scoring begins.",
                "The graph pattern filters current memory candidates for a request.",
                "A GQL procedure scores candidate nodes after a MATCH-derived scope.",
                "Closed graph types validate the same memory record shape every commit.",
                "Path traversal finds the supporting evidence behind a recalled fact.",
                "A graph query can exclude stale facts without changing vector distance.",
                "Serializable mutation order decides which replacement edge is current.",
                "Projection names keep reusable candidate subgraphs stable across calls.",
            ][..],
        ),
        (
            Topic::Vector,
            &[
                "A query embedding scores active facts after graph filtering.",
                "The vector index returns semantic candidates for a request.",
                "An ANN procedure scores candidate nodes after an embedding-derived scope.",
                "Vector dimensions validate the same memory record shape every insert.",
                "Nearest-neighbor search finds the supporting evidence behind a recalled fact.",
                "A vector query can retrieve stale facts unless graph currentness filters them.",
                "Approximate ranking order decides which replacement fact appears nearest.",
                "Centroid partitions keep reusable candidate regions stable across searches.",
            ][..],
        ),
        (
            Topic::AgentMemory,
            &[
                "A session anchor scores active facts after graph and vector filtering.",
                "The memory graph returns current candidates for a request.",
                "An agent recall procedure scores candidate nodes after a session-derived scope.",
                "Memory schemas validate the same preference record shape every commit.",
                "Provenance traversal finds the supporting evidence behind a recalled fact.",
                "A memory query can exclude stale facts through graph currentness.",
                "Supersession edge order decides which replacement fact is current.",
                "Dependency hints keep reusable candidate memories stable across tasks.",
            ][..],
        ),
        (
            Topic::Code,
            &[
                "A benchmark anchor scores active fixtures after graph filtering.",
                "The Rust fixture returns current candidates for a request.",
                "A batch scoring function scores candidate nodes after a test-derived scope.",
                "Constructor assertions validate the same vector record shape every run.",
                "Fixture traversal finds the supporting edge behind a measured fact.",
                "A benchmark query can exclude stale fixtures without changing vector distance.",
                "Commit order decides which replacement edge appears in the snapshot.",
                "Stable benchmark IDs keep reusable candidate rows comparable across runs.",
            ][..],
        ),
    ] {
        inputs.extend(texts.iter().map(|text| CorpusInput {
            topic,
            is_document: true,
            text,
            target_key: None,
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which graph query filters current candidate facts before scoring?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How does GQL traversal find supporting evidence for a recalled fact?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which embedding search returns semantic candidates before reranking?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "How can vector ranking retrieve stale facts without graph filtering?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which memory graph candidates are current for this session request?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "How do dependency hints keep recalled agent memories stable?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which Rust fixture function scores candidate nodes in a batch?",
            target_key: None,
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "How do stable benchmark IDs keep candidate rows comparable?",
            target_key: None,
        },
    ]);
    inputs
}

fn scaled_ambiguous_memory_inputs() -> Vec<CorpusInput> {
    let mut inputs = ambiguous_memory_inputs();
    inputs.extend(agent_memory_inputs());
    inputs
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{CorpusProfile, Topic, topic_label};

    #[test]
    fn tiny_profile_has_four_topics_with_documents_and_queries() {
        let inputs = CorpusProfile::Tiny.inputs();
        let document_count = inputs.iter().filter(|input| input.is_document).count();
        let query_count = inputs.len() - document_count;

        assert_eq!(document_count, 16);
        assert_eq!(query_count, 4);
    }

    #[test]
    fn scaled_ambiguous_profile_combines_ambiguous_and_agent_memory() {
        let scaled = CorpusProfile::ScaledAmbiguousMemory.inputs();
        let expected = CorpusProfile::AmbiguousMemory.inputs().len()
            + CorpusProfile::AgentMemory.inputs().len();

        assert_eq!(scaled.len(), expected);
    }

    #[test]
    fn code_alias_profile_targets_existing_documents() {
        let inputs = CorpusProfile::CodeAliasMemory.inputs();
        let document_keys = inputs
            .iter()
            .filter(|input| input.is_document)
            .filter_map(|input| input.target_key)
            .collect::<HashSet<_>>();
        let query_targets = inputs
            .iter()
            .filter(|input| !input.is_document)
            .map(|input| input.target_key.expect("code alias query has target"))
            .collect::<Vec<_>>();

        assert_eq!(query_targets.len(), 8);
        assert!(
            query_targets
                .iter()
                .all(|target| document_keys.contains(target))
        );
    }

    #[test]
    fn wide_code_alias_profile_extends_target_queries() {
        let inputs = CorpusProfile::CodeAliasWideMemory.inputs();
        let document_keys = inputs
            .iter()
            .filter(|input| input.is_document)
            .filter_map(|input| input.target_key)
            .collect::<HashSet<_>>();
        let query_targets = inputs
            .iter()
            .filter(|input| !input.is_document)
            .map(|input| input.target_key.expect("wide code alias query has target"))
            .collect::<Vec<_>>();

        assert_eq!(query_targets.len(), 16);
        assert!(
            query_targets
                .iter()
                .all(|target| document_keys.contains(target))
        );
    }

    #[test]
    fn project_code_profile_targets_existing_documents() {
        let inputs = CorpusProfile::ProjectCodeMemory.inputs();
        let document_keys = inputs
            .iter()
            .filter(|input| input.is_document)
            .filter_map(|input| input.target_key)
            .collect::<HashSet<_>>();
        let query_targets = inputs
            .iter()
            .filter(|input| !input.is_document)
            .map(|input| input.target_key.expect("project code query has target"))
            .collect::<Vec<_>>();

        assert_eq!(query_targets.len(), 16);
        assert!(
            query_targets
                .iter()
                .all(|target| document_keys.contains(target))
        );
    }

    #[test]
    fn project_code_alias_profile_targets_existing_documents() {
        let inputs = CorpusProfile::ProjectCodeAliasMemory.inputs();
        let document_keys = inputs
            .iter()
            .filter(|input| input.is_document)
            .filter_map(|input| input.target_key)
            .collect::<HashSet<_>>();
        let query_targets = inputs
            .iter()
            .filter(|input| !input.is_document)
            .map(|input| {
                input
                    .target_key
                    .expect("project code alias query has target")
            })
            .collect::<Vec<_>>();

        assert_eq!(query_targets.len(), 16);
        assert!(
            query_targets
                .iter()
                .all(|target| document_keys.contains(target))
        );
    }

    #[test]
    fn project_source_code_profile_targets_existing_documents() {
        let inputs = CorpusProfile::ProjectSourceCodeMemory.inputs();
        let document_keys = inputs
            .iter()
            .filter(|input| input.is_document)
            .filter_map(|input| input.target_key)
            .collect::<HashSet<_>>();
        let query_targets = inputs
            .iter()
            .filter(|input| !input.is_document)
            .map(|input| {
                input
                    .target_key
                    .expect("project source code query has target")
            })
            .collect::<Vec<_>>();

        assert_eq!(query_targets.len(), 16);
        assert!(
            query_targets
                .iter()
                .all(|target| document_keys.contains(target))
        );
    }

    #[test]
    fn parses_corpus_profile_values() {
        assert!(matches!(
            CorpusProfile::from_value("tiny"),
            CorpusProfile::Tiny
        ));
        assert!(matches!(
            CorpusProfile::from_value("code_alias"),
            CorpusProfile::CodeAliasMemory
        ));
        assert!(matches!(
            CorpusProfile::from_value("code_alias_wide"),
            CorpusProfile::CodeAliasWideMemory
        ));
        assert!(matches!(
            CorpusProfile::from_value("selene_project_code"),
            CorpusProfile::ProjectCodeMemory
        ));
        assert!(matches!(
            CorpusProfile::from_value("selene_project_code_alias"),
            CorpusProfile::ProjectCodeAliasMemory
        ));
        assert!(matches!(
            CorpusProfile::from_value("selene_source_code"),
            CorpusProfile::ProjectSourceCodeMemory
        ));
        assert!(matches!(
            CorpusProfile::from_value("scaled_ambiguous_memory"),
            CorpusProfile::ScaledAmbiguousMemory
        ));
    }

    #[test]
    fn topic_labels_are_distinct() {
        let labels = [
            topic_label(Topic::Gql),
            topic_label(Topic::Vector),
            topic_label(Topic::AgentMemory),
            topic_label(Topic::Code),
        ];
        let unique = labels.iter().collect::<HashSet<_>>();

        assert_eq!(unique.len(), labels.len());
    }
}
