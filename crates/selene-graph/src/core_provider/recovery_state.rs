//! Recovery-mode state for the core graph provider.

use std::collections::BTreeMap;
use std::sync::Arc;

use selene_core::{Change, EdgeId, GraphId, IStr, NodeId, PropertyDiff, PropertyMap, SchemaChange};
use smallvec::SmallVec;

use crate::core_provider::sections::{
    CompositeSchemaEntry, CompositeSchemaKey, EdgeRow, MetaPayload, NodeRow, SchemaEntry,
    SchemaKey, decode_composite_schemas, decode_edges, decode_graph_types, decode_meta,
    decode_nodes, decode_schemas,
};
use crate::core_provider::{
    CORE_CPIX_SUB, CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_SCMA_SUB,
    inconsistent, invalid_payload,
};
use crate::graph::{CompositePropertyIndexEntry, GraphMeta, PropertyIndexEntry, SeleneGraph};
use crate::graph_types::GraphTypeDef;
use crate::typed_index::{TypedIndex, TypedIndexKind};

mod materialize;
mod schema_replay;

use materialize::{insert_edge_row, insert_node_row};

/// Accumulator populated by snapshot sections and WAL replay.
#[derive(Default)]
pub(crate) struct RecoveryState {
    meta: Option<MetaPayload>,
    graph_types: BTreeMap<u32, Arc<GraphTypeDef>>,
    pending_schema_changes: Vec<SchemaChange>,
    pending_property_index_changes: Vec<PendingIndex>,
    pending_composite_property_index_changes: Vec<PendingCompositeIndex>,
    nodes: BTreeMap<NodeId, NodeRow>,
    edges: BTreeMap<EdgeId, EdgeRow>,
    /// BRIEF-Item-4a STEP 9: the snapshot row (= section position) each
    /// committed id was decoded at, so `into_graph` materializes snapshot rows
    /// **positionally** instead of by `id - 1` arithmetic. Aborted-tx hole rows
    /// (`*Id::TOMBSTONE`) are not recorded — they are re-materialized as the
    /// pad slots between the real rows the column places. WAL-created ids absent
    /// here fall back to arithmetic placement (live append; 4e revisits this for
    /// WAL events that cross a 4b compaction epoch).
    node_snapshot_rows: BTreeMap<NodeId, u32>,
    edge_snapshot_rows: BTreeMap<EdgeId, u32>,
    schemas: BTreeMap<SchemaKey, SchemaEntry>,
    composite_schemas: Vec<(CompositeSchemaKey, CompositeSchemaEntry)>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingIndex {
    Create {
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
        name: Option<IStr>,
    },
    Drop {
        label: IStr,
        property: IStr,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingCompositeIndex {
    Create {
        label: IStr,
        properties: SmallVec<[IStr; 4]>,
        kinds: SmallVec<[TypedIndexKind; 4]>,
        name: Option<IStr>,
    },
    Drop {
        label: IStr,
        properties: SmallVec<[IStr; 4]>,
    },
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
            _ => {
                return Err(invalid_payload(format!("unknown CORE sub-tag {sub_tag}")));
            }
        }
        Ok(())
    }

    /// Apply one WAL change to recovery state.
    ///
    /// `SchemaChange` routing is intentionally exhaustive: silent-skip
    /// wildcards are forbidden. The executable intent matrix lives in
    /// `SCHEMA_CHANGE_INTENT` in this module's tests; new variants must update
    /// both this match and that table.
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
            Change::NodePropertyRemoved { id, property } => {
                let row = require_live_node(&mut self.nodes, *id)?;
                row.properties.remove(property);
            }
            Change::EdgePropertyRemoved { id, property } => {
                let row = require_live_edge(&mut self.edges, *id)?;
                row.properties.remove(property);
            }
            Change::NodeLabelRemoved { id, label } => {
                let row = require_live_node(&mut self.nodes, *id)?;
                row.labels.remove(label);
            }
            Change::NodesOfTypeTruncated { label } => {
                // Re-derive the truncated rows from the recovered store: every
                // alive node carrying the label, plus every alive edge incident
                // to such a node (any edge type). This reconstructs the exact
                // post-`delete_node`-cascade state without persisting any ids.
                let mut truncated_nodes = std::collections::BTreeSet::new();
                for (id, row) in self.nodes.iter_mut() {
                    if row.alive && row.labels.contains(label) {
                        row.alive = false;
                        truncated_nodes.insert(*id);
                    }
                }
                for row in self.edges.values_mut() {
                    if row.alive
                        && (truncated_nodes.contains(&row.source)
                            || truncated_nodes.contains(&row.target))
                    {
                        row.alive = false;
                    }
                }
            }
            Change::EdgesOfTypeTruncated { label } => {
                for row in self.edges.values_mut() {
                    if row.alive && row.label == *label {
                        row.alive = false;
                    }
                }
            }
            Change::GraphReset {} => {
                // Re-derive every live row from the recovered store at this WAL
                // position and mark it dead — identical to the runtime mutator,
                // which carries no ids in the declarative change ("replay walks
                // the store"). Wipes ALL nodes/edges incl untyped ones.
                for row in self.nodes.values_mut() {
                    row.alive = false;
                }
                for row in self.edges.values_mut() {
                    row.alive = false;
                }
                // A reset moots every prior schema/index intent in the WAL up to
                // this point, and forces the recovered graph open.
                self.schema_reset_to_open = true;
                self.pending_schema_changes.clear();
                self.pending_property_index_changes.clear();
                self.pending_composite_property_index_changes.clear();
            }
            Change::SchemaChanged { change, .. } => match change {
                SchemaChange::NodeTypeAdded { .. }
                | SchemaChange::EdgeTypeAdded { .. }
                | SchemaChange::NodeTypeAddedV2 { .. }
                | SchemaChange::EdgeTypeAddedV2 { .. }
                | SchemaChange::NodeTypeDropped { .. }
                | SchemaChange::EdgeTypeDropped { .. } => {
                    self.pending_schema_changes.push(change.clone());
                }
                SchemaChange::PropertyIndexCreated { .. }
                | SchemaChange::PropertyIndexCreatedNamed { .. }
                | SchemaChange::PropertyIndexDropped { .. } => {
                    let pending = pending_property_index_change(change)
                        .expect("property-index variants map to pending recovery intent");
                    self.pending_property_index_changes.push(pending);
                }
                SchemaChange::CompositePropertyIndexCreated { .. }
                | SchemaChange::CompositePropertyIndexDropped { .. } => {
                    let pending = pending_composite_property_index_change(change)
                        .expect("composite property-index variants map to pending recovery intent");
                    self.pending_composite_property_index_changes.push(pending);
                }
                SchemaChange::GraphCreated { .. }
                | SchemaChange::GraphDropped { .. }
                | SchemaChange::GraphTypeCreated { .. }
                | SchemaChange::GraphTypeDropped { .. }
                | SchemaChange::RecordTypeAdded { .. } => {
                    return Err(schema_replay::unsupported_schema_recovery(change));
                }
            },
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

        // BRIEF-Item-4a STEP 9: materialize each row at its true row index.
        // Snapshot rows use their decoded position (`node_snapshot_rows`); a
        // WAL-created id absent from the snapshot falls back to `id - 1`
        // arithmetic (the live append slot in the identity-era store). Iteration
        // is id-ascending (BTreeMap), and `insert_node_row` pads-then-sets, so an
        // out-of-order snapshot (id != row+1, a 4b preview) still lands each row
        // at its recorded position. The hole slots between recorded positions are
        // padded with `NodeId::TOMBSTONE` and stay out of the id->row map.
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
        // are filled by `rebuild_property_indexes` (called downstream via
        // `try_from_graph`) so the registration set survives restart even though
        // the entry contents are derived from primary state.
        for (key, entry) in self.schemas {
            graph.property_index.insert(
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
        crate::composite_property_index::rebuild_composite_property_indexes(&mut graph)?;
        replay_property_index_changes(&mut graph, &self.pending_property_index_changes)?;
        replay_composite_property_index_changes(
            &mut graph,
            &self.pending_composite_property_index_changes,
        )?;
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

fn pending_property_index_change(change: &SchemaChange) -> Option<PendingIndex> {
    match change {
        SchemaChange::PropertyIndexCreated {
            label,
            property,
            kind,
        } => Some(PendingIndex::Create {
            label: *label,
            property: *property,
            kind: typed_kind_from(*kind),
            name: None,
        }),
        SchemaChange::PropertyIndexCreatedNamed {
            label,
            property,
            kind,
            name,
        } => Some(PendingIndex::Create {
            label: *label,
            property: *property,
            kind: typed_kind_from(*kind),
            name: *name,
        }),
        SchemaChange::PropertyIndexDropped { label, property } => Some(PendingIndex::Drop {
            label: *label,
            property: *property,
        }),
        _ => None,
    }
}

fn replay_property_index_changes(
    graph: &mut SeleneGraph,
    changes: &[PendingIndex],
) -> crate::GraphResult<()> {
    for change in changes {
        match *change {
            PendingIndex::Create {
                label,
                property,
                kind,
                name,
            } => {
                let index = crate::property_index::build_property_index_lenient(
                    graph, label, property, kind,
                )?;
                graph
                    .property_index
                    .insert((label, property), PropertyIndexEntry::new(index, name));
            }
            PendingIndex::Drop { label, property } => {
                graph.property_index.remove(&(label, property));
            }
        }
    }
    Ok(())
}

fn pending_composite_property_index_change(change: &SchemaChange) -> Option<PendingCompositeIndex> {
    match change {
        SchemaChange::CompositePropertyIndexCreated {
            label,
            properties,
            kinds,
            name,
        } => Some(PendingCompositeIndex::Create {
            label: *label,
            properties: properties.clone(),
            kinds: kinds.iter().copied().map(typed_kind_from).collect(),
            name: *name,
        }),
        SchemaChange::CompositePropertyIndexDropped { label, properties } => {
            Some(PendingCompositeIndex::Drop {
                label: *label,
                properties: properties.clone(),
            })
        }
        _ => None,
    }
}

fn replay_composite_property_index_changes(
    graph: &mut SeleneGraph,
    changes: &[PendingCompositeIndex],
) -> crate::GraphResult<()> {
    for change in changes {
        match change {
            PendingCompositeIndex::Create {
                label,
                properties,
                kinds,
                name,
            } => {
                let index =
                    crate::composite_property_index::build_composite_property_index_lenient(
                        graph,
                        *label,
                        properties.clone(),
                        kinds.clone(),
                    )?;
                let key = crate::graph::composite_property_key(properties);
                graph.composite_property_index.insert(
                    (*label, key),
                    CompositePropertyIndexEntry::new(index, properties.clone(), *name),
                );
            }
            PendingCompositeIndex::Drop { label, properties } => {
                let key = crate::graph::composite_property_key(properties);
                graph.composite_property_index.remove(&(*label, key));
            }
        }
    }
    Ok(())
}

const fn typed_kind_from(kind: selene_core::SchemaPropertyIndexKind) -> TypedIndexKind {
    match kind {
        selene_core::SchemaPropertyIndexKind::I64 => TypedIndexKind::I64,
        selene_core::SchemaPropertyIndexKind::F64 => TypedIndexKind::F64,
        selene_core::SchemaPropertyIndexKind::String => TypedIndexKind::String,
        selene_core::SchemaPropertyIndexKind::Date => TypedIndexKind::Date,
        selene_core::SchemaPropertyIndexKind::LocalDateTime => TypedIndexKind::LocalDateTime,
        selene_core::SchemaPropertyIndexKind::Uuid => TypedIndexKind::Uuid,
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

#[cfg(test)]
mod tests;
