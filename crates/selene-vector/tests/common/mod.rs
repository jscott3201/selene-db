use selene_core::NodeId;
use selene_vector::{HnswGraph, hnsw::InternalIndex};

pub(crate) type NodeSummary = (NodeId, u8, Vec<f32>, Vec<Vec<InternalIndex>>);
#[allow(dead_code)]
pub(crate) type GraphSummary = (usize, Option<InternalIndex>, u8, Vec<NodeSummary>);

pub(crate) fn graph_summary(graph: &HnswGraph) -> Vec<NodeSummary> {
    graph
        .iter_nodes()
        .map(|node| {
            (
                node.node_id,
                node.max_layer,
                node.vector.to_vec(),
                node.neighbors.iter().cloned().collect(),
            )
        })
        .collect()
}

#[allow(dead_code)]
pub(crate) fn full_graph_summary(graph: &HnswGraph) -> GraphSummary {
    (
        graph.dimensions(),
        graph.entry_point(),
        graph.max_layer(),
        graph_summary(graph),
    )
}
