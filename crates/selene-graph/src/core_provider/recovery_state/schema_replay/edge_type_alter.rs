//! Additive edge-type ALTER validation during WAL replay.

use selene_core::{DbString, EdgeEndpointDef as CoreEdgeEndpointDef, PropertyDef};

use crate::{EdgeEndpointDef, GraphTypeDef, ProviderError, core_provider::inconsistent};

pub(super) fn apply(
    graph_type: &mut GraphTypeDef,
    name: &DbString,
    source_node_type: Option<&CoreEdgeEndpointDef>,
    target_node_type: Option<&CoreEdgeEndpointDef>,
    properties: &[PropertyDef],
) -> Result<(), ProviderError> {
    let index = graph_type
        .edge_type_index_for(name.clone())
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAlteredV2 references unknown edge type {name}"
            ))
        })? as usize;
    let mut edge_type = graph_type.edge_types[index].clone();

    if let Some(source) = source_node_type {
        let decoded = runtime_endpoint(graph_type, source, "source")?;
        ensure_endpoint_widening("source", name, &edge_type.source_node_type, &decoded)?;
        edge_type.source_node_type = decoded;
    }
    if let Some(target) = target_node_type {
        let decoded = runtime_endpoint(graph_type, target, "target")?;
        ensure_endpoint_widening("target", name, &edge_type.target_node_type, &decoded)?;
        edge_type.target_node_type = decoded;
    }

    for property in properties {
        let decoded = super::runtime_property(property)?;
        if let Some(existing) = edge_type
            .properties
            .iter()
            .find(|candidate| candidate.name == decoded.name)
        {
            if existing == &decoded {
                continue;
            }
            return Err(non_additive(
                name,
                &format!("redefines property {}", decoded.name),
            ));
        }
        if decoded.required {
            return Err(non_additive(
                name,
                &format!("appends required property {}", decoded.name),
            ));
        }
        crate::type_validator::validate_property_default(&decoded).map_err(|error| {
            non_additive(
                name,
                &format!("property {} has invalid default: {error}", decoded.name),
            )
        })?;
        edge_type.properties.push(decoded);
    }

    graph_type.edge_types[index] = edge_type;
    Ok(())
}

pub(super) fn runtime_endpoint(
    graph_type: &GraphTypeDef,
    endpoint: &CoreEdgeEndpointDef,
    role: &str,
) -> Result<EdgeEndpointDef, ProviderError> {
    match endpoint {
        CoreEdgeEndpointDef::Any => Ok(EdgeEndpointDef::Any),
        CoreEdgeEndpointDef::NodeType(node_type) => Ok(EdgeEndpointDef::NodeType(
            resolve_node_type_name(graph_type, node_type, role)?,
        )),
        CoreEdgeEndpointDef::OneOf(node_types) => {
            if node_types.is_empty() {
                return Err(inconsistent(format!(
                    "WAL EdgeTypeAlteredV2 {role} endpoint has an empty node-type set"
                )));
            }
            let indices = node_types
                .iter()
                .map(|node_type| resolve_node_type_name(graph_type, node_type, role))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(EdgeEndpointDef::one_of(indices))
        }
    }
}

fn resolve_node_type_name(
    graph_type: &GraphTypeDef,
    node_type: &selene_core::NodeTypeRef,
    role: &str,
) -> Result<u32, ProviderError> {
    graph_type
        .node_type_index_for(node_type.0.clone())
        .ok_or_else(|| {
            inconsistent(format!(
                "WAL EdgeTypeAlteredV2 references unknown {role} node type {}",
                node_type.0
            ))
        })
}

fn ensure_endpoint_widening(
    role: &str,
    name: &DbString,
    current: &EdgeEndpointDef,
    next: &EdgeEndpointDef,
) -> Result<(), ProviderError> {
    if endpoint_is_subset(current, next) {
        return Ok(());
    }
    Err(non_additive(
        name,
        &format!("{role} endpoint narrows from {current} to {next}"),
    ))
}

fn endpoint_is_subset(current: &EdgeEndpointDef, next: &EdgeEndpointDef) -> bool {
    match (current, next) {
        (_, EdgeEndpointDef::Any) => true,
        (EdgeEndpointDef::Any, _) => matches!(next, EdgeEndpointDef::Any),
        (EdgeEndpointDef::NodeType(current), EdgeEndpointDef::NodeType(next)) => current == next,
        (EdgeEndpointDef::NodeType(current), EdgeEndpointDef::OneOf(next)) => {
            next.binary_search(current).is_ok()
        }
        (EdgeEndpointDef::OneOf(current), EdgeEndpointDef::OneOf(next)) => current
            .iter()
            .all(|current| next.binary_search(current).is_ok()),
        (EdgeEndpointDef::OneOf(_), EdgeEndpointDef::NodeType(_)) => false,
    }
}

fn non_additive(name: &DbString, reason: &str) -> ProviderError {
    inconsistent(format!(
        "WAL EdgeTypeAlteredV2 for {name} is not additive: {reason}"
    ))
}
