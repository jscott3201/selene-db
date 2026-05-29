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
    #[diagnostic(code(SLENE_G_010))]
    UnknownNodeLabel {
        /// Node ID.
        id: NodeId,
        /// Observed node label set.
        labels: LabelSet,
    },

    /// Edge label does not match any edge type.
    #[error("edge {id} has label {label}, which does not match any edge type")]
    #[diagnostic(code(SLENE_G_011))]
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
    #[diagnostic(code(SLENE_G_012))]
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
    #[diagnostic(code(SLENE_G_013))]
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
    #[diagnostic(code(SLENE_G_014))]
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

    /// Extension-owned values are not declarable in v1.0 graph types.
    #[error("{entity_id} property {property} uses an extension-owned value")]
    #[diagnostic(code(SLENE_G_015))]
    ExtensionValueRejected {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Property name.
        property: IStr,
    },

    /// Property is not declared by the matched node or edge type.
    #[error("{entity_id} property {property} is not declared by the matched type")]
    #[diagnostic(code(SLENE_G_016))]
    UndeclaredProperty {
        /// Entity that violated the declaration.
        entity_id: EntityId,
        /// Undeclared property name.
        property: IStr,
    },

    /// Immutable property was updated or removed.
    #[error("{entity_id} property {property} declared in {declared_in} is immutable")]
    #[diagnostic(code(SLENE_G_017))]
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
                node_type.name,
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
                edge_type.name,
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
                node_type.name,
                &node_type.properties,
                *property,
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
                edge_type.name,
                &edge_type.properties,
                *property,
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
        | Change::SchemaChanged { .. }
        | Change::IndexExtensionEvent { .. } => Ok(Vec::new()),
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
    let labels = graph.node_labels(id).cloned().unwrap_or_else(LabelSet::new);
    let node_type_index =
        type_def
            .find_node_type_index(&labels)
            .ok_or_else(|| TypeViolation::UnknownNodeLabel {
                id,
                labels: labels.clone(),
            })?;
    let node_type = &type_def.node_types[node_type_index as usize];
    let properties = graph
        .node_properties(id)
        .cloned()
        .unwrap_or_else(PropertyMap::new);
    let warnings = validate_properties(
        EntityId::Node(id),
        node_type.name,
        node_type.validation_mode,
        &node_type.properties,
        &properties,
    )?;
    Ok((node_type_index, warnings))
}

fn validate_edge_state<'a>(
    id: EdgeId,
    graph: &SeleneGraph,
    type_def: &'a GraphTypeDef,
) -> Result<(&'a crate::graph_types::EdgeTypeDef, Vec<TypeWarning>), TypeViolation> {
    let label = *graph
        .edge_label(id)
        .ok_or(TypeViolation::UnknownEdgeLabel {
            id,
            label: selene_core::intern("__selene_missing_edge_label").expect("static label admits"),
        })?;
    let (source, target) = graph
        .edge_endpoints(id)
        .ok_or(TypeViolation::UnknownEdgeLabel { id, label })?;
    let (source_type, mut warnings) = validate_node_state(source, graph, type_def)?;
    let (target_type, target_warnings) = validate_node_state(target, graph, type_def)?;
    warnings.extend(target_warnings);

    let Some(edge_type) = type_def.find_edge_type(label, source_type, target_type) else {
        let Some(expected) = type_def.first_edge_type_with_label(label) else {
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
    let properties = graph
        .edge_properties(id)
        .cloned()
        .unwrap_or_else(PropertyMap::new);
    warnings.extend(validate_properties(
        EntityId::Edge(id),
        edge_type.name,
        edge_type.validation_mode,
        &edge_type.properties,
        &properties,
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
        reject_if_immutable(entity_id, declared_in, declarations, *key)?;
    }
    for key in &diff.removed {
        reject_if_immutable(entity_id, declared_in, declarations, *key)?;
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
                property: *key,
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
                property: *key,
            });
        }
        if matches!(value, Value::Null) {
            if declaration.required {
                return Err(TypeViolation::MissingRequiredProperty {
                    entity_id,
                    property: *key,
                    declared_in,
                });
            }
            continue;
        }
        if !property_value_matches(declaration, value) {
            return Err(TypeViolation::PropertyTypeMismatch {
                entity_id,
                property: *key,
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
                property: declaration.name,
                declared_in,
            });
        }
    }
    Ok(warnings)
}

fn property_value_matches(declaration: &PropertyTypeDef, value: &Value) -> bool {
    if !declaration.value_type.matches(value) {
        return false;
    }
    if declaration.value_type != PropertyValueType::List {
        return true;
    }
    let Some(element_type) = declaration.list_element_type.as_ref() else {
        return true;
    };
    match value {
        Value::List(values) => values.iter().all(|value| element_type.matches(value)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use selene_core::{ExtensionTypeId, GraphId, intern};

    use super::*;
    use crate::{GraphError, SharedGraph};

    fn istr(name: &str) -> IStr {
        intern(name).unwrap()
    }

    fn prop(name: &str, value: Value) -> PropertyMap {
        PropertyMap::from_pairs([(istr(name), value)]).unwrap()
    }

    fn graph_type() -> GraphTypeDef {
        GraphTypeDef {
            name: istr("validator.graph"),
            node_types: vec![
                crate::NodeTypeDef {
                    name: istr("validator.person"),
                    key_labels: LabelSet::single(istr("Person")),
                    properties: vec![PropertyTypeDef {
                        name: istr("name"),
                        value_type: PropertyValueType::String,
                        list_element_type: None,
                        required: true,
                        default: None,
                        immutable: false,
                    }],
                    validation_mode: ValidationMode::Strict,
                },
                crate::NodeTypeDef {
                    name: istr("validator.company"),
                    key_labels: LabelSet::single(istr("Company")),
                    properties: vec![PropertyTypeDef {
                        name: istr("name"),
                        value_type: PropertyValueType::String,
                        list_element_type: None,
                        required: true,
                        default: None,
                        immutable: false,
                    }],
                    validation_mode: ValidationMode::Strict,
                },
            ],
            edge_types: vec![crate::EdgeTypeDef {
                name: istr("validator.works_at"),
                label: istr("WORKS_AT"),
                source_node_type: EdgeEndpointDef::NodeType(0),
                target_node_type: EdgeEndpointDef::NodeType(1),
                properties: vec![PropertyTypeDef {
                    name: istr("since"),
                    value_type: PropertyValueType::Int,
                    list_element_type: None,
                    required: false,
                    default: None,
                    immutable: false,
                }],
                validation_mode: ValidationMode::Strict,
            }],
        }
    }

    fn valid_graph() -> SeleneGraph {
        let shared = SharedGraph::builder(GraphId::new(1)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let person = mutator
                .create_node(
                    LabelSet::single(istr("Person")),
                    prop("name", Value::String(istr("Alice"))),
                )
                .unwrap();
            let company = mutator
                .create_node(
                    LabelSet::single(istr("Company")),
                    prop("name", Value::String(istr("Acme"))),
                )
                .unwrap();
            mutator
                .create_edge(
                    istr("WORKS_AT"),
                    person,
                    company,
                    prop("since", Value::Int(2026)),
                )
                .unwrap();
        }
        txn.commit().unwrap();
        shared.read().as_ref().clone()
    }

    #[test]
    fn validate_entity_state_accepts_valid_graph() {
        validate_entity_state(&valid_graph(), &graph_type()).unwrap();
    }

    #[test]
    fn validate_change_accepts_applied_node_created() {
        let graph = valid_graph();
        validate_change(
            &Change::NodeCreated {
                id: NodeId::new(1),
                labels: LabelSet::single(istr("Person")),
                properties: prop("name", Value::String(istr("Alice"))),
            },
            &graph,
            &graph_type(),
        )
        .unwrap();
    }

    #[test]
    fn rejects_unknown_node_label() {
        let shared = SharedGraph::builder(GraphId::new(2)).build().unwrap();
        let mut txn = shared.begin_write();
        let node = {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::single(istr("Project")), PropertyMap::new())
                .unwrap()
        };
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::UnknownNodeLabel { id, .. }) if id == node
        ));
    }

    #[test]
    fn rejects_unknown_edge_label() {
        let mut graph = valid_graph();
        graph.edge_store.label.set(0, istr("KNOWS"));
        assert!(matches!(
            validate_entity_state(&graph, &graph_type()),
            Err(TypeViolation::UnknownEdgeLabel { id, label })
                if id == EdgeId::new(1) && label == istr("KNOWS")
        ));
    }

    #[test]
    fn rejects_edge_endpoint_mismatch() {
        let shared = SharedGraph::builder(GraphId::new(3)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let a = mutator
                .create_node(
                    LabelSet::single(istr("Company")),
                    prop("name", Value::String(istr("A"))),
                )
                .unwrap();
            let b = mutator
                .create_node(
                    LabelSet::single(istr("Person")),
                    prop("name", Value::String(istr("B"))),
                )
                .unwrap();
            mutator
                .create_edge(istr("WORKS_AT"), a, b, PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::EdgeEndpointTypeMismatch {
                observed_source_type: 1,
                observed_target_type: 0,
                ..
            })
        ));
    }

    #[test]
    fn rejects_missing_required_property() {
        let shared = SharedGraph::builder(GraphId::new(4)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(LabelSet::single(istr("Person")), PropertyMap::new())
                .unwrap();
        }
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::MissingRequiredProperty { property, .. }) if property == istr("name")
        ));
    }

    #[test]
    fn rejects_property_type_mismatch() {
        let shared = SharedGraph::builder(GraphId::new(5)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(istr("Person")),
                    prop("name", Value::Int(7)),
                )
                .unwrap();
        }
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::PropertyTypeMismatch {
                expected: PropertyValueType::String,
                observed: "Int",
                ..
            })
        ));
    }

    #[test]
    fn legacy_untyped_list_declaration_accepts_any_list_elements() {
        let declaration = PropertyTypeDef {
            name: istr("legacy"),
            value_type: PropertyValueType::List,
            list_element_type: None,
            required: false,
            default: None,
            immutable: false,
        };

        assert!(property_value_matches(
            &declaration,
            &Value::List(vec![Value::Int(1), Value::String(istr("two"))])
        ));
        assert!(!property_value_matches(&declaration, &Value::Int(1)));
    }

    #[test]
    fn rejects_extension_value() {
        let shared = SharedGraph::builder(GraphId::new(6)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            mutator
                .create_node(
                    LabelSet::single(istr("Person")),
                    prop(
                        "name",
                        Value::Extended {
                            type_id: ExtensionTypeId(0x100),
                            payload: Arc::from([1_u8]),
                        },
                    ),
                )
                .unwrap();
        }
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::ExtensionValueRejected { property, .. }) if property == istr("name")
        ));
    }

    #[test]
    fn rejects_undeclared_property() {
        let shared = SharedGraph::builder(GraphId::new(7)).build().unwrap();
        let mut txn = shared.begin_write();
        {
            let mut mutator = txn.mutator();
            let mut props = prop("name", Value::String(istr("Alice")));
            props.set(istr("extra"), Value::Bool(true)).unwrap();
            mutator
                .create_node(LabelSet::single(istr("Person")), props)
                .unwrap();
        }
        txn.commit().unwrap();
        assert!(matches!(
            validate_entity_state(shared.read().as_ref(), &graph_type()),
            Err(TypeViolation::UndeclaredProperty { property, .. }) if property == istr("extra")
        ));
    }

    #[test]
    fn graph_error_wraps_type_violation() {
        let error = GraphError::from(TypeViolation::UnknownEdgeLabel {
            id: EdgeId::new(1),
            label: istr("BAD"),
        });
        assert_eq!(error.gqlstatus(), "G2000");
    }
}
