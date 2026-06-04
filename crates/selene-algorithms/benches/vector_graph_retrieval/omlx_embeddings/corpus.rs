//! Tiny local corpus for oMLX embedding benchmark rows.

use selene_core::IStr;

use super::super::support::istr;

#[derive(Clone, Copy)]
pub(super) enum CorpusProfile {
    Tiny,
    AgentMemory,
    AmbiguousMemory,
    ScaledAmbiguousMemory,
}

impl CorpusProfile {
    pub(super) fn from_env(env_name: &str) -> Self {
        match std::env::var(env_name).ok().as_deref() {
            None | Some("") | Some("tiny") => Self::Tiny,
            Some("agent_memory") | Some("memory") => Self::AgentMemory,
            Some("ambiguous_memory") | Some("ambiguous") => Self::AmbiguousMemory,
            Some("scaled_ambiguous_memory") | Some("scaled_ambiguous") => {
                Self::ScaledAmbiguousMemory
            }
            Some(other) => panic!("unsupported SELENE_OMLX_CORPUS value: {other}"),
        }
    }

    pub(super) fn inputs(self) -> Vec<CorpusInput> {
        match self {
            Self::Tiny => tiny_inputs(),
            Self::AgentMemory => agent_memory_inputs(),
            Self::AmbiguousMemory => ambiguous_memory_inputs(),
            Self::ScaledAmbiguousMemory => scaled_ambiguous_memory_inputs(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Topic {
    Gql,
    Vector,
    AgentMemory,
    Code,
}

#[derive(Clone, Copy)]
pub(super) struct CorpusInput {
    pub(super) topic: Topic,
    pub(super) is_document: bool,
    pub(super) text: &'static str,
}

pub(super) fn topic_label(topic: Topic) -> IStr {
    match topic {
        Topic::Gql => istr("OmlxTopicGql"),
        Topic::Vector => istr("OmlxTopicVector"),
        Topic::AgentMemory => istr("OmlxTopicAgentMemory"),
        Topic::Code => istr("OmlxTopicCode"),
    }
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
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How does GQL execute graph pattern matching and procedure calls?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which vector index should rerank embedding candidates in memory?",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Find current task memory while ignoring contradicted facts.",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where is the Rust batch vector candidate scoring API implemented?",
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
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How can GQL retrieve active agent memories through graph procedures?",
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which graph patterns connect a task to supporting memory evidence?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "How should vector candidates be reranked after graph filtering?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "When does ANN help compared with exact scoring over graph candidates?",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Find current preferences and ignore superseded or contradictory facts.",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Retrieve session-scoped agent memory with provenance and recency hints.",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Where does the Rust benchmark derive graph candidate sets?",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which code path converts row indexes back to stable node ids?",
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
        }));
    }
    inputs.extend([
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "Which graph query filters current candidate facts before scoring?",
        },
        CorpusInput {
            topic: Topic::Gql,
            is_document: false,
            text: "How does GQL traversal find supporting evidence for a recalled fact?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "Which embedding search returns semantic candidates before reranking?",
        },
        CorpusInput {
            topic: Topic::Vector,
            is_document: false,
            text: "How can vector ranking retrieve stale facts without graph filtering?",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "Which memory graph candidates are current for this session request?",
        },
        CorpusInput {
            topic: Topic::AgentMemory,
            is_document: false,
            text: "How do dependency hints keep recalled agent memories stable?",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "Which Rust fixture function scores candidate nodes in a batch?",
        },
        CorpusInput {
            topic: Topic::Code,
            is_document: false,
            text: "How do stable benchmark IDs keep candidate rows comparable?",
        },
    ]);
    inputs
}

fn scaled_ambiguous_memory_inputs() -> Vec<CorpusInput> {
    let mut inputs = ambiguous_memory_inputs();
    inputs.extend(agent_memory_inputs());
    inputs
}
