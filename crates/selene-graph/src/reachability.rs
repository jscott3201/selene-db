//! Bounded graph reachability candidate production.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use selene_core::{CancellationCause, CancellationChecker, DbString, NodeId};

use crate::error::GraphError;
use crate::graph::SeleneGraph;

const REACHABILITY_CANCEL_STRIDE: usize = 1024;

/// Direction used by reachability traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReachabilityDirection {
    /// Follow outgoing edges from each frontier node.
    Outgoing,
    /// Follow incoming edges into each frontier node.
    Incoming,
    /// Follow both outgoing and incoming edges.
    Both,
}

/// One reachable node and its minimum hop depth from the supplied roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReachableNode {
    /// Reachable node id.
    pub node_id: NodeId,
    /// Minimum number of graph hops from any root.
    pub depth: usize,
}

struct ReachabilityVisit<'a> {
    edge_label: &'a DbString,
    direction: ReachabilityDirection,
    k: usize,
    depths: &'a mut BTreeMap<NodeId, usize>,
    frontier: &'a mut VecDeque<(NodeId, usize)>,
}

/// Error returned by checked reachability APIs.
#[derive(Debug, thiserror::Error)]
pub enum ReachabilityError {
    /// Graph storage or stable-ID consistency failure.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Caller requested cooperative cancellation.
    #[error("reachability traversal cancelled")]
    Cancelled,
    /// Statement deadline elapsed.
    #[error("reachability traversal timed out after {elapsed:?}")]
    Timeout {
        /// Wall-clock duration since the deadline elapsed.
        elapsed: Duration,
    },
    /// Deterministic node-scan budget was exceeded.
    #[error("reachability traversal node scan budget exceeded ({scanned} > {limit})")]
    NodeScanBudgetExceeded {
        /// Maximum allowed scanned nodes.
        limit: usize,
        /// Observed scanned nodes after the batch that crossed the limit.
        scanned: usize,
    },
}

impl From<CancellationCause> for ReachabilityError {
    fn from(cause: CancellationCause) -> Self {
        match cause {
            CancellationCause::Cancelled => Self::Cancelled,
            CancellationCause::Timeout { elapsed } => Self::Timeout { elapsed },
            CancellationCause::NodeScanBudgetExceeded { limit, scanned } => {
                Self::NodeScanBudgetExceeded { limit, scanned }
            }
        }
    }
}

impl SeleneGraph {
    /// Return nodes reachable from `roots` through edges carrying `edge_label`.
    ///
    /// Roots are preserved at depth `0`. `max_depth = Some(0)` therefore
    /// returns only roots, while `None` traverses until the reachable set is
    /// exhausted or `k` nodes have been admitted.
    ///
    /// # Errors
    ///
    /// Returns [`ReachabilityError`] when stable-ID binding fails, cooperative
    /// cancellation or timeout occurs, or the deterministic scan budget trips.
    pub fn reachable_nodes_checked(
        &self,
        roots: &[NodeId],
        edge_label: &DbString,
        direction: ReachabilityDirection,
        max_depth: Option<usize>,
        k: usize,
        checker: CancellationChecker<'_>,
    ) -> Result<Vec<ReachableNode>, ReachabilityError> {
        checker.check()?;
        if k == 0 || roots.is_empty() {
            return Ok(Vec::new());
        }

        let roots = self.bind_node_candidates(roots.iter().copied())?;
        let mut depths = BTreeMap::<NodeId, usize>::new();
        let mut frontier = VecDeque::<(NodeId, usize)>::new();
        for root in roots.iter() {
            if depths.len() >= k {
                break;
            }
            if depths.insert(root, 0).is_none() {
                frontier.push_back((root, 0));
            }
        }

        let mut scanned_since_check = 0usize;
        while let Some((node, depth)) = frontier.pop_front() {
            if max_depth.is_some_and(|max_depth| depth >= max_depth) {
                continue;
            }
            scanned_since_check += 1;
            if scanned_since_check >= REACHABILITY_CANCEL_STRIDE {
                checker.note_nodes_scanned(scanned_since_check)?;
                scanned_since_check = 0;
            }
            self.visit_reachable_neighbors(
                node,
                depth + 1,
                &mut ReachabilityVisit {
                    edge_label,
                    direction,
                    k,
                    depths: &mut depths,
                    frontier: &mut frontier,
                },
            );
            if depths.len() >= k {
                break;
            }
        }
        if scanned_since_check > 0 {
            checker.note_nodes_scanned(scanned_since_check)?;
        }

        let mut nodes = depths
            .into_iter()
            .map(|(node_id, depth)| ReachableNode { node_id, depth })
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| (node.depth, node.node_id));
        Ok(nodes)
    }

    fn visit_reachable_neighbors(
        &self,
        node: NodeId,
        next_depth: usize,
        visit: &mut ReachabilityVisit<'_>,
    ) {
        if matches!(
            visit.direction,
            ReachabilityDirection::Outgoing | ReachabilityDirection::Both
        ) && let Some(entry) = self.outgoing_edges(node)
        {
            for edge in entry.iter_label(visit.edge_label) {
                if visit.depths.len() >= visit.k {
                    return;
                }
                if visit.depths.insert(edge.neighbor, next_depth).is_none() {
                    visit.frontier.push_back((edge.neighbor, next_depth));
                }
            }
        }
        if matches!(
            visit.direction,
            ReachabilityDirection::Incoming | ReachabilityDirection::Both
        ) && let Some(entry) = self.incoming_edges(node)
        {
            for edge in entry.iter_label(visit.edge_label) {
                if visit.depths.len() >= visit.k {
                    return;
                }
                if visit.depths.insert(edge.neighbor, next_depth).is_none() {
                    visit.frontier.push_back((edge.neighbor, next_depth));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use selene_core::{GraphId, LabelSet, PropertyMap, db_string};

    use crate::{ReachabilityDirection, SharedGraph};

    #[test]
    fn reachable_nodes_walks_transitive_outgoing_edges() {
        let shared = SharedGraph::new(GraphId::new(41_001));
        let node_label = db_string("reach.node").unwrap();
        let edge_label = db_string("PARENT").unwrap();
        let other_label = db_string("OTHER").unwrap();
        let (root, child, grandchild, sibling, wrong_label) = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let root = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let child = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let grandchild = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let sibling = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let wrong_label = mutator
                .create_node(LabelSet::single(node_label), PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(edge_label.clone(), root, child, PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(edge_label.clone(), child, grandchild, PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(edge_label.clone(), root, sibling, PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(other_label, root, wrong_label, PropertyMap::new())
                .unwrap();
            txn.commit().unwrap();
            (root, child, grandchild, sibling, wrong_label)
        };

        let hits = shared
            .read()
            .reachable_nodes_checked(
                &[root],
                &edge_label,
                ReachabilityDirection::Outgoing,
                None,
                10,
                selene_core::CancellationChecker::disabled(),
            )
            .unwrap();
        let ids = hits.iter().map(|hit| hit.node_id).collect::<Vec<_>>();
        assert_eq!(ids, vec![root, child, sibling, grandchild]);
        assert_eq!(
            hits.iter().map(|hit| hit.depth).collect::<Vec<_>>(),
            vec![0, 1, 1, 2]
        );
        assert!(!ids.contains(&wrong_label));
    }

    #[test]
    fn reachable_nodes_honors_direction_depth_and_k() {
        let shared = SharedGraph::new(GraphId::new(41_002));
        let node_label = db_string("reach.node").unwrap();
        let edge_label = db_string("LINK").unwrap();
        let (a, b, c) = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let b = mutator
                .create_node(LabelSet::single(node_label.clone()), PropertyMap::new())
                .unwrap();
            let c = mutator
                .create_node(LabelSet::single(node_label), PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(edge_label.clone(), a, b, PropertyMap::new())
                .unwrap();
            mutator
                .create_edge(edge_label.clone(), b, c, PropertyMap::new())
                .unwrap();
            txn.commit().unwrap();
            (a, b, c)
        };

        let snapshot = shared.read();
        let incoming = snapshot
            .reachable_nodes_checked(
                &[c],
                &edge_label,
                ReachabilityDirection::Incoming,
                Some(1),
                10,
                selene_core::CancellationChecker::disabled(),
            )
            .unwrap();
        assert_eq!(
            incoming
                .iter()
                .map(|hit| (hit.node_id, hit.depth))
                .collect::<Vec<_>>(),
            vec![(c, 0), (b, 1)]
        );

        let capped = snapshot
            .reachable_nodes_checked(
                &[a],
                &edge_label,
                ReachabilityDirection::Outgoing,
                None,
                2,
                selene_core::CancellationChecker::disabled(),
            )
            .unwrap();
        assert_eq!(capped.len(), 2);
    }

    #[test]
    fn reachable_nodes_ignores_missing_roots() {
        let shared = SharedGraph::new(GraphId::new(41_003));
        let node_label = db_string("reach.node").unwrap();
        let edge_label = db_string("LINK").unwrap();
        let root = {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            let root = mutator
                .create_node(LabelSet::single(node_label), PropertyMap::new())
                .unwrap();
            txn.commit().unwrap();
            root
        };
        let missing = selene_core::NodeId::new(root.get() + 10_000);

        let hits = shared
            .read()
            .reachable_nodes_checked(
                &[missing, root],
                &edge_label,
                ReachabilityDirection::Outgoing,
                Some(0),
                10,
                selene_core::CancellationChecker::disabled(),
            )
            .unwrap();

        assert_eq!(
            hits.iter()
                .map(|hit| (hit.node_id, hit.depth))
                .collect::<Vec<_>>(),
            vec![(root, 0)]
        );
    }

    #[test]
    fn reachable_nodes_propagates_live_root_mapping_inconsistency() {
        let shared = SharedGraph::new(GraphId::new(41_004));
        let edge_label = db_string("LINK").unwrap();
        let root = {
            let mut txn = shared.begin_write();
            let root = txn
                .mutator()
                .create_node(LabelSet::new(), PropertyMap::new())
                .unwrap();
            txn.commit().unwrap();
            root
        };
        let mut graph = shared.read().as_ref().clone();
        let row = graph.node_row_for_id(root).unwrap();
        graph
            .node_store
            .row_to_id
            .set(row.index(), selene_core::NodeId::new(9_999));

        let error = graph
            .reachable_nodes_checked(
                &[root],
                &edge_label,
                ReachabilityDirection::Outgoing,
                Some(0),
                1,
                selene_core::CancellationChecker::disabled(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            super::ReachabilityError::Graph(crate::GraphError::Inconsistent { .. })
        ));
    }
}
