//! Tiny local corpus for oMLX embedding benchmark rows.

use selene_core::IStr;

use super::super::support::istr;

#[derive(Clone, Copy)]
pub(super) enum CorpusProfile {
    Tiny,
    AgentMemory,
}

impl CorpusProfile {
    pub(super) fn from_env(env_name: &str) -> Self {
        match std::env::var(env_name).ok().as_deref() {
            None | Some("") | Some("tiny") => Self::Tiny,
            Some("agent_memory") | Some("memory") => Self::AgentMemory,
            Some(other) => panic!("unsupported SELENE_OMLX_CORPUS value: {other}"),
        }
    }

    pub(super) fn inputs(self) -> Vec<CorpusInput> {
        match self {
            Self::Tiny => tiny_inputs(),
            Self::AgentMemory => agent_memory_inputs(),
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
