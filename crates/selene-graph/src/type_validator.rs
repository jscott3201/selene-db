//! Closed graph type validation.

use std::fmt;

use selene_core::{Change, EdgeId, IStr, LabelSet, NodeId, PropertyMap, PropertyValueType, Value};

use crate::graph::SeleneGraph;
use crate::graph_types::{EdgeEndpointDef, GraphTypeDef, PropertyTypeDef, ValidationMode};

/// Identifier for a typed graph entity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EntityId {
    /// Node entity.
    Node(NodeId),
    /// Edge entity.
    Edge(EdgeId),
}

impl fmt::Display for EntityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(id) => write!(formatter, "node {id}"),
            Self::Edge(id) => write!(formatter, "edge {id}"),
        }
    }
}

/// Closed graph type validation failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error, miette::Diagnostic)]
#[non_exhaustive]
pub enum TypeViolation {
    /// Node labels do not match any node type.
    #[error("node {id} has labels {labels:?}, which do not match any node type")]
    #[diagnostic(code(SLENE_G_030))]
    UnknownNodeLabel {
        /// Node ID.
        id: NodeId,
        /// Observed node label set.
        labels: LabelSet,
    },

    /// Edge label does not match any edge type.
    #[error("edge {id} has label {label}, which does not match any edge type")]
    #[diagnostic(code(SLENE_G_031))]
    UnknownEdgeLabel {
        /// Edge ID.
        id: EdgeId,
        /// Observed edge label.
        label: IStr,
    },

    /// Edge endpoints do not match the declared edge type endpoints.
    #[error(
        "edge {id} label {label} expected endpoint types ({expected_source_type}, {expected_target_type}) but observed ({observed_source_type}, {observed_target_type})"
    )]
    #[diagnostic(code(SLENE_G_032))]
    EdgeEndpointTypeMismatch {
        /// Edge ID.
        id: EdgeId,
        /// Edge label.
        label: IStr,
        /// Expected source endpoint.
        expected_source_type: EdgeEndpointDef,
        /// Observed source node-type index.
        observed_source_type: u32,
        /// Expected target endpoint.
        expected_target_type: EdgeEndpointDef,
        /// Observed target node-type index.
        observed_target_type: u32,
    },

    /// Required property is absent or null.
    #[error("{entity_id} is missing required property {property} declared in {declared_in}")]
    #[diagnostic(code(SLENE_G_033))]
    MissingRequiredProperty {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Missing property name.
        property: IStr,
        /// Node or edge type that declares the property.
        declared_in: IStr,
    },

    /// Property value has the wrong runtime type.
    #[error("{entity_id} property {property} expected {expected} but observed {observed}")]
    #[diagnostic(code(SLENE_G_034))]
    PropertyTypeMismatch {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Property name.
        property: IStr,
        /// Expected property value type.
        expected: PropertyValueType,
        /// Observed runtime value type.
        observed: &'static str,
    },

    /// `Value::Extended` is not a declarable closed-graph type.
    #[error("{entity_id} property {property} uses a Value::Extended payload")]
    #[diagnostic(code(SLENE_G_035))]
    ExtensionValueRejected {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Property name.
        property: IStr,
    },

    /// Property is not declared by the matched node or edge type.
    #[error("{entity_id} property {property} is not declared by the matched type")]
    #[diagnostic(code(SLENE_G_036))]
    UndeclaredProperty {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Undeclared property name.
        property: IStr,
    },

    /// Immutable property was updated or removed.
    #[error("{entity_id} property {property} declared in {declared_in} is immutable")]
    #[diagnostic(code(SLENE_G_037))]
    ImmutablePropertyUpdate {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Immutable property name.
        property: IStr,
        /// Node or edge type that declares the property.
        declared_in: IStr,
    },
}

/// Non-fatal closed graph validation record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeWarning {
    /// Relaxed type-model violation.
    pub violation: TypeViolation,
}

/// Validate a single already-applied change against a graph type.
///
/// `graph` must be the post-change working snapshot. This lets update changes
/// validate required properties and edge endpoint types from the same state
/// that would publish on successful commit.
pub fn validate_change(
    change: &Change,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<Vec<TypeWarning>, TypeViolation> {
    match change {
        Change::NodeCreated { id, .. } => {
            // Skip validation for entities the same transaction has since
            // deleted: aborted-tx-IDs become permanent holes (D11), but a
            // create-then-delete pair has no net effect and should not
            // surface UnknownNodeLabel for a row that no longer exists.
            if !graph.is_node_alive(*id) {
                return Ok(Vec::new());
            }
            validate_node_state(*id, graph, type_def).map(|(_, warnings)| warnings)
        }
        Change::NodeUpdated {
            id,
            properties_diff,
            ..
        } => {
            if !graph.is_node_alive(*id) {
                return Ok(Vec::new());
            }
            let (node_type_index, mut warnings) = validate_node_state(*id, graph, type_def)?;
            let node_type = &type_def.node_types[node_type_index as usize];
            reject_immutable_property_update(
                EntityId::Node(*id),
                node_type.name.clone(),
                &node_type.properties,
                properties_diff,
            )?;
            // A label change can invalidate every incident edge's
            // (label, source_type, target_type) constraint without the
            // edge itself producing a Change. Re-validate every alive
            // incident edge so closed-graph commits cannot publish a
            // graph that violates the edge-type rules.
            warnings.extend(revalidate_incident_edges(*id, graph, type_def)?);
            Ok(warnings)
        }
        Change::EdgeCreated { id, .. } => {
            if !graph.is_edge_alive(*id) {
                return Ok(Vec::new());
            }
            validate_edge_state(*id, graph, type_def).map(|(_, warnings)| warnings)
        }
        Change::EdgeUpdated {
            id,
            properties_diff,
        } => {
            if !graph.is_edge_alive(*id) {
                return Ok(Vec::new());
            }
            let (edge_type, warnings) = validate_edge_state(*id, graph, type_def)?;
            reject_immutable_property_update(
                EntityId::Edge(*id),
                edge_type.name.clone(),
                &edge_type.properties,
                properties_diff,
            )?;
            Ok(warnings)
        }
        Change::NodePropertyRemoved { id, property } => {
            if !graph.is_node_alive(*id) {
                return Ok(Vec::new());
            }
            let (node_type_index, warnings) = validate_node_state(*id, graph, type_def)?;
            let node_type = &type_def.node_types[node_type_index as usize];
            reject_if_immutable(
                EntityId::Node(*id),
                node_type.name.clone(),
                &node_type.properties,
                property.clone(),
            )?;
            Ok(warnings)
        }
        Change::EdgePropertyRemoved { id, property } => {
            if !graph.is_edge_alive(*id) {
                return Ok(Vec::new());
            }
            let (edge_type, warnings) = validate_edge_state(*id, graph, type_def)?;
            reject_if_immutable(
                EntityId::Edge(*id),
                edge_type.name.clone(),
                &edge_type.properties,
                property.clone(),
            )?;
            Ok(warnings)
        }
        Change::NodeLabelRemoved { id, .. } => {
            if !graph.is_node_alive(*id) {
                return Ok(Vec::new());
            }
            let (_, mut warnings) = validate_node_state(*id, graph, type_def)?;
            warnings.extend(revalidate_incident_edges(*id, graph, type_def)?);
            Ok(warnings)
        }
        // Truncation removes INSTANCES and keeps the bound type intact (the
        // node/edge type still exists), and node-truncate cascades incident
        // edges so the graph stays dangling-free — it can never violate GG02,
        // exactly like NodeDeleted/EdgeDeleted. BRIEF-150 / audit Item 11.
        // GraphReset sets bound_type = None in the same txn, so validate_change
        // is never even invoked for it (the commit-time loop is gated on a Some
        // bound_type). The arm is kept for exhaustiveness and is a no-op:
        // wiping the whole graph + dropping the type can never violate GG02.
        Change::NodeDeleted { .. }
        | Change::EdgeDeleted { .. }
        | Change::NodesOfTypeTruncated { .. }
        | Change::EdgesOfTypeTruncated { .. }
        | Change::GraphReset { .. }
        | Change::SchemaChanged { .. } => Ok(Vec::new()),
    }
}

fn revalidate_incident_edges(
    node: NodeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<Vec<TypeWarning>, TypeViolation> {
    let mut warnings = Vec::new();
    if let Some(entry) = graph.outgoing_edges(node) {
        for edge in entry.iter() {
            if graph.is_edge_alive(edge.edge_id) {
                warnings.extend(validate_edge_state(edge.edge_id, graph, type_def)?.1);
            }
        }
    }
    if let Some(entry) = graph.incoming_edges(node) {
        for edge in entry.iter() {
            if graph.is_edge_alive(edge.edge_id) {
                warnings.extend(validate_edge_state(edge.edge_id, graph, type_def)?.1);
            }
        }
    }
    Ok(warnings)
}

/// Validate every alive node and edge in a materialized graph.
pub fn validate_entity_state(
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<Vec<TypeWarning>, TypeViolation> {
    let mut warnings = Vec::new();
    for row in graph.node_store.alive.iter() {
        let id = graph
            .node_id_for_row(crate::store::RowIndex::new(row))
            .expect("alive node row has a mapped external id (BRIEF-Item-4a)");
        warnings.extend(validate_node_state(id, graph, type_def)?.1);
    }
    for row in graph.edge_store.alive.iter() {
        let id = graph
            .edge_id_for_row(crate::store::RowIndex::new(row))
            .expect("alive edge row has a mapped external id (BRIEF-Item-4a)");
        warnings.extend(validate_edge_state(id, graph, type_def)?.1);
    }
    Ok(warnings)
}

fn validate_node_state(
    id: NodeId,
    graph: &SeleneGraph,
    type_def: &GraphTypeDef,
) -> Result<(u32, Vec<TypeWarning>), TypeViolation> {
    // Borrow the live LabelSet/PropertyMap through; only the None (missing-row)
    // path materializes an empty default, and only the error path clones the
    // label set. On schema-changing commits this avoids deep-cloning every alive
    // node's LabelSet + PropertyMap solely to read them.
    let empty_labels = LabelSet::new();
    let labels = graph.node_labels(id).unwrap_or(&empty_labels);
    let node_type_index =
        type_def
            .find_node_type_index(labels)
            .ok_or_else(|| TypeViolation::UnknownNodeLabel {
                id,
                labels: labels.clone(),
            })?;
    let node_type = &type_def.node_types[node_type_index as usize];
    let empty_props = PropertyMap::new();
    let properties = graph.node_properties(id).unwrap_or(&empty_props);
    let warnings = validate_properties(
        EntityId::Node(id),
        node_type.name.clone(),
        node_type.validation_mode,
        &node_type.properties,
        properties,
    )?;
    Ok((node_type_index, warnings))
}

fn validate_edge_state<'a>(
    id: EdgeId,
    graph: &SeleneGraph,
    type_def: &'a GraphTypeDef,
) -> Result<(&'a crate::graph_types::EdgeTypeDef, Vec<TypeWarning>), TypeViolation> {
    let label = graph
        .edge_label(id)
        .cloned()
        .ok_or(TypeViolation::UnknownEdgeLabel {
            id,
            label: selene_core::intern("__selene_missing_edge_label").expect("static label admits"),
        })?;
    let (source, target) =
        graph
            .edge_endpoints(id)
            .ok_or_else(|| TypeViolation::UnknownEdgeLabel {
                id,
                label: label.clone(),
            })?;
    let (source_type, mut warnings) = validate_node_state(source, graph, type_def)?;
    let (target_type, target_warnings) = validate_node_state(target, graph, type_def)?;
    warnings.extend(target_warnings);

    let Some(edge_type) = type_def.find_edge_type(label.clone(), source_type, target_type) else {
        let Some(expected) = type_def.first_edge_type_with_label(label.clone()) else {
            return Err(TypeViolation::UnknownEdgeLabel { id, label });
        };
        return Err(TypeViolation::EdgeEndpointTypeMismatch {
            id,
            label,
            expected_source_type: expected.source_node_type.clone(),
            observed_source_type: source_type,
            expected_target_type: expected.target_node_type.clone(),
            observed_target_type: target_type,
        });
    };
    let empty_props = PropertyMap::new();
    let properties = graph.edge_properties(id).unwrap_or(&empty_props);
    warnings.extend(validate_properties(
        EntityId::Edge(id),
        edge_type.name.clone(),
        edge_type.validation_mode,
        &edge_type.properties,
        properties,
    )?);
    Ok((edge_type, warnings))
}

fn reject_immutable_property_update(
    entity_id: EntityId,
    declared_in: IStr,
    declarations: &[PropertyTypeDef],
    diff: &selene_core::PropertyDiff,
) -> Result<(), TypeViolation> {
    for (key, _) in &diff.set {
        reject_if_immutable(entity_id, declared_in.clone(), declarations, key.clone())?;
    }
    for key in &diff.removed {
        reject_if_immutable(entity_id, declared_in.clone(), declarations, key.clone())?;
    }
    Ok(())
}

fn reject_if_immutable(
    entity_id: EntityId,
    declared_in: IStr,
    declarations: &[PropertyTypeDef],
    property: IStr,
) -> Result<(), TypeViolation> {
    if declarations
        .iter()
        .any(|declaration| declaration.name == property && declaration.immutable)
    {
        return Err(TypeViolation::ImmutablePropertyUpdate {
            entity_id,
            property,
            declared_in,
        });
    }
    Ok(())
}

fn validate_properties(
    entity_id: EntityId,
    declared_in: IStr,
    validation_mode: ValidationMode,
    declarations: &[PropertyTypeDef],
    properties: &PropertyMap,
) -> Result<Vec<TypeWarning>, TypeViolation> {
    let mut warnings = Vec::new();
    for (key, value) in properties.iter() {
        let Some(declaration) = declarations.iter().find(|decl| decl.name == *key) else {
            let violation = TypeViolation::UndeclaredProperty {
                entity_id,
                property: key.clone(),
            };
            if validation_mode == ValidationMode::Warn {
                warnings.push(TypeWarning { violation });
                continue;
            }
            return Err(violation);
        };
        if matches!(value, Value::Extended { .. }) {
            return Err(TypeViolation::ExtensionValueRejected {
                entity_id,
                property: key.clone(),
            });
        }
        if matches!(value, Value::Null) {
            if declaration.required {
                return Err(TypeViolation::MissingRequiredProperty {
                    entity_id,
                    property: key.clone(),
                    declared_in: declared_in.clone(),
                });
            }
            continue;
        }
        if !property_value_matches(declaration, value) {
            return Err(TypeViolation::PropertyTypeMismatch {
                entity_id,
                property: key.clone(),
                expected: declaration.value_type,
                observed: PropertyValueType::observed_name(value),
            });
        }
    }

    for declaration in declarations.iter().filter(|decl| decl.required) {
        if properties
            .get(&declaration.name)
            .is_none_or(|value| matches!(value, Value::Null))
        {
            return Err(TypeViolation::MissingRequiredProperty {
                entity_id,
                property: declaration.name.clone(),
                declared_in: declared_in.clone(),
            });
        }
    }
    Ok(warnings)
}

fn property_value_matches(declaration: &PropertyTypeDef, value: &Value) -> bool {
    match declaration.value_type {
        PropertyValueType::List => {
            let Some(element_type) = declaration.list_element_type.as_ref() else {
                return matches!(value, Value::List(_));
            };
            match value {
                Value::List(values) => values.iter().all(|value| element_type.matches(value)),
                _ => false,
            }
        }
        // A RECORD-typed property accepts either record value form — the open
        // `Value::Record` (the `RECORD{...}` constructor / by-name form) or the positional
        // `Value::RecordTyped` — because the constructor always yields the open form
        // regardless of the declared type. Structural conformance against a closed
        // descriptor (or permissive acceptance for an open/bare `None` descriptor) is then
        // decided by [`RecordFieldTypes::matches`].
        // Why: closed/typed RECORD conformance per ISO 39075:2024 §4.15.4 (a closed record
        // value must have the same field-name set as the descriptor and each field must
        // match) → graph type violation G2000 (§4.13.2.1).
        PropertyValueType::Record | PropertyValueType::RecordTyped => {
            if !matches!(value, Value::Record(_) | Value::RecordTyped(_)) {
                return false;
            }
            match declaration.record_field_types.as_ref() {
                Some(fields) => fields.matches(value),
                None => true,
            }
        }
        _ => declaration.value_type.matches(value),
    }
}

#[cfg(test)]
#[path = "type_validator_tests.rs"]
mod tests;
