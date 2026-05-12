use selene_core::NodeId;
use selene_vector::{HnswGraph, hnsw::InternalIndex};

pub(crate) type NodeSummary = (NodeId, u8, Vec<f32>, Vec<Vec<InternalIndex>>);

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
