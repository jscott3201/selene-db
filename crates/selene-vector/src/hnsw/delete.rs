//! Tombstone-based HNSW deletion.

use selene_core::NodeId;

use crate::VectorError;

use super::{HnswGraph, InternalIndex};

/// Mark `node_id` dead without rewiring existing neighbor lists.
///
/// Missing nodes are treated as a successful no-op. Deletion clears the live
/// bit and reverse lookup entry so a future insert may reuse the same
/// [`NodeId`]. If the deleted node was the entry point or occupied the current
/// maximum layer, the live entry point is recomputed eagerly.
pub(crate) fn tombstone_node(graph: &mut HnswGraph, node_id: NodeId) -> Result<(), VectorError> {
    let Some(idx) = graph.node_id_to_idx.remove(&node_id) else {
        return Ok(());
    };
    if !graph.is_alive_idx(idx) {
        return Ok(());
    }

    let deleted_max_layer = graph.node_by_idx(idx).map_or(0, |node| node.max_layer);
    graph.clear_alive_idx(idx);

    if graph.entry_point == Some(idx) || deleted_max_layer == graph.max_layer {
        recompute_entry_point(graph);
    }
    Ok(())
}

fn recompute_entry_point(graph: &mut HnswGraph) {
    let mut best: Option<(InternalIndex, u8)> = None;
    for idx in graph.live_indices() {
        let Some(node) = graph.node_by_idx(idx) else {
            continue;
        };
        let layer = node.max_layer;
        let replace = best.is_none_or(|(best_idx, best_layer)| {
            layer > best_layer || (layer == best_layer && idx < best_idx)
        });
        if replace {
            best = Some((idx, layer));
        }
    }

    graph.entry_point = best.map(|(idx, _)| idx);
    graph.max_layer = best.map_or(0, |(_, layer)| layer);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::hnsw::build::insert_node;
    use crate::{DistanceMetric, HnswConfig, HnswParams};

    fn params() -> HnswParams {
        HnswParams::from_config(
            &HnswConfig::with_params(2, 4, 16, 8, DistanceMetric::L2)
                .expect("test config is valid"),
        )
    }

    #[test]
    fn delete_missing_node_is_noop() {
        let mut graph = HnswGraph::empty(2);

        tombstone_node(&mut graph, NodeId::new(9)).expect("missing delete succeeds");

        assert_eq!(graph.live_len(), 0);
        assert_eq!(graph.entry_point(), None);
    }

    #[test]
    fn delete_entry_promotes_highest_live_layer() {
        let params = params();
        let mut graph = HnswGraph::empty(2);
        insert_node(
            &mut graph,
            NodeId::new(1),
            Arc::from([1.0, 0.0]),
            2,
            &params,
        )
        .unwrap();
        insert_node(
            &mut graph,
            NodeId::new(2),
            Arc::from([2.0, 0.0]),
            1,
            &params,
        )
        .unwrap();

        tombstone_node(&mut graph, NodeId::new(1)).expect("delete succeeds");

        assert_eq!(graph.entry_point(), Some(1));
        assert_eq!(graph.max_layer(), 1);
        assert_eq!(graph.live_len(), 1);
        assert!(graph.idx_for(NodeId::new(1)).is_none());
    }

    #[test]
    fn delete_all_nodes_clears_entry_point() {
        let params = params();
        let mut graph = HnswGraph::empty(2);
        insert_node(
            &mut graph,
            NodeId::new(1),
            Arc::from([1.0, 0.0]),
            1,
            &params,
        )
        .unwrap();

        tombstone_node(&mut graph, NodeId::new(1)).expect("delete succeeds");

        assert_eq!(graph.entry_point(), None);
        assert_eq!(graph.max_layer(), 0);
        assert_eq!(graph.live_len(), 0);
    }
}
