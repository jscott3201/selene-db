//! Recovery-mode state for the core graph provider.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use selene_core::{
    Change, EdgeId, GraphId, IStr, LabelSet, NodeId, PredefinedValueType, PropertyDiff,
    PropertyMap, PropertyValueType, SchemaChange, ValueType,
};

use crate::core_provider::sections::{
    EdgeRow, MetaPayload, NodeRow, SchemaEntry, SchemaKey, decode_edges, decode_graph_types,
    decode_meta, decode_nodes, decode_schemas,
};
use crate::core_provider::{
    CORE_EDGE_SUB, CORE_GTYP_SUB, CORE_META_SUB, CORE_NODE_SUB, CORE_SCMA_SUB, inconsistent,
    invalid_payload,
};
use crate::graph::{GraphMeta, SeleneGraph};
use crate::graph_types::{EdgeTypeDef, GraphTypeDef, NodeTypeDef, PropertyTypeDef};
use crate::store::{edge_row_index, node_row_index};
use crate::typed_index::{TypedIndex, TypedIndexKind};

/// Accumulator populated by snapshot sections and WAL replay.
#[derive(Default)]
pub(crate) struct RecoveryState {
    meta: Option<MetaPayload>,
    graph_types: BTreeMap<u32, Arc<GraphTypeDef>>,
    pending_schema_changes: Vec<SchemaChange>,
    pending_property_index_changes: Vec<PendingIndex>,
    nodes: BTreeMap<NodeId, NodeRow>,
    edges: BTreeMap<EdgeId, EdgeRow>,
    schemas: BTreeMap<SchemaKey, SchemaEntry>,
    sequence: u64,
}

const V1_BOUND_GRAPH_TYPE_INDEX: u32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingIndex {
    Create {
        label: IStr,
        property: IStr,
        kind: TypedIndexKind,
    },
    Drop {
        label: IStr,
        property: IStr,
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
            Change::SchemaChanged { change, .. } => {
                match change {
                    SchemaChange::NodeTypeAdded { .. }
                    | SchemaChange::EdgeTypeAdded { .. }
                    | SchemaChange::NodeTypeDropped { .. }
                    | SchemaChange::EdgeTypeDropped { .. } => {
                        self.pending_schema_changes.push(change.clone());
                    }
                    SchemaChange::PropertyIndexCreated { .. }
                    | SchemaChange::PropertyIndexDropped { .. } => {
                        let pending = pending_property_index_change(change)
                            .expect("property-index variants map to pending recovery intent");
                        self.pending_property_index_changes.push(pending);
                    }
                    SchemaChange::ProcedurePackLifecycle { .. } => {
                        // Procedure-pack lifecycle changes are pure audit history.
                        // Pack-history readers consume them from the WAL directly;
                        // graph-state recovery has no materialized state to update.
                    }
                    SchemaChange::ProcedurePackActivated { .. }
                    | SchemaChange::ProcedurePackDeprecated { .. }
                    | SchemaChange::ProcedurePackDisabled { .. } => {
                        // Why: legacy, never emitted; postcard discriminant
                        // pinned for ABI stability.
                    }
                    SchemaChange::GraphCreated { .. }
                    | SchemaChange::GraphDropped { .. }
                    | SchemaChange::GraphTypeCreated { .. }
                    | SchemaChange::GraphTypeDropped { .. }
                    | SchemaChange::RecordTypeAdded { .. } => {
                        return Err(unsupported_schema_recovery(change));
                    }
                }
            }
            Change::IndexExtensionEvent { .. } => {}
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
        let meta = match self.meta {
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
                replay_schema_changes(&mut bound_type, &self.pending_schema_changes)?;
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
                replay_schema_changes(&mut bound_type, &self.pending_schema_changes)?;
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
        replay_property_index_changes(&mut graph, &self.pending_property_index_changes)?;
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
            } => {
                let index = crate::property_index::build_property_index_lenient(
                    graph, label, property, kind,
                )?;
                graph
                    .property_index
                    .insert((label, property), Arc::new(index));
            }
            PendingIndex::Drop { label, property } => {
                graph.property_index.remove(&(label, property));
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

fn replay_schema_changes(
    bound_type: &mut Option<Arc<GraphTypeDef>>,
    changes: &[SchemaChange],
) -> Result<(), crate::GraphError> {
    if changes.is_empty() {
        return Ok(());
    }
    let Some(base) = bound_type.as_deref() else {
        let variant = schema_change_variant(&changes[0]);
        return Err(crate::GraphError::Provider(inconsistent(format!(
            "WAL {variant} references missing graph type index {V1_BOUND_GRAPH_TYPE_INDEX}"
        ))));
    };
    let mut graph_type = base.clone();
    for change in changes {
        apply_schema_change(&mut graph_type, change).map_err(crate::GraphError::Provider)?;
    }
    graph_type.validate_ref()?;
    *bound_type = Some(Arc::new(graph_type));
    Ok(())
}

/// Apply a graph-type schema change to the recovered bound graph type.
///
/// This match is exhaustive by design. Variants that are handled outside
/// graph-type replay or intentionally ignored must say so inline; unsupported
/// variants reject loudly so recovery never silently drops new durable schema
/// payloads. See the `SCHEMA_CHANGE_INTENT` test table for the executable
/// contract.
fn apply_schema_change(
    graph_type: &mut GraphTypeDef,
    change: &SchemaChange,
) -> Result<(), crate::ProviderError> {
    match change {
        SchemaChange::GraphCreated { .. }
        | SchemaChange::GraphDropped { .. }
        | SchemaChange::GraphTypeCreated { .. }
        | SchemaChange::GraphTypeDropped { .. }
        | SchemaChange::RecordTypeAdded { .. } => {
            return Err(unsupported_schema_recovery(change));
        }
        SchemaChange::NodeTypeAdded { label, def, .. } => {
            // Why: snapshot sections store the bound graph type at recovery
            // index 0 in v1.0. SchemaChange.graph_type uses the durable
            // catalog id space, so replay maps every catalog type DDL event
            // onto that single recovery-state entry until multi-bound-type
            // work replaces the constant.
            graph_type
                .node_types
                .push(runtime_node_type_def(*label, def)?);
        }
        SchemaChange::EdgeTypeAdded { label, def, .. } => {
            graph_type
                .edge_types
                .push(runtime_edge_type_def(graph_type, *label, def)?);
        }
        SchemaChange::NodeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_node_type(*name).ok_or_else(|| {
                inconsistent(format!(
                    "WAL NodeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::EdgeTypeDropped { name, .. } => {
            *graph_type = graph_type.without_edge_type(*name).ok_or_else(|| {
                inconsistent(format!(
                    "WAL EdgeTypeDropped references unknown type {name}"
                ))
            })?;
        }
        SchemaChange::ProcedurePackActivated { .. }
        | SchemaChange::ProcedurePackDeprecated { .. }
        | SchemaChange::ProcedurePackDisabled { .. } => {
            // Why: legacy, never emitted; postcard discriminant pinned for ABI
            // stability.
        }
        SchemaChange::PropertyIndexCreated { .. } | SchemaChange::PropertyIndexDropped { .. } => {
            // Why: property-index intent is queued by apply_change and replayed
            // after primary node/edge rows materialize.
        }
        SchemaChange::ProcedurePackLifecycle { .. } => {
            // Why: lifecycle events are audit history consumed from the WAL by
            // selene-pack; CORE graph recovery has no materialized state.
        }
    }
    Ok(())
}

fn runtime_node_type_def(
    label: IStr,
    def: &selene_core::NodeTypeDef,
) -> Result<NodeTypeDef, crate::ProviderError> {
    Ok(NodeTypeDef {
        name: label,
        key_labels: def.labels.clone(),
        properties: runtime_properties(&def.properties)?,
    })
}

fn runtime_edge_type_def(
    graph_type: &GraphTypeDef,
    label: IStr,
    def: &selene_core::EdgeTypeDef,
) -> Result<EdgeTypeDef, crate::ProviderError> {
    let source_node_type = graph_type
        .find_node_type_index(&LabelSet::single(def.source_node_type.0))
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAdded references unknown source node type {}",
                def.source_node_type.0
            ))
        })?;
    let target_node_type = graph_type
        .find_node_type_index(&LabelSet::single(def.target_node_type.0))
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAdded references unknown target node type {}",
                def.target_node_type.0
            ))
        })?;
    Ok(EdgeTypeDef {
        name: label,
        label: def.label,
        source_node_type,
        target_node_type,
        properties: runtime_properties(&def.properties)?,
    })
}

fn runtime_properties(
    properties: &[selene_core::PropertyDef],
) -> Result<Vec<PropertyTypeDef>, crate::ProviderError> {
    properties
        .iter()
        .map(|property| {
            Ok(PropertyTypeDef {
                name: property.name,
                value_type: runtime_value_type(&property.value_type)?,
                required: !property.nullable || property.value_type.not_null,
            })
        })
        .collect()
}

fn runtime_value_type(value_type: &ValueType) -> Result<PropertyValueType, crate::ProviderError> {
    if value_type.list_of.is_some() {
        return Ok(PropertyValueType::List);
    }
    if value_type.record.is_some() {
        return Ok(PropertyValueType::RecordTyped);
    }
    if value_type.union.is_some() {
        return Err(inconsistent(
            "WAL property definition uses unsupported union value type",
        ));
    }
    let Some(predefined) = value_type.predefined else {
        return Ok(PropertyValueType::Null);
    };
    Ok(match predefined {
        PredefinedValueType::Bool => PropertyValueType::Bool,
        PredefinedValueType::Int
        | PredefinedValueType::Int8
        | PredefinedValueType::Int16
        | PredefinedValueType::Int32
        | PredefinedValueType::Int64 => PropertyValueType::Int,
        PredefinedValueType::Int128 => PropertyValueType::Int128,
        PredefinedValueType::Uint
        | PredefinedValueType::Uint8
        | PredefinedValueType::Uint16
        | PredefinedValueType::Uint32
        | PredefinedValueType::Uint64 => PropertyValueType::Uint,
        PredefinedValueType::Uint128 => PropertyValueType::Uint128,
        PredefinedValueType::Float | PredefinedValueType::Float64 => PropertyValueType::Float,
        PredefinedValueType::Float32 => PropertyValueType::Float32,
        PredefinedValueType::Decimal => PropertyValueType::Decimal,
        PredefinedValueType::String => PropertyValueType::String,
        PredefinedValueType::Bytes => PropertyValueType::Bytes,
        PredefinedValueType::Date => PropertyValueType::Date,
        PredefinedValueType::LocalTime => PropertyValueType::LocalTime,
        PredefinedValueType::ZonedTime => PropertyValueType::ZonedTime,
        PredefinedValueType::LocalDateTime => PropertyValueType::LocalDateTime,
        PredefinedValueType::ZonedDateTime => PropertyValueType::ZonedDateTime,
        PredefinedValueType::Duration => PropertyValueType::Duration,
        PredefinedValueType::NodeRef => PropertyValueType::NodeRef,
        PredefinedValueType::EdgeRef => PropertyValueType::EdgeRef,
        PredefinedValueType::GraphRef => PropertyValueType::GraphRef,
        PredefinedValueType::TableRef => PropertyValueType::TableRef,
        PredefinedValueType::Path => PropertyValueType::Path,
        PredefinedValueType::Uuid => PropertyValueType::Uuid,
        PredefinedValueType::Extended(_) => {
            return Err(inconsistent(
                "WAL property definition uses unsupported extended value type",
            ));
        }
    })
}

/// Return the diagnostic name for a schema-change payload.
///
/// Exhaustive naming is part of the silent-skip-forbidden contract and is
/// covered by the `SCHEMA_CHANGE_INTENT` test table.
fn schema_change_variant(change: &SchemaChange) -> &'static str {
    match change {
        SchemaChange::GraphCreated { .. } => "GraphCreated",
        SchemaChange::GraphDropped { .. } => "GraphDropped",
        SchemaChange::GraphTypeCreated { .. } => "GraphTypeCreated",
        SchemaChange::GraphTypeDropped { .. } => "GraphTypeDropped",
        SchemaChange::NodeTypeAdded { .. } => "NodeTypeAdded",
        SchemaChange::EdgeTypeAdded { .. } => "EdgeTypeAdded",
        SchemaChange::NodeTypeDropped { .. } => "NodeTypeDropped",
        SchemaChange::EdgeTypeDropped { .. } => "EdgeTypeDropped",
        SchemaChange::RecordTypeAdded { .. } => "RecordTypeAdded",
        SchemaChange::ProcedurePackActivated { .. } => "ProcedurePackActivated",
        SchemaChange::ProcedurePackDeprecated { .. } => "ProcedurePackDeprecated",
        SchemaChange::ProcedurePackDisabled { .. } => "ProcedurePackDisabled",
        SchemaChange::PropertyIndexCreated { .. } => "PropertyIndexCreated",
        SchemaChange::PropertyIndexDropped { .. } => "PropertyIndexDropped",
        SchemaChange::ProcedurePackLifecycle { .. } => "ProcedurePackLifecycle",
    }
}

fn unsupported_schema_recovery(change: &SchemaChange) -> crate::ProviderError {
    inconsistent(format!(
        "WAL {} is not supported by CORE graph recovery; add an explicit recovery intent",
        schema_change_variant(change)
    ))
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

#[cfg(test)]
mod tests;
