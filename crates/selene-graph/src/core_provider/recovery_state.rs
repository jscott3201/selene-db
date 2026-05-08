//! Recovery-mode state for the core graph provider.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use selene_core::{Change, EdgeId, GraphId, IStr, LabelSet, NodeId, PropertyDiff, PropertyMap};

use crate::core_provider::sections::{
    EdgeRow, MetaPayload, NodeRow, SchemaEntry, SchemaKey, decode_edges, decode_graph_types,
    decode_meta, decode_nodes, decode_schemas,
};
use crate::core_provider::{
    CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_SCMA_SUB, inconsistent,
    invalid_payload,
};
use crate::graph::{GraphMeta, SeleneGraph};
use crate::graph_types::GraphTypeDef;
use crate::store::{edge_row_index, node_row_index};
use crate::typed_index::TypedIndex;

/// Accumulator populated by snapshot sections and WAL replay.
#[derive(Default)]
pub(crate) struct RecoveryState {
    meta: Option<MetaPayload>,
    graph_types: BTreeMap<u32, Arc<GraphTypeDef>>,
    nodes: BTreeMap<NodeId, NodeRow>,
    edges: BTreeMap<EdgeId, EdgeRow>,
    schemas: BTreeMap<SchemaKey, SchemaEntry>,
    sequence: u64,
}

impl RecoveryState {
    /// Construct an empty recovery accumulator.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn read_section(
        &mut self,
        sub_tag: crate::SubTag,
        bytes: &[u8],
    ) -> Result<(), crate::ProviderError> {
        match sub_tag.0 {
            CORE_GTYP_SUB => {
                let mut graph_types = BTreeMap::new();
                for (index, graph_type) in decode_graph_types(bytes)? {
                    let graph_type = graph_type
                        .validate()
                        .map_err(|error| inconsistent(format!("CORE/GTYP is invalid: {error}")))?;
                    graph_types.insert(index, Arc::new(graph_type));
                }
                self.graph_types = graph_types;
            }
            CORE_META_SUB => {
                let payload = decode_meta(bytes)?;
                self.sequence = payload.sequence;
                self.meta = Some(payload);
            }
            CORE_NODE_SUB => {
                self.nodes = decode_nodes(bytes)?.into_iter().collect();
            }
            CORE_EDGE_SUB => {
                self.edges = decode_edges(bytes)?.into_iter().collect();
            }
            CORE_SCMA_SUB => {
                self.schemas = decode_schemas(bytes)?.into_iter().collect();
            }
            _ => {
                return Err(invalid_payload(format!("unknown CORE sub-tag {sub_tag}")));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_change(&mut self, change: &Change) -> Result<(), crate::ProviderError> {
        match change {
            Change::NodeCreated {
                id,
                labels,
                properties,
            } => {
                if self.nodes.contains_key(id) {
                    return Err(inconsistent(format!(
                        "WAL replay attempted to recreate node {id}; node ids are never \
                         reused once allocated (D11)"
                    )));
                }
                self.nodes.insert(
                    *id,
                    NodeRow {
                        labels: labels.clone(),
                        properties: properties.clone(),
                        alive: true,
                    },
                );
            }
            Change::NodeUpdated {
                id,
                labels_diff,
                properties_diff,
            } => {
                let row = require_live_node(&mut self.nodes, *id)?;
                for label in labels_diff.added.iter().copied() {
                    row.labels.insert(label);
                }
                for label in labels_diff.removed.iter() {
                    row.labels.remove(label);
                }
                apply_property_diff(&mut row.properties, properties_diff)?;
            }
            Change::NodeDeleted { id } => {
                let row = require_live_node(&mut self.nodes, *id)?;
                row.alive = false;
            }
            Change::EdgeCreated {
                id,
                label,
                source,
                target,
                properties,
            } => {
                require_live_node_ref(&self.nodes, *source)?;
                require_live_node_ref(&self.nodes, *target)?;
                if self.edges.contains_key(id) {
                    return Err(inconsistent(format!(
                        "WAL replay attempted to recreate edge {id}; edge ids are never \
                         reused once allocated (D11)"
                    )));
                }
                self.edges.insert(
                    *id,
                    EdgeRow {
                        label: *label,
                        source: *source,
                        target: *target,
                        properties: properties.clone(),
                        alive: true,
                    },
                );
            }
            Change::EdgeUpdated {
                id,
                properties_diff,
            } => {
                let row = require_live_edge(&mut self.edges, *id)?;
                apply_property_diff(&mut row.properties, properties_diff)?;
            }
            Change::EdgeDeleted { id } => {
                let row = require_live_edge(&mut self.edges, *id)?;
                row.alive = false;
            }
            Change::SchemaChanged { .. } | Change::IndexExtensionEvent { .. } => {}
        }
        Ok(())
    }

    pub(crate) fn into_graph(self, expected_graph_id: GraphId) -> crate::GraphResult<SeleneGraph> {
        let meta = match self.meta {
            Some(meta) => {
                if meta.graph_id != expected_graph_id {
                    return Err(crate::GraphError::Provider(inconsistent(format!(
                        "CORE/META declares {} but caller asserted {} during recovery; \
                         refusing to silently reconstruct under the wrong identity",
                        meta.graph_id, expected_graph_id,
                    ))));
                }
                let bound_type = match meta.bound_type_index {
                    Some(index) => {
                        Some(self.graph_types.get(&index).cloned().ok_or_else(|| {
                            crate::GraphError::Provider(inconsistent(format!(
                                "CORE/META references missing CORE/GTYP index {index}"
                            )))
                        })?)
                    }
                    None => None,
                };
                GraphMeta {
                    graph_id: meta.graph_id,
                    generation: meta.generation,
                    next_node_id: meta.next_node_id,
                    next_edge_id: meta.next_edge_id,
                    bound_type,
                }
            }
            None => GraphMeta {
                graph_id: expected_graph_id,
                generation: 0,
                next_node_id: 1,
                next_edge_id: 1,
                bound_type: None,
            },
        };
        let mut graph = SeleneGraph::new(meta.graph_id);
        graph.meta = meta;

        let mut next_node_id = graph.meta.next_node_id.max(1);
        for (id, row) in self.nodes {
            next_node_id = next_node_id.max(id.get().saturating_add(1));
            insert_node_row(&mut graph, id, row)?;
        }
        graph.meta.next_node_id = next_node_id;

        let mut next_edge_id = graph.meta.next_edge_id.max(1);
        for (id, row) in self.edges {
            next_edge_id = next_edge_id.max(id.get().saturating_add(1));
            insert_edge_row(&mut graph, id, row)?;
        }
        graph.meta.next_edge_id = next_edge_id;

        // Re-register property indexes from SCMA. The empty TypedIndex placeholders
        // are filled by `rebuild_property_indexes` (called downstream via
        // `try_from_graph`) so the registration set survives restart even though
        // the entry contents are derived from primary state.
        for (key, entry) in self.schemas {
            graph.property_index.insert(
                (key.label, key.property),
                Arc::new(TypedIndex::new(entry.kind)),
            );
        }
        if let Some(type_def) = graph.meta.bound_type.as_deref() {
            crate::type_validator::validate_entity_state(&graph, type_def).map_err(|error| {
                crate::GraphError::Provider(inconsistent(format!(
                    "recovered closed graph violates bound type: {error}"
                )))
            })?;
        }
        Ok(graph)
    }
}

fn require_live_node(
    nodes: &mut BTreeMap<NodeId, NodeRow>,
    id: NodeId,
) -> Result<&mut NodeRow, crate::ProviderError> {
    let row = nodes
        .get_mut(&id)
        .ok_or_else(|| inconsistent(format!("WAL replay referenced missing node {id}")))?;
    if !row.alive {
        return Err(inconsistent(format!(
            "WAL replay referenced deleted node {id}"
        )));
    }
    Ok(row)
}

fn require_live_node_ref(
    nodes: &BTreeMap<NodeId, NodeRow>,
    id: NodeId,
) -> Result<(), crate::ProviderError> {
    let row = nodes
        .get(&id)
        .ok_or_else(|| inconsistent(format!("WAL replay referenced missing node {id}")))?;
    if !row.alive {
        return Err(inconsistent(format!(
            "WAL replay referenced deleted node {id}"
        )));
    }
    Ok(())
}

fn require_live_edge(
    edges: &mut BTreeMap<EdgeId, EdgeRow>,
    id: EdgeId,
) -> Result<&mut EdgeRow, crate::ProviderError> {
    let row = edges
        .get_mut(&id)
        .ok_or_else(|| inconsistent(format!("WAL replay referenced missing edge {id}")))?;
    if !row.alive {
        return Err(inconsistent(format!(
            "WAL replay referenced deleted edge {id}"
        )));
    }
    Ok(row)
}

fn apply_property_diff(
    map: &mut PropertyMap,
    diff: &PropertyDiff,
) -> Result<(), crate::ProviderError> {
    for (key, value) in diff.set.iter() {
        map.set(*key, value.clone())
            .map_err(|error| inconsistent(format!("WAL replay property set failed: {error}")))?;
    }
    for key in diff.removed.iter() {
        map.remove(key);
    }
    Ok(())
}

fn insert_node_row(graph: &mut SeleneGraph, id: NodeId, row: NodeRow) -> crate::GraphResult<()> {
    let row_index = node_row_index(id).ok_or_else(|| {
        crate::GraphError::Provider(invalid_payload(format!(
            "CORE/NODE payload used invalid node id {id}"
        )))
    })? as usize;
    while graph.node_store.len() < row_index {
        graph.node_store.labels.push(LabelSet::new());
        graph.node_store.properties.push(PropertyMap::new());
    }
    if graph.node_store.len() == row_index {
        graph.node_store.labels.push(row.labels);
        graph.node_store.properties.push(row.properties);
    } else {
        graph.node_store.labels.set(row_index, row.labels);
        graph.node_store.properties.set(row_index, row.properties);
    }
    set_alive(&mut graph.node_store.alive, row_index, row.alive);
    Ok(())
}

fn insert_edge_row(graph: &mut SeleneGraph, id: EdgeId, row: EdgeRow) -> crate::GraphResult<()> {
    let row_index = edge_row_index(id).ok_or_else(|| {
        crate::GraphError::Provider(invalid_payload(format!(
            "CORE/EDGE payload used invalid edge id {id}"
        )))
    })? as usize;
    while graph.edge_store.len() < row_index {
        graph.edge_store.label.push(edge_hole_label()?);
        graph.edge_store.source.push(NodeId::TOMBSTONE);
        graph.edge_store.target.push(NodeId::TOMBSTONE);
        graph.edge_store.properties.push(PropertyMap::new());
    }
    if graph.edge_store.len() == row_index {
        graph.edge_store.label.push(row.label);
        graph.edge_store.source.push(row.source);
        graph.edge_store.target.push(row.target);
        graph.edge_store.properties.push(row.properties);
    } else {
        graph.edge_store.label.set(row_index, row.label);
        graph.edge_store.source.set(row_index, row.source);
        graph.edge_store.target.set(row_index, row.target);
        graph.edge_store.properties.set(row_index, row.properties);
    }
    set_alive(&mut graph.edge_store.alive, row_index, row.alive);
    Ok(())
}

fn set_alive(bitmap: &mut roaring::RoaringBitmap, row_index: usize, alive: bool) {
    let row = u32::try_from(row_index).expect("row index was validated before liveness update");
    if alive {
        bitmap.insert(row);
    } else {
        bitmap.remove(row);
    }
}

fn edge_hole_label() -> Result<IStr, crate::GraphError> {
    static CELL: OnceLock<IStr> = OnceLock::new();
    if let Some(label) = CELL.get() {
        return Ok(*label);
    }
    let label = selene_core::intern("__selene_hole").map_err(crate::GraphError::Core)?;
    let _ = CELL.set(label);
    Ok(label)
}
