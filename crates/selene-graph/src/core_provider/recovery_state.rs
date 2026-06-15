//! Recovery-mode state for the core graph provider.

use std::collections::BTreeMap;
use std::sync::Arc;

use selene_core::{EdgeId, GraphId, NodeId, SchemaChange};
use smallvec::SmallVec;

use crate::core_provider::sections::{
    CompositeSchemaEntry, CompositeSchemaKey, EdgeRow, MetaPayload, NodeRow, SchemaEntityKind,
    SchemaEntry, SchemaKey, TextSchemaEntry, TextSchemaKey, VectorSchemaEntry, VectorSchemaKey,
    decode_composite_schemas, decode_edges, decode_graph_types, decode_meta, decode_nodes,
    decode_schemas, decode_text_schemas, decode_vector_schemas,
};
use crate::core_provider::{
    CORE_CPIX_SUB, CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_SCMA_SUB,
    CORE_TIDX_SUB, CORE_VIDX_SUB, inconsistent, invalid_payload,
};
use crate::graph::{
    CompositePropertyIndexEntry, GraphMeta, PropertyIndexEntry, SeleneGraph, TextIndexEntry,
    VectorIndexEntry,
};
use crate::graph_types::GraphTypeDef;
use crate::typed_index::TypedIndex;

mod change_replay;
mod index_replay;
mod materialize;
mod schema_replay;

use index_replay::{
    PendingCompositeIndex, PendingIndex, PendingTextIndex, PendingVectorIndex,
    replay_composite_property_index_changes, replay_property_index_changes,
    replay_text_index_changes, replay_vector_index_changes,
};
use materialize::{insert_edge_row, insert_node_row};

/// Accumulator populated by snapshot sections and WAL replay.
#[derive(Default)]
pub(crate) struct RecoveryState {
    meta: Option<MetaPayload>,
    graph_types: BTreeMap<u32, Arc<GraphTypeDef>>,
    pending_schema_changes: Vec<SchemaChange>,
    pending_property_index_changes: Vec<PendingIndex>,
    pending_composite_property_index_changes: Vec<PendingCompositeIndex>,
    pending_vector_index_changes: Vec<PendingVectorIndex>,
    pending_text_index_changes: Vec<PendingTextIndex>,
    nodes: BTreeMap<NodeId, NodeRow>,
    edges: BTreeMap<EdgeId, EdgeRow>,
    /// BRIEF-Item-4a STEP 9: the snapshot row (= section position) each
    /// committed id was decoded at, so `into_graph` materializes snapshot rows
    /// **positionally** rather than by deriving the row from the id. Aborted-tx
    /// hole rows (`*Id::TOMBSTONE`) are not recorded — they are re-materialized as
    /// the pad slots between the real rows the column places. WAL-created ids
    /// absent here append at the dense end (BRIEF-Item-4c; 4e revisits this for
    /// WAL events that cross a 4b compaction epoch).
    node_snapshot_rows: BTreeMap<NodeId, u32>,
    edge_snapshot_rows: BTreeMap<EdgeId, u32>,
    schemas: BTreeMap<SchemaKey, SchemaEntry>,
    composite_schemas: Vec<(CompositeSchemaKey, CompositeSchemaEntry)>,
    vector_schemas: Vec<(VectorSchemaKey, VectorSchemaEntry)>,
    text_schemas: Vec<(TextSchemaKey, TextSchemaEntry)>,
    sequence: u64,
    /// Set once a [`Change::GraphReset`] (BRIEF-152, audit Item 10) is replayed.
    ///
    /// A factory-reset moots all schema/index intents seen so far in the WAL, so
    /// the reset arm clears the pending lists and sets this flag. `into_graph`
    /// then short-circuits the snapshot/caller bound-type reconciliation and
    /// forces `bound_type = None` (open), matching the runtime reset. Without
    /// this, a `recover_closed(bound_type)` after a reset would reject (snapshot
    /// declares no binding, caller asserts one) or silently restore the
    /// pre-reset type from the snapshot.
    schema_reset_to_open: bool,
}

const V1_BOUND_GRAPH_TYPE_INDEX: u32 = 0;

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
                // BRIEF-Item-4a STEP 9: the section is positional. Record each
                // committed id's row (= decode position) for positional
                // materialization; skip `NodeId::TOMBSTONE` hole rows — they are
                // re-padded between the real rows in `into_graph` and binding
                // their (absent) id would resurrect an aborted-tx id as NotAlive.
                for (position, (id, row)) in decode_nodes(bytes)?.into_iter().enumerate() {
                    if id == NodeId::TOMBSTONE {
                        continue;
                    }
                    let position = u32::try_from(position).map_err(|_| {
                        invalid_payload("CORE/NODE row position exceeds u32::MAX".to_string())
                    })?;
                    self.node_snapshot_rows.insert(id, position);
                    self.nodes.insert(id, row);
                }
            }
            CORE_EDGE_SUB => {
                for (position, (id, row)) in decode_edges(bytes)?.into_iter().enumerate() {
                    if id == EdgeId::TOMBSTONE {
                        continue;
                    }
                    let position = u32::try_from(position).map_err(|_| {
                        invalid_payload("CORE/EDGE row position exceeds u32::MAX".to_string())
                    })?;
                    self.edge_snapshot_rows.insert(id, position);
                    self.edges.insert(id, row);
                }
            }
            CORE_SCMA_SUB => {
                self.schemas = decode_schemas(bytes)?.into_iter().collect();
            }
            CORE_CPIX_SUB => {
                self.composite_schemas = decode_composite_schemas(bytes)?;
            }
            CORE_VIDX_SUB => {
                self.vector_schemas = decode_vector_schemas(bytes)?;
            }
            CORE_TIDX_SUB => {
                self.text_schemas = decode_text_schemas(bytes)?;
            }
            _ => {
                return Err(invalid_payload(format!("unknown CORE sub-tag {sub_tag}")));
            }
        }
        Ok(())
    }

    pub(crate) fn into_graph(
        self,
        expected_graph_id: GraphId,
        expected_bound_type: Option<Arc<GraphTypeDef>>,
    ) -> crate::GraphResult<SeleneGraph> {
        // F5: GTYP non-empty + META missing is a structurally inconsistent
        // snapshot. Type rows must have a META to bind to, otherwise recovery
        // would silently downgrade a closed graph to open.
        if self.meta.is_none() && !self.graph_types.is_empty() {
            return Err(crate::GraphError::Provider(inconsistent(
                "CORE/GTYP non-empty but CORE/META missing; snapshot is \
                 structurally inconsistent",
            )));
        }
        // BRIEF-152: a replayed GraphReset forces the recovered graph open and
        // moots every prior schema intent. Short-circuit the snapshot/caller
        // bound-type reconciliation entirely — bind to None regardless of what
        // the snapshot or caller asserted — so a `recover_closed(bound_type)`
        // after a reset reconstructs the identical empty+open post-state the
        // runtime produced instead of rejecting on the reconciliation conflict.
        let schema_reset_to_open = self.schema_reset_to_open;
        let meta = match self.meta {
            Some(meta) if schema_reset_to_open => {
                if meta.graph_id != expected_graph_id {
                    return Err(crate::GraphError::Provider(inconsistent(format!(
                        "CORE/META declares {} but caller asserted {} during recovery; \
                         refusing to silently reconstruct under the wrong identity",
                        meta.graph_id, expected_graph_id,
                    ))));
                }
                GraphMeta {
                    graph_id: meta.graph_id,
                    generation: meta.generation,
                    next_node_id: meta.next_node_id,
                    next_edge_id: meta.next_edge_id,
                    bound_type: None,
                }
            }
            None if schema_reset_to_open => GraphMeta {
                graph_id: expected_graph_id,
                generation: 0,
                next_node_id: 1,
                next_edge_id: 1,
                bound_type: None,
            },
            Some(meta) => {
                if meta.graph_id != expected_graph_id {
                    return Err(crate::GraphError::Provider(inconsistent(format!(
                        "CORE/META declares {} but caller asserted {} during recovery; \
                         refusing to silently reconstruct under the wrong identity",
                        meta.graph_id, expected_graph_id,
                    ))));
                }
                let snapshot_bound_type = match meta.bound_type_index {
                    Some(index) => {
                        Some(self.graph_types.get(&index).cloned().ok_or_else(|| {
                            crate::GraphError::Provider(inconsistent(format!(
                                "CORE/META references missing CORE/GTYP index {index}"
                            )))
                        })?)
                    }
                    None => None,
                };
                // Reconcile snapshot binding with caller's assertion. Either
                // side disagreeing is closed/open drift the user must surface.
                let mut bound_type = match (&snapshot_bound_type, &expected_bound_type) {
                    (Some(snap), Some(caller)) if snap.as_ref() != caller.as_ref() => {
                        return Err(crate::GraphError::Provider(inconsistent(
                            "CORE/META bound_type disagrees with caller-supplied \
                             bound_type during recovery; refusing to reconstruct \
                             under the wrong type",
                        )));
                    }
                    (Some(snap), _) => Some(snap.clone()),
                    (None, Some(_)) => {
                        return Err(crate::GraphError::Provider(inconsistent(
                            "caller supplied bound_type but CORE/META declares no \
                             binding; refusing to reconstruct under the wrong shape",
                        )));
                    }
                    (None, None) => None,
                };
                schema_replay::replay_schema_changes(
                    &mut bound_type,
                    &self.pending_schema_changes,
                )?;
                GraphMeta {
                    graph_id: meta.graph_id,
                    generation: meta.generation,
                    next_node_id: meta.next_node_id,
                    next_edge_id: meta.next_edge_id,
                    bound_type,
                }
            }
            // F2: WAL-only recovery preserves the caller's binding so a
            // closed-graph crash before the first snapshot does not silently
            // downgrade to open and skip GG02 validation forever after.
            None => {
                let mut bound_type = expected_bound_type.clone();
                schema_replay::replay_schema_changes(
                    &mut bound_type,
                    &self.pending_schema_changes,
                )?;
                GraphMeta {
                    graph_id: expected_graph_id,
                    generation: 0,
                    next_node_id: 1,
                    next_edge_id: 1,
                    bound_type,
                }
            }
        };
        let mut graph = SeleneGraph::new(meta.graph_id);
        graph.meta = meta;

        // BRIEF-Item-4a STEP 9 / BRIEF-Item-4c: materialize each row at its true
        // row index. Snapshot rows use their decoded position
        // (`node_snapshot_rows`); a WAL-created id absent from the snapshot
        // appends at the dense end (the live-append slot). Iteration is
        // id-ascending (BTreeMap), and `insert_node_row` pads-then-sets, so a
        // snapshot whose rows are not in id order (a compacted snapshot) still
        // lands each row at its recorded position. The hole slots between
        // recorded positions are padded with `NodeId::TOMBSTONE` and stay out of
        // the id->row map.
        let mut next_node_id = graph.meta.next_node_id.max(1);
        for (id, row) in self.nodes {
            next_node_id = next_node_id.max(id.get().saturating_add(1));
            // BRIEF-Item-4c: WAL-created ids (absent from the snapshot) APPEND at
            // the dense end, not `id - 1`. After a compacted snapshot loads (dense
            // rows, sparse high-water ids) a post-compaction `NodeCreated` would
            // otherwise re-pad the reclaimed holes on reload. WAL-created ids are
            // monotonic and greater than every snapshot id, and iteration is
            // id-ascending, so by the time one is reached every snapshot row is
            // placed and `len()` is the next dense slot — matching the live
            // append create path.
            let row_index = match self.node_snapshot_rows.get(&id) {
                Some(&position) => position as usize,
                None => {
                    let len = graph.node_store.len();
                    // u32::MAX is reserved as RowIndex::TOMBSTONE; the last real
                    // row is u32::MAX - 1, so a live row never aliases the sentinel.
                    if !u32::try_from(len).is_ok_and(|row| row != u32::MAX) {
                        return Err(crate::GraphError::Provider(invalid_payload(format!(
                            "WAL-created node id {id} exceeds the u32 row space"
                        ))));
                    }
                    len
                }
            };
            insert_node_row(&mut graph, id, row, row_index)?;
        }
        graph.meta.next_node_id = next_node_id;

        let mut next_edge_id = graph.meta.next_edge_id.max(1);
        for (id, row) in self.edges {
            next_edge_id = next_edge_id.max(id.get().saturating_add(1));
            // BRIEF-Item-4c: WAL-created edge ids APPEND at the dense end (see the
            // node arm above).
            let row_index = match self.edge_snapshot_rows.get(&id) {
                Some(&position) => position as usize,
                None => {
                    let len = graph.edge_store.len();
                    // u32::MAX is reserved as RowIndex::TOMBSTONE (see the node arm).
                    if !u32::try_from(len).is_ok_and(|row| row != u32::MAX) {
                        return Err(crate::GraphError::Provider(invalid_payload(format!(
                            "WAL-created edge id {id} exceeds the u32 row space"
                        ))));
                    }
                    len
                }
            };
            insert_edge_row(&mut graph, id, row, row_index)?;
        }
        graph.meta.next_edge_id = next_edge_id;

        // Re-register property indexes from SCMA. The empty TypedIndex placeholders
        // are filled by the downstream rebuild passes (`try_from_graph`) so the
        // registration sets survive restart even though entry contents are
        // derived from primary state.
        for (key, entry) in self.schemas {
            let target = match key.entity {
                SchemaEntityKind::Node => &mut graph.property_index,
                SchemaEntityKind::Edge => &mut graph.edge_property_index,
            };
            target.insert(
                (key.label, key.property),
                PropertyIndexEntry::new(TypedIndex::new(entry.kind), entry.name),
            );
        }
        for (key, entry) in self.composite_schemas {
            let properties = SmallVec::from_iter(key.properties);
            let kinds = SmallVec::from_iter(entry.kinds);
            let canonical_key = crate::graph::composite_property_key(&properties);
            graph.composite_property_index.insert(
                (key.label, canonical_key),
                CompositePropertyIndexEntry::new(
                    crate::CompositeTypedIndex::new(kinds),
                    properties,
                    entry.name,
                ),
            );
        }
        for (key, entry) in self.vector_schemas {
            graph.vector_index.insert(
                (key.label, key.property),
                VectorIndexEntry::new(
                    crate::VectorIndex::new_with_configs(
                        entry.kind,
                        entry.dimension,
                        entry.hnsw_config,
                        entry.ivf_config,
                    )?,
                    entry.name,
                ),
            );
        }
        for (key, entry) in self.text_schemas {
            graph.text_index.insert(
                (key.label.clone(), key.property.clone()),
                TextIndexEntry::new(crate::TextIndex::empty(key.label, key.property), entry.name),
            );
        }
        // Apply the post-snapshot WAL index intents to the registration set
        // only. `into_graph` deliberately does NOT rebuild index contents or
        // re-validate the closed-graph state here: the single rebuild + validate
        // site is `SharedGraph::from_graph_parts_and_snapshot`, which every
        // recovered graph flows through next (GRAPH-06 dedup). Doing it twice was
        // pure redundant work on the cold startup path.
        replay_property_index_changes(&mut graph, &self.pending_property_index_changes)?;
        replay_composite_property_index_changes(
            &mut graph,
            &self.pending_composite_property_index_changes,
        )?;
        replay_vector_index_changes(&mut graph, &self.pending_vector_index_changes)?;
        replay_text_index_changes(&mut graph, &self.pending_text_index_changes)?;
        Ok(graph)
    }
}

#[cfg(test)]
mod tests;
