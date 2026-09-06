// Lower-engine example using observed CandidateSet APIs; not compile-tested here.
// A facade caller must validate database-scoped provenance before reaching this layer.
use selene_core::NodeId;
use selene_graph::SeleneGraph;
use std::error::Error;

pub fn intersect_live_ids(
    graph: &SeleneGraph,
    left_ids: impl IntoIterator<Item = NodeId>,
    right_ids: impl IntoIterator<Item = NodeId>,
) -> Result<Vec<NodeId>, Box<dyn Error>> {
    // Binding is liveness-only: no requirement for a vector/property/index membership.
    let left = graph.bind_node_candidates(left_ids)?;
    let right = graph.bind_node_candidates(right_ids)?;

    // Keep graph/generation/layout/workspace validation inside graph-owned algebra.
    let overlap = graph.intersect_candidates(&left, &right)?;
    Ok(overlap.iter().collect())
}
