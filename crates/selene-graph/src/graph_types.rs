//! Closed graph type catalog definitions.

use std::collections::BTreeSet;

use selene_core::{IStr, LabelSet, PropertyValueType, Value};
use serde::{Deserialize, Serialize};

use crate::error::{GraphError, GraphResult};

/// Maximum supported nesting for catalog `LIST<T>` property element descriptors.
pub const MAX_LIST_TYPE_NESTING: u32 = 64;

/// Definition of a closed graph type per ISO clause 18.
#[derive(
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct GraphTypeDef {
    /// Graph type name.
    pub name: IStr,
    /// Node-type elements in graph-type order.
    pub node_types: Vec<NodeTypeDef>,
    /// Edge-type elements in graph-type order.
    pub edge_types: Vec<EdgeTypeDef>,
}

impl GraphTypeDef {
    /// Validate this graph type's structural invariants.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the type contains duplicate
    /// names, invalid edge endpoint indexes, duplicate properties within a
    /// node/edge type, duplicate edge triples, or an empty node label set.
    pub fn validate(self) -> GraphResult<Self> {
        self.validate_ref()?;
        Ok(self)
    }

    /// Return the first node type matching `labels`.
    #[must_use]
    pub fn find_node_type(&self, labels: &LabelSet) -> Option<&NodeTypeDef> {
        self.node_types
            .iter()
            .find(|node_type| &node_type.key_labels == labels)
    }

    /// Return the first node-type index matching `labels`.
    #[must_use]
    pub fn find_node_type_index(&self, labels: &LabelSet) -> Option<u32> {
        self.node_types
            .iter()
            .position(|node_type| &node_type.key_labels == labels)
            .and_then(|index| u32::try_from(index).ok())
    }

    /// Return the node-type index matching `name`.
    #[must_use]
    pub fn node_type_index_for(&self, name: IStr) -> Option<u32> {
        self.node_types
            .iter()
            .position(|node_type| node_type.name == name)
            .and_then(|index| u32::try_from(index).ok())
    }

    /// Return the edge type for `(label, source_node_type, target_node_type)`.
    #[must_use]
    pub fn find_edge_type(
        &self,
        label: IStr,
        source_node_type: u32,
        target_node_type: u32,
    ) -> Option<&EdgeTypeDef> {
        self.edge_types.iter().find(|edge_type| {
            edge_type.label == label
                && edge_type.source_node_type == source_node_type
                && edge_type.target_node_type == target_node_type
        })
    }

    /// Return the first edge type carrying `label`.
    #[must_use]
    pub fn first_edge_type_with_label(&self, label: IStr) -> Option<&EdgeTypeDef> {
        self.edge_types
            .iter()
            .find(|edge_type| edge_type.label == label)
    }

    /// Return a copy with the named node type removed.
    ///
    /// Edge endpoint indexes are intentionally not rewritten. Callers that
    /// cannot tolerate positional drift must reject the drop before using this
    /// helper.
    #[must_use]
    pub fn without_node_type(&self, name: IStr) -> Option<Self> {
        let index = self
            .node_types
            .iter()
            .position(|node_type| node_type.name == name)?;
        let mut next = self.clone();
        next.node_types.remove(index);
        Some(next)
    }

    /// Return a copy with the named edge type removed.
    #[must_use]
    pub fn without_edge_type(&self, name: IStr) -> Option<Self> {
        let index = self
            .edge_types
            .iter()
            .position(|edge_type| edge_type.name == name)?;
        let mut next = self.clone();
        next.edge_types.remove(index);
        Some(next)
    }

    /// Validate the type without consuming it.
    ///
    /// Same checks as [`GraphTypeDef::validate`]; preferred when callers
    /// already hold a reference (recovery, [`crate::SharedGraph::from_graph`]
    /// re-validation) and cannot move the value.
    pub fn validate_ref(&self) -> GraphResult<()> {
        ensure_unique_names(
            "node type",
            self.node_types.iter().map(|node_type| node_type.name),
        )?;
        ensure_unique_names(
            "edge type",
            self.edge_types.iter().map(|edge_type| edge_type.name),
        )?;

        let mut seen_label_sets = BTreeSet::new();
        for node_type in &self.node_types {
            if node_type.key_labels.is_empty() {
                return Err(GraphError::Inconsistent {
                    reason: format!("node type {} has an empty label set", node_type.name),
                });
            }
            // Why: find_node_type_index uses first-match semantics, so two
            // node types with identical key_labels would leave the second
            // unreachable AND cause edge / property validation to dispatch
            // against the wrong type. Reject ambiguity at type-construction
            // time rather than letting it manifest as silent mis-typing.
            let label_key: Vec<IStr> = node_type.key_labels.iter().copied().collect();
            if !seen_label_sets.insert(label_key) {
                return Err(GraphError::Inconsistent {
                    reason: format!(
                        "node type {} duplicates the key_labels of an earlier node type",
                        node_type.name
                    ),
                });
            }
            ensure_unique_names(
                "node property",
                node_type.properties.iter().map(|property| property.name),
            )?;
            validate_property_element_types(node_type.name, &node_type.properties)?;
        }

        let node_type_count = self.node_types.len();
        let mut edge_triples = BTreeSet::new();
        for edge_type in &self.edge_types {
            ensure_node_type_index(node_type_count, edge_type.source_node_type, edge_type.name)?;
            ensure_node_type_index(node_type_count, edge_type.target_node_type, edge_type.name)?;
            ensure_unique_names(
                "edge property",
                edge_type.properties.iter().map(|property| property.name),
            )?;
            validate_property_element_types(edge_type.name, &edge_type.properties)?;
            if !edge_triples.insert((
                edge_type.label,
                edge_type.source_node_type,
                edge_type.target_node_type,
            )) {
                return Err(GraphError::Inconsistent {
                    reason: format!(
                        "duplicate edge type triple ({}, {}, {})",
                        edge_type.label, edge_type.source_node_type, edge_type.target_node_type
                    ),
                });
            }
        }
        Ok(())
    }
}

fn validate_property_element_types(
    type_name: IStr,
    properties: &[PropertyTypeDef],
) -> GraphResult<()> {
    for property in properties {
        if property.value_type == PropertyValueType::List {
            let Some(element_type) = property.list_element_type.as_ref() else {
                // Legacy snapshots written before typed LIST<T> descriptors
                // stored only the coarse LIST tag. Keep that shape valid so
                // recovery preserves existing closed graph schemas; new GQL
                // catalog DDL always fills the descriptor.
                continue;
            };
            validate_property_element_type(type_name, property.name, element_type, 1)?;
        } else if property.list_element_type.is_some() {
            return Err(GraphError::Inconsistent {
                reason: format!(
                    "property {} on type {type_name} declares a list element type for non-LIST value type {}",
                    property.name, property.value_type
                ),
            });
        }
    }
    Ok(())
}

fn validate_property_element_type(
    type_name: IStr,
    property_name: IStr,
    element_type: &PropertyElementType,
    depth: u32,
) -> GraphResult<()> {
    if depth > MAX_LIST_TYPE_NESTING {
        return Err(GraphError::Inconsistent {
            reason: format!(
                "property {property_name} on type {type_name} exceeds LIST nesting limit"
            ),
        });
    }
    match element_type {
        PropertyElementType::Scalar(
            PropertyValueType::List | PropertyValueType::Record | PropertyValueType::RecordTyped,
        ) => Err(GraphError::Inconsistent {
            reason: format!(
                "property {property_name} on type {type_name} uses unsupported LIST element type {}",
                element_type.value_type()
            ),
        }),
        PropertyElementType::Scalar(_) => Ok(()),
        PropertyElementType::List(inner) => {
            validate_property_element_type(type_name, property_name, inner, depth + 1)
        }
    }
}

/// Node-type element.
#[derive(
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct NodeTypeDef {
    /// Node type name.
    pub name: IStr,
    /// Defining label set for this node type.
    pub key_labels: LabelSet,
    /// Declared properties.
    pub properties: Vec<PropertyTypeDef>,
    /// Validation mode for undeclared-property writes.
    pub validation_mode: ValidationMode,
}

/// Edge-type element.
#[derive(
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct EdgeTypeDef {
    /// Edge type name.
    pub name: IStr,
    /// Edge label.
    pub label: IStr,
    /// Index into [`GraphTypeDef::node_types`] for the source endpoint.
    pub source_node_type: u32,
    /// Index into [`GraphTypeDef::node_types`] for the target endpoint.
    pub target_node_type: u32,
    /// Declared properties.
    pub properties: Vec<PropertyTypeDef>,
    /// Validation mode for undeclared-property writes.
    pub validation_mode: ValidationMode,
}

/// Property declaration for a closed graph type.
#[derive(
    Clone,
    Debug,
    Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub struct PropertyTypeDef {
    /// Property name.
    pub name: IStr,
    /// Declared value type.
    pub value_type: PropertyValueType,
    /// Declared element type when [`PropertyTypeDef::value_type`] is `List`.
    pub list_element_type: Option<PropertyElementType>,
    /// `true` means NOT NULL / required.
    pub required: bool,
    /// Default value materialized when the property is omitted on create.
    pub default: Option<PropertyDefaultValue>,
    /// Whether updates to this property are forbidden after creation.
    pub immutable: bool,
}

/// Persistable element-type descriptor for `LIST<T>` property declarations.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
#[rkyv(
    bytecheck(bounds(__C: rkyv::validation::ArchiveContext)),
    deserialize_bounds(__D::Error: rkyv::rancor::Source),
    serialize_bounds(__S: rkyv::ser::Writer)
)]
#[non_exhaustive]
pub enum PropertyElementType {
    /// Scalar list element type.
    Scalar(PropertyValueType),
    /// Nested list element type.
    List(#[rkyv(omit_bounds)] Box<PropertyElementType>),
}

impl PropertyElementType {
    /// Return the coarse property-value type for this descriptor.
    #[must_use]
    pub const fn value_type(&self) -> PropertyValueType {
        match self {
            Self::Scalar(value_type) => *value_type,
            Self::List(_) => PropertyValueType::List,
        }
    }

    /// Return true when `value` belongs to this element type.
    #[must_use]
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Self::Scalar(value_type) => value_type.matches(value),
            Self::List(element_type) => match value {
                Value::List(values) => values.iter().all(|value| element_type.matches(value)),
                _ => false,
            },
        }
    }
}

/// Persistable default-value descriptor for closed graph property declarations.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
#[non_exhaustive]
pub enum PropertyDefaultValue {
    /// Null default.
    Null,
    /// Boolean default.
    Boolean(bool),
    /// Signed integer default.
    Integer(i64),
    /// Interned string default.
    String(IStr),
}

impl PropertyDefaultValue {
    /// Materialize this descriptor as a runtime value.
    #[must_use]
    pub const fn to_value(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Boolean(value) => Value::Bool(*value),
            Self::Integer(value) => Value::Int(*value),
            Self::String(value) => Value::String(*value),
        }
    }

    /// Convert a runtime value into a persistable default descriptor.
    #[must_use]
    pub const fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Null => Some(Self::Null),
            Value::Bool(value) => Some(Self::Boolean(*value)),
            Value::Int(value) => Some(Self::Integer(*value)),
            Value::String(value) => Some(Self::String(*value)),
            _ => None,
        }
    }
}

/// Closed-graph validation mode.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    Hash,
    PartialEq,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
    Serialize,
)]
pub enum ValidationMode {
    /// Reject undeclared-property writes.
    #[default]
    Strict,
    /// Allow undeclared-property writes and record a warning.
    Warn,
}

fn ensure_unique_names(kind: &'static str, names: impl Iterator<Item = IStr>) -> GraphResult<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(GraphError::Inconsistent {
                reason: format!("duplicate {kind} name {name}"),
            });
        }
    }
    Ok(())
}

fn ensure_node_type_index(count: usize, index: u32, edge_name: IStr) -> GraphResult<()> {
    if usize::try_from(index).is_ok_and(|index| index < count) {
        return Ok(());
    }
    Err(GraphError::Inconsistent {
        reason: format!(
            "edge type {edge_name} references node type index {index}, but only {count} node types exist"
        ),
    })
}

#[cfg(test)]
mod tests {
    use selene_core::{PropertyValueType, intern};

    use super::*;

    fn label(name: &str) -> IStr {
        intern(name).unwrap()
    }

    fn property(name: &str) -> PropertyTypeDef {
        PropertyTypeDef {
            name: label(name),
            value_type: PropertyValueType::String,
            list_element_type: None,
            required: true,
            default: None,
            immutable: false,
        }
    }

    fn valid_type() -> GraphTypeDef {
        GraphTypeDef {
            name: label("types.graph"),
            node_types: vec![
                NodeTypeDef {
                    name: label("types.person"),
                    key_labels: LabelSet::single(label("Person")),
                    properties: vec![property("name")],
                    validation_mode: ValidationMode::Strict,
                },
                NodeTypeDef {
                    name: label("types.company"),
                    key_labels: LabelSet::single(label("Company")),
                    properties: vec![property("name")],
                    validation_mode: ValidationMode::Strict,
                },
            ],
            edge_types: vec![EdgeTypeDef {
                name: label("types.works_at"),
                label: label("WORKS_AT"),
                source_node_type: 0,
                target_node_type: 1,
                properties: vec![property("since")],
                validation_mode: ValidationMode::Strict,
            }],
        }
    }

    #[test]
    fn validate_accepts_well_formed_type() {
        assert!(valid_type().validate().is_ok());
    }

    #[test]
    fn validate_rejects_duplicate_node_type_names() {
        let mut graph_type = valid_type();
        graph_type.node_types[1].name = graph_type.node_types[0].name;
        assert!(matches!(
            graph_type.validate(),
            Err(GraphError::Inconsistent { reason }) if reason.contains("duplicate node type name")
        ));
    }

    #[test]
    fn validate_rejects_edge_index_out_of_range() {
        let mut graph_type = valid_type();
        graph_type.edge_types[0].target_node_type = 99;
        assert!(matches!(
            graph_type.validate(),
            Err(GraphError::Inconsistent { reason }) if reason.contains("references node type index")
        ));
    }

    #[test]
    fn validate_rejects_duplicate_property_names() {
        let mut graph_type = valid_type();
        graph_type.node_types[0].properties.push(property("name"));
        assert!(matches!(
            graph_type.validate(),
            Err(GraphError::Inconsistent { reason }) if reason.contains("duplicate node property name")
        ));
    }

    #[test]
    fn validate_rejects_empty_node_label_set() {
        let mut graph_type = valid_type();
        graph_type.node_types[0].key_labels = LabelSet::new();
        assert!(matches!(
            graph_type.validate(),
            Err(GraphError::Inconsistent { reason }) if reason.contains("empty label set")
        ));
    }

    #[test]
    fn validate_rejects_duplicate_node_key_label_sets() {
        // Two node types with identical key_labels would leave the second
        // unreachable via find_node_type_index (first-match wins); rejecting
        // at construction prevents silent mis-typing of edges and properties.
        let mut graph_type = valid_type();
        graph_type.node_types[1].key_labels = graph_type.node_types[0].key_labels.clone();
        assert!(matches!(
            graph_type.validate(),
            Err(GraphError::Inconsistent { reason })
                if reason.contains("duplicates the key_labels")
        ));
    }

    #[test]
    fn lookup_returns_matching_elements() {
        let graph_type = valid_type();
        let person = LabelSet::single(label("Person"));
        assert_eq!(
            graph_type
                .find_node_type(&person)
                .map(|node_type| node_type.name),
            Some(label("types.person"))
        );
        assert_eq!(graph_type.find_node_type_index(&person), Some(0));
        assert_eq!(
            graph_type.node_type_index_for(label("types.company")),
            Some(1)
        );
        assert_eq!(
            graph_type
                .find_edge_type(label("WORKS_AT"), 0, 1)
                .map(|edge_type| edge_type.name),
            Some(label("types.works_at"))
        );
    }

    #[test]
    fn without_helpers_remove_named_type_without_reindexing() {
        let graph_type = valid_type();

        let without_node = graph_type
            .without_node_type(label("types.person"))
            .expect("node type removed");
        assert_eq!(without_node.node_types.len(), 1);
        assert_eq!(without_node.edge_types[0].source_node_type, 0);
        assert_eq!(without_node.edge_types[0].target_node_type, 1);

        let without_edge = graph_type
            .without_edge_type(label("types.works_at"))
            .expect("edge type removed");
        assert!(without_edge.edge_types.is_empty());
        assert!(graph_type.without_node_type(label("missing")).is_none());
        assert!(graph_type.without_edge_type(label("missing")).is_none());
    }

    #[test]
    fn rkyv_round_trips_graph_type_def() {
        let graph_type = valid_type();
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&graph_type).unwrap();
        let decoded = rkyv::from_bytes::<GraphTypeDef, rkyv::rancor::Error>(&bytes).unwrap();
        assert_eq!(decoded, graph_type);
    }
}
