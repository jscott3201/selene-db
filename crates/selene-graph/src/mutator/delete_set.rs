use std::collections::BTreeSet;

use selene_core::{EdgeId, NodeId};

use super::Mutator;
use crate::{GraphError, GraphResult};

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Delete a set of node and edge references.
    ///
    /// Nodes are deleted first through [`Self::delete_node`], so incident edges
    /// keep the same cascade behavior and change ordering as a direct node
    /// delete. Explicit edges already removed by a node cascade are skipped
    /// during the edge pass. IDs that are mapped but already invalidated are
    /// clean no-ops, matching set-oriented query deletion where duplicate rows
    /// may reference the same graph element more than once. IDs that were never
    /// allocated still return the same not-found errors as single-element
    /// delete calls. The full set is validated before the first row is removed,
    /// so a mixed set cannot partially mutate the transaction before surfacing a
    /// missing-id error.
    pub fn delete_elements(
        &mut self,
        nodes: BTreeSet<NodeId>,
        edges: BTreeSet<EdgeId>,
    ) -> GraphResult<()> {
        self.validate_delete_set(&nodes, &edges)?;
        for node in nodes {
            if self.node_can_be_deleted(node)? {
                self.delete_node(node)?;
            }
        }
        for edge in edges {
            if self.edge_can_be_deleted(edge)? {
                self.delete_edge(edge)?;
            }
        }
        Ok(())
    }

    fn validate_delete_set(
        &self,
        nodes: &BTreeSet<NodeId>,
        edges: &BTreeSet<EdgeId>,
    ) -> GraphResult<()> {
        for node in nodes {
            self.node_can_be_deleted(*node)?;
        }
        for edge in edges {
            self.edge_can_be_deleted(*edge)?;
        }
        Ok(())
    }

    fn node_can_be_deleted(&self, id: NodeId) -> GraphResult<bool> {
        let graph = self.txn.read();
        let row = graph
            .node_row_for_id(id)
            .ok_or(GraphError::NodeNotFound { id })?;
        if row.index() >= graph.node_store.len() {
            return Err(GraphError::NodeNotFound { id });
        }
        Ok(graph.node_store.is_row_alive(row))
    }

    fn edge_can_be_deleted(&self, id: EdgeId) -> GraphResult<bool> {
        let graph = self.txn.read();
        let row = graph
            .edge_row_for_id(id)
            .ok_or(GraphError::EdgeNotFound { id })?;
        if row.index() >= graph.edge_store.len() {
            return Err(GraphError::EdgeNotFound { id });
        }
        Ok(graph.edge_store.is_row_alive(row))
    }
}
