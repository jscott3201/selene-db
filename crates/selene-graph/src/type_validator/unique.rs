//! Unique property validation helpers.

use std::collections::HashMap;

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

pub(crate) fn unique_property_check_required(
    changes: &[Change],
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<bool, TypeViolation> {
    if !graph_type_has_unique_properties(type_def) {
        return Ok(false);
    }

    for change in changes {
        match change {
            Change::NodeCreated { id, .. } => {
                if node_unique_value_present(*id, graph, type_def)? {
                    return Ok(true);
                }
            }
            Change::NodeUpdated {
                id,
                labels_diff,
                properties_diff,
            } => {
                if !graph.is_node_alive(*id) {
                    continue;
                }
                let declarations = node_type_properties(*id, graph, type_def)?;
                if !labels_diff.is_empty()
                    && node_unique_value_present_for(*id, graph, declarations)
                {
                    return Ok(true);
                }
                if diff_sets_unique_value(declarations, properties_diff) {
                    return Ok(true);
                }
            }
            Change::EdgeCreated { id, .. } => {
                if edge_unique_value_present(*id, graph, type_def)? {
                    return Ok(true);
                }
            }
            Change::EdgeUpdated {
                id,
                properties_diff,
            } => {
                if !graph.is_edge_alive(*id) {
                    continue;
                }
                let declarations = edge_type_properties(*id, graph, type_def)?;
                if diff_sets_unique_value(declarations, properties_diff) {
                    return Ok(true);
                }
            }
            Change::NodeLabelRemoved { id, .. } => {
                if node_unique_value_present(*id, graph, type_def)? {
                    return Ok(true);
                }
            }
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

    Ok(false)
}

fn node_unique_value_present(
    id: NodeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<bool, TypeViolation> {
    if !graph.is_node_alive(id) {
        return Ok(false);
    }
    let declarations = node_type_properties(id, graph, type_def)?;
    Ok(node_unique_value_present_for(id, graph, declarations))
}

fn node_unique_value_present_for(
    id: NodeId,
    graph: &SeleneGraph,
    declarations: &[PropertyTypeDef],
) -> bool {
    let empty_props = PropertyMap::new();
    let properties = graph.node_properties(id).unwrap_or(&empty_props);
    unique_value_present(declarations, properties)
}

fn edge_unique_value_present(
    id: EdgeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<bool, TypeViolation> {
    if !graph.is_edge_alive(id) {
        return Ok(false);
    }
    let declarations = edge_type_properties(id, graph, type_def)?;
    let empty_props = PropertyMap::new();
    let properties = graph.edge_properties(id).unwrap_or(&empty_props);
    Ok(unique_value_present(declarations, properties))
}

fn node_type_properties<'a>(
    id: NodeId,
    graph: &SeleneGraph,
    type_def: &'a GraphTypeDef,
) -> Result<&'a [PropertyTypeDef], TypeViolation> {
    let (node_type_index, _) = validate_node_state(id, graph, type_def)?;
    Ok(&type_def.node_types[node_type_index as usize].properties)
}

fn edge_type_properties<'a>(
    id: EdgeId,
    graph: &SeleneGraph,
    type_def: &'a GraphTypeDef,
) -> Result<&'a [PropertyTypeDef], TypeViolation> {
    let (edge_type, _) = validate_edge_state(id, graph, type_def)?;
    Ok(&edge_type.properties)
}

fn unique_value_present(declarations: &[PropertyTypeDef], properties: &PropertyMap) -> bool {
    declarations
        .iter()
        .filter(|declaration| declaration.unique)
        .any(|declaration| {
            properties
                .get(&declaration.name)
                .is_some_and(|value| !matches!(value, Value::Null))
        })
}

fn diff_sets_unique_value(
    declarations: &[PropertyTypeDef],
    diff: &selene_core::PropertyDiff,
) -> bool {
    diff.set.iter().any(|(property, value)| {
        !matches!(value, Value::Null)
            && declarations
                .iter()
                .any(|declaration| declaration.unique && declaration.name == *property)
    })
}

pub(crate) fn validate_unique_property_state(
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<(), TypeViolation> {
    if !graph_type_has_unique_properties(type_def) {
        return Ok(());
    }

    let mut seen = HashMap::new();
    for row in graph.node_store.alive.iter() {
        let id = graph
            .node_id_for_row(crate::store::RowIndex::new(row))
            .expect("alive node row has a mapped external id (BRIEF-Item-4a)");
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
    for row in graph.edge_store.alive.iter() {
        let id = graph
            .edge_id_for_row(crate::store::RowIndex::new(row))
            .expect("alive edge row has a mapped external id (BRIEF-Item-4a)");
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct UniquePropertyKey {
    entity_kind: UniqueEntityKind,
    declared_in: DbString,
    property: DbString,
    value: UniqueValueKey,
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
