//! Unique property validation helpers.

use std::collections::{HashMap, HashSet};

use selene_core::{Change, DbString, EdgeId, NodeId, PropertyMap, Value};

use super::{EntityId, TypeViolation, validate_edge_state, validate_node_state};
use crate::graph::SeleneGraph;
use crate::graph_types::{GraphTypeDef, PropertyTypeDef};

pub(crate) fn graph_type_has_unique_properties(type_def: &GraphTypeDef) -> bool {
    type_def
        .node_types
        .iter()
        .any(|node_type| node_type.properties.iter().any(|property| property.unique))
        || type_def
            .edge_types
            .iter()
            .any(|edge_type| edge_type.properties.iter().any(|property| property.unique))
}

#[cfg(test)]
pub(crate) fn unique_property_check_required(
    changes: &[Change],
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<bool, TypeViolation> {
    Ok(!collect_changed_unique_candidates(changes, graph, type_def)?.is_empty())
}

pub(crate) fn validate_unique_property_changes(
    changes: &[Change],
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<(), TypeViolation> {
    let candidates = collect_changed_unique_candidates(changes, graph, type_def)?;
    if candidates.is_empty() {
        return Ok(());
    }
    let (candidate_by_key, impacted_domains) = index_unique_candidates(&candidates)?;
    validate_candidate_conflicts(graph, type_def, &candidate_by_key, &impacted_domains)
}

pub(crate) fn validate_unique_property_state(
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<(), TypeViolation> {
    if !graph_type_has_unique_properties(type_def) {
        return Ok(());
    }

    let mut seen = HashMap::new();
    let nodes = graph.live_node_candidates();
    for id in nodes
        .iter_ids(graph)
        .expect("graph-owned node candidates match their snapshot")
    {
        let (node_type_index, _) = validate_node_state(id, graph, type_def)?;
        let node_type = &type_def.node_types[node_type_index as usize];
        let empty_props = PropertyMap::new();
        let properties = graph.node_properties(id).unwrap_or(&empty_props);
        record_unique_properties(
            EntityId::Node(id),
            UniqueEntityKind::Node,
            node_type.name.clone(),
            &node_type.properties,
            properties,
            &mut seen,
        )?;
    }
    let edges = graph.live_edge_candidates();
    for id in edges
        .iter_ids(graph)
        .expect("graph-owned edge candidates match their snapshot")
    {
        let (edge_type, _) = validate_edge_state(id, graph, type_def)?;
        let empty_props = PropertyMap::new();
        let properties = graph.edge_properties(id).unwrap_or(&empty_props);
        record_unique_properties(
            EntityId::Edge(id),
            UniqueEntityKind::Edge,
            edge_type.name.clone(),
            &edge_type.properties,
            properties,
            &mut seen,
        )?;
    }
    Ok(())
}

fn collect_changed_unique_candidates(
    changes: &[Change],
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<Vec<UniqueCandidate>, TypeViolation> {
    if !graph_type_has_unique_properties(type_def) {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for change in changes {
        match change {
            Change::NodeCreated { id, .. } => collect_node_candidates(
                *id,
                graph,
                type_def,
                UniqueSelection::All,
                &mut candidates,
            )?,
            Change::NodeUpdated {
                id,
                labels_diff,
                properties_diff,
            } => {
                let selection = if labels_diff.is_empty() {
                    UniqueSelection::SetProperties(properties_diff)
                } else {
                    UniqueSelection::All
                };
                collect_node_candidates(*id, graph, type_def, selection, &mut candidates)?;
            }
            Change::EdgeCreated { id, .. } => collect_edge_candidates(
                *id,
                graph,
                type_def,
                UniqueSelection::All,
                &mut candidates,
            )?,
            Change::EdgeUpdated {
                id,
                properties_diff,
            } => collect_edge_candidates(
                *id,
                graph,
                type_def,
                UniqueSelection::SetProperties(properties_diff),
                &mut candidates,
            )?,
            Change::NodeLabelRemoved { id, .. } => collect_node_candidates(
                *id,
                graph,
                type_def,
                UniqueSelection::All,
                &mut candidates,
            )?,
            Change::NodeDeleted { .. }
            | Change::EdgeDeleted { .. }
            | Change::SchemaChanged { .. }
            | Change::NodePropertyRemoved { .. }
            | Change::EdgePropertyRemoved { .. }
            | Change::NodesOfTypeTruncated { .. }
            | Change::EdgesOfTypeTruncated { .. }
            | Change::GraphReset { .. } => {}
        }
    }
    Ok(candidates)
}

fn collect_node_candidates(
    id: NodeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
    selection: UniqueSelection<'_>,
    candidates: &mut Vec<UniqueCandidate>,
) -> Result<(), TypeViolation> {
    if !graph.is_node_alive(id) {
        return Ok(());
    }
    let (node_type_index, _) = validate_node_state(id, graph, type_def)?;
    let node_type = &type_def.node_types[node_type_index as usize];
    let empty_props = PropertyMap::new();
    let properties = graph.node_properties(id).unwrap_or(&empty_props);
    collect_entity_candidates(
        EntityId::Node(id),
        UniqueEntityKind::Node,
        node_type.name.clone(),
        &node_type.properties,
        properties,
        selection,
        candidates,
    );
    Ok(())
}

fn collect_edge_candidates(
    id: EdgeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
    selection: UniqueSelection<'_>,
    candidates: &mut Vec<UniqueCandidate>,
) -> Result<(), TypeViolation> {
    if !graph.is_edge_alive(id) {
        return Ok(());
    }
    let (edge_type, _) = validate_edge_state(id, graph, type_def)?;
    let empty_props = PropertyMap::new();
    let properties = graph.edge_properties(id).unwrap_or(&empty_props);
    collect_entity_candidates(
        EntityId::Edge(id),
        UniqueEntityKind::Edge,
        edge_type.name.clone(),
        &edge_type.properties,
        properties,
        selection,
        candidates,
    );
    Ok(())
}

fn collect_entity_candidates(
    entity_id: EntityId,
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    declarations: &[PropertyTypeDef],
    properties: &PropertyMap,
    selection: UniqueSelection<'_>,
    candidates: &mut Vec<UniqueCandidate>,
) {
    for declaration in declarations
        .iter()
        .filter(|declaration| declaration.unique && selection.includes(&declaration.name))
    {
        let Some(value) = properties.get(&declaration.name) else {
            continue;
        };
        if matches!(value, Value::Null) {
            continue;
        }
        candidates.push(UniqueCandidate {
            entity_id,
            key: UniquePropertyKey {
                entity_kind,
                declared_in: declared_in.clone(),
                property: declaration.name.clone(),
                value: UniqueValueKey::new(value),
            },
        });
    }
}

fn index_unique_candidates(
    candidates: &[UniqueCandidate],
) -> Result<
    (
        HashMap<UniquePropertyKey, EntityId>,
        HashSet<UniquePropertyDomain>,
    ),
    TypeViolation,
> {
    let mut candidate_by_key = HashMap::with_capacity(candidates.len());
    let mut impacted_domains = HashSet::new();
    for candidate in candidates {
        impacted_domains.insert(candidate.key.domain());
        if let Some(conflicting_entity_id) =
            candidate_by_key.insert(candidate.key.clone(), candidate.entity_id)
        {
            if conflicting_entity_id == candidate.entity_id {
                continue;
            }
            return Err(TypeViolation::UniquePropertyDuplicate {
                entity_id: candidate.entity_id,
                conflicting_entity_id,
                property: candidate.key.property.clone(),
                declared_in: candidate.key.declared_in.clone(),
            });
        }
    }
    Ok((candidate_by_key, impacted_domains))
}

fn validate_candidate_conflicts(
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
    candidate_by_key: &HashMap<UniquePropertyKey, EntityId>,
    impacted_domains: &HashSet<UniquePropertyDomain>,
) -> Result<(), TypeViolation> {
    let nodes = graph.live_node_candidates();
    for id in nodes
        .iter_ids(graph)
        .expect("graph-owned node candidates match their snapshot")
    {
        let (node_type_index, _) = validate_node_state(id, graph, type_def)?;
        let node_type = &type_def.node_types[node_type_index as usize];
        let empty_props = PropertyMap::new();
        let properties = graph.node_properties(id).unwrap_or(&empty_props);
        validate_entity_candidate_conflicts(
            EntityId::Node(id),
            UniqueEntityKind::Node,
            node_type.name.clone(),
            &node_type.properties,
            properties,
            candidate_by_key,
            impacted_domains,
        )?;
    }
    let edges = graph.live_edge_candidates();
    for id in edges
        .iter_ids(graph)
        .expect("graph-owned edge candidates match their snapshot")
    {
        let (edge_type, _) = validate_edge_state(id, graph, type_def)?;
        let empty_props = PropertyMap::new();
        let properties = graph.edge_properties(id).unwrap_or(&empty_props);
        validate_entity_candidate_conflicts(
            EntityId::Edge(id),
            UniqueEntityKind::Edge,
            edge_type.name.clone(),
            &edge_type.properties,
            properties,
            candidate_by_key,
            impacted_domains,
        )?;
    }
    Ok(())
}

fn validate_entity_candidate_conflicts(
    entity_id: EntityId,
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    declarations: &[PropertyTypeDef],
    properties: &PropertyMap,
    candidate_by_key: &HashMap<UniquePropertyKey, EntityId>,
    impacted_domains: &HashSet<UniquePropertyDomain>,
) -> Result<(), TypeViolation> {
    for declaration in declarations.iter().filter(|property| property.unique) {
        let domain = UniquePropertyDomain {
            entity_kind,
            declared_in: declared_in.clone(),
            property: declaration.name.clone(),
        };
        if !impacted_domains.contains(&domain) {
            continue;
        }
        let Some(value) = properties.get(&declaration.name) else {
            continue;
        };
        if matches!(value, Value::Null) {
            continue;
        }
        let key = UniquePropertyKey {
            value: UniqueValueKey::new(value),
            entity_kind: domain.entity_kind,
            declared_in: domain.declared_in.clone(),
            property: domain.property.clone(),
        };
        if let Some(candidate_entity_id) = candidate_by_key.get(&key).copied()
            && candidate_entity_id != entity_id
        {
            return Err(TypeViolation::UniquePropertyDuplicate {
                entity_id: candidate_entity_id,
                conflicting_entity_id: entity_id,
                property: key.property,
                declared_in: key.declared_in,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UniqueCandidate {
    entity_id: EntityId,
    key: UniquePropertyKey,
}

#[derive(Clone, Copy)]
enum UniqueSelection<'a> {
    All,
    SetProperties(&'a selene_core::PropertyDiff),
}

impl UniqueSelection<'_> {
    fn includes(self, property: &DbString) -> bool {
        match self {
            Self::All => true,
            Self::SetProperties(diff) => diff
                .set
                .iter()
                .any(|(changed_property, _)| changed_property == property),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct UniquePropertyKey {
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    property: DbString,
    value: UniqueValueKey,
}

impl UniquePropertyKey {
    fn domain(&self) -> UniquePropertyDomain {
        UniquePropertyDomain {
            entity_kind: self.entity_kind,
            declared_in: self.declared_in.clone(),
            property: self.property.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct UniquePropertyDomain {
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    property: DbString,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum UniqueEntityKind {
    Node,
    Edge,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct UniqueValueKey(Vec<u8>);

impl UniqueValueKey {
    fn new(value: &Value) -> Self {
        let mut bytes = Vec::new();
        write_value_key(value, &mut bytes);
        Self(bytes)
    }
}

fn record_unique_properties(
    entity_id: EntityId,
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    declarations: &[PropertyTypeDef],
    properties: &PropertyMap,
    seen: &mut HashMap<UniquePropertyKey, EntityId>,
) -> Result<(), TypeViolation> {
    for declaration in declarations.iter().filter(|property| property.unique) {
        let Some(value) = properties.get(&declaration.name) else {
            continue;
        };
        if matches!(value, Value::Null) {
            continue;
        }
        let key = UniquePropertyKey {
            entity_kind,
            declared_in: declared_in.clone(),
            property: declaration.name.clone(),
            value: UniqueValueKey::new(value),
        };
        if let Some(conflicting_entity_id) = seen.get(&key).copied() {
            return Err(TypeViolation::UniquePropertyDuplicate {
                entity_id,
                conflicting_entity_id,
                property: declaration.name.clone(),
                declared_in,
            });
        }
        seen.insert(key, entity_id);
    }
    Ok(())
}

fn write_value_key(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Bool(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        Value::Int(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::Uint(value) => {
            out.push(3);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::Int128(value) => {
            out.push(4);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::Uint128(value) => {
            out.push(5);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::Float(value) => {
            out.push(6);
            out.extend_from_slice(&canonical_f64_bits(*value).to_le_bytes());
        }
        Value::Float32(value) => {
            out.push(7);
            out.extend_from_slice(&canonical_f32_bits(*value).to_le_bytes());
        }
        Value::Decimal(value) => {
            write_variable_key(8, value.normalize().to_string().as_bytes(), out)
        }
        Value::String(value) => write_variable_key(9, value.as_str().as_bytes(), out),
        Value::Bytes(value) => write_variable_key(10, value, out),
        Value::List(values) => {
            out.push(11);
            write_len(values.len(), out);
            for value in values {
                write_value_key(value, out);
            }
        }
        Value::Record(record) => {
            out.push(12);
            match record.as_ref() {
                selene_core::Record::Open(fields) => {
                    write_len(fields.len(), out);
                    for (name, value) in fields {
                        write_variable_key(0, name.as_str().as_bytes(), out);
                        write_value_key(value, out);
                    }
                }
                _ => write_fallback_key(record.as_ref(), out),
            }
        }
        Value::RecordTyped(record) => {
            out.push(13);
            write_fallback_key(record.type_id, out);
            write_len(record.values.len(), out);
            for value in &record.values {
                match value {
                    Some(value) => {
                        out.push(1);
                        write_value_key(value, out);
                    }
                    None => out.push(0),
                }
            }
        }
        Value::Json(value) => write_variable_key(30, value.to_canonical_string().as_bytes(), out),
        Value::Vector(value) => {
            out.push(31);
            write_len(value.dimension(), out);
            for component in value.as_slice() {
                out.extend_from_slice(&canonical_f32_bits(*component).to_le_bytes());
            }
        }
        _ => {
            out.push(255);
            write_fallback_key(value, out);
        }
    }
}

fn write_variable_key(tag: u8, bytes: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    write_len(bytes.len(), out);
    out.extend_from_slice(bytes);
}

fn write_len(len: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&(len as u64).to_le_bytes());
}

fn write_fallback_key<T: serde::Serialize>(value: T, out: &mut Vec<u8>) {
    let bytes = postcard::to_allocvec(&value).expect("Value uniqueness key payload serializes");
    write_len(bytes.len(), out);
    out.extend_from_slice(&bytes);
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value.is_nan() {
        f64::NAN.to_bits()
    } else if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

fn canonical_f32_bits(value: f32) -> u32 {
    if value.is_nan() {
        f32::NAN.to_bits()
    } else if value == 0.0 {
        0.0_f32.to_bits()
    } else {
        value.to_bits()
    }
}
