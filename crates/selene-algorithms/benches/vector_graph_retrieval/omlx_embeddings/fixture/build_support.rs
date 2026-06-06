//! Private build helpers for the local oMLX vector fixture.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use selene_core::{GraphId, IStr, NodeId, VectorValue};
use selene_graph::{SeleneGraph, VectorCandidateSet, VectorNeighborDirection};
use selene_testing::local_omlx::Topic;

pub(super) struct DocumentMeta {
    pub(super) node: NodeId,
    pub(super) topic: Topic,
    pub(super) graph_hint: bool,
}

pub(super) struct QueryAnchor {
    pub(super) node: NodeId,
    pub(super) topic: Topic,
}

pub(super) struct QueryVector {
    pub(super) anchor: NodeId,
    pub(super) topic: Topic,
    pub(super) text: String,
    pub(super) vector: VectorValue,
}

pub(super) fn topic_hint_expansion_set_for(
    graph: &SeleneGraph,
    dependency_edge: &IStr,
    support_edge: &IStr,
    anchor: NodeId,
) -> VectorCandidateSet {
    let roots = graph.vector_neighbor_candidates(
        anchor,
        dependency_edge,
        VectorNeighborDirection::Outgoing,
    );
    graph.expand_vector_candidate_set(&roots, support_edge, VectorNeighborDirection::Outgoing)
}

pub(super) fn graph_id_for_model(model: &str) -> GraphId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    GraphId::new(97_000 + hasher.finish() % 1_000)
}

pub(super) fn admits_graph_hint(
    graph_hint_counts: &mut HashMap<Topic, usize>,
    topic: Topic,
    graph_hint_docs_per_topic: Option<usize>,
) -> bool {
    let Some(limit) = graph_hint_docs_per_topic else {
        return true;
    };
    let count = graph_hint_counts.entry(topic).or_insert(0);
    if *count >= limit {
        return false;
    }
    *count += 1;
    true
}
