//! Tiny local corpus for oMLX embedding benchmark rows.

use selene_core::IStr;

use super::super::support::istr;

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

pub(super) fn corpus_inputs() -> Vec<CorpusInput> {
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
