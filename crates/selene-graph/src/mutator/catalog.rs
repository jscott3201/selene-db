//! Catalog mutation methods for the transaction mutator.

use std::sync::Arc;

use selene_core::{
    Change, GraphTypeId, IStr, LabelSet, NodeTypeRef, PredefinedValueType, PropertyDef,
    PropertyValueType, SchemaChange, ValueType,
};
use smallvec::SmallVec;

use crate::{
    EdgeTypeDef, GraphError, GraphResult, GraphTypeDef, Mutator, NodeTypeDef, PropertyTypeDef,
};

const OPEN_GRAPH_CATALOG_DDL: &str =
    "open graph (GG01) does not support catalog type DDL -- use a closed graph (GG02)";

impl<'tx, 'g> Mutator<'tx, 'g> {
    /// Add a node type to the transaction-local closed graph type.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the graph is open, the type
    /// already exists, or the resulting graph type is structurally invalid.
    pub fn create_node_type(
        &mut self,
        name: IStr,
        key_labels: LabelSet,
        properties: Vec<PropertyTypeDef>,
    ) -> GraphResult<()> {
        let mut graph_type = self.current_graph_type()?;
        if graph_type
            .node_types
            .iter()
            .any(|node_type| node_type.name == name)
        {
            return Err(GraphError::Inconsistent {
                reason: format!("node type {name} already exists"),
            });
        }
        let node_type = NodeTypeDef {
            name,
            key_labels,
            properties,
        };
        graph_type.node_types.push(node_type.clone());
        graph_type.validate_ref()?;
        self.txn.working.meta.bound_type = Some(Arc::new(graph_type));
        self.txn.changes.push(Change::SchemaChanged {
            graph: self.txn.working.graph_id(),
            change: SchemaChange::NodeTypeAdded {
                graph_type: implicit_graph_type_id(),
                label: name,
                def: core_node_type_def(&node_type),
            },
        });
        Ok(())
    }

    /// Add an edge type to the transaction-local closed graph type.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the graph is open, the type
    /// already exists, an endpoint index is invalid, or the resulting graph
    /// type is structurally invalid.
    pub fn create_edge_type(
        &mut self,
        name: IStr,
        label: IStr,
        source_node_type: u32,
        target_node_type: u32,
        properties: Vec<PropertyTypeDef>,
    ) -> GraphResult<()> {
        let mut graph_type = self.current_graph_type()?;
        if graph_type
            .edge_types
            .iter()
            .any(|edge_type| edge_type.name == name)
        {
            return Err(GraphError::Inconsistent {
                reason: format!("edge type {name} already exists"),
            });
        }
        let edge_type = EdgeTypeDef {
            name,
            label,
            source_node_type,
            target_node_type,
            properties,
        };
        graph_type.edge_types.push(edge_type.clone());
        graph_type.validate_ref()?;
        self.txn.working.meta.bound_type = Some(Arc::new(graph_type.clone()));
        self.txn.changes.push(Change::SchemaChanged {
            graph: self.txn.working.graph_id(),
            change: SchemaChange::EdgeTypeAdded {
                graph_type: implicit_graph_type_id(),
                label,
                def: core_edge_type_def(&graph_type, &edge_type)?,
            },
        });
        Ok(())
    }

    /// Drop a node type from the transaction-local closed graph type.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the graph is open, the type
    /// does not exist, or any edge endpoint would reference the dropped type or
    /// require positional endpoint reindexing.
    pub fn drop_node_type(&mut self, name: IStr) -> GraphResult<()> {
        let graph_type = self.current_graph_type()?;
        let removed_index =
            graph_type
                .node_type_index_for(name)
                .ok_or_else(|| GraphError::Inconsistent {
                    reason: format!("node type {name} does not exist"),
                })?;
        for edge_type in &graph_type.edge_types {
            if edge_type.source_node_type >= removed_index
                || edge_type.target_node_type >= removed_index
            {
                return Err(GraphError::Inconsistent {
                    reason: format!(
                        "cannot drop node type {name}: edge type {} still depends on node-type indexes that would require reindexing",
                        edge_type.name
                    ),
                });
            }
        }
        let next = graph_type
            .without_node_type(name)
            .expect("node type existed above");
        next.validate_ref()?;
        self.txn.working.meta.bound_type = Some(Arc::new(next));
        self.txn.changes.push(Change::SchemaChanged {
            graph: self.txn.working.graph_id(),
            change: SchemaChange::NodeTypeDropped {
                graph_type: implicit_graph_type_id(),
                name,
            },
        });
        Ok(())
    }

    /// Drop an edge type from the transaction-local closed graph type.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Inconsistent`] when the graph is open, the type
    /// does not exist, or the resulting graph type is structurally invalid.
    pub fn drop_edge_type(&mut self, name: IStr) -> GraphResult<()> {
        let graph_type = self.current_graph_type()?;
        let next = graph_type
            .without_edge_type(name)
            .ok_or_else(|| GraphError::Inconsistent {
                reason: format!("edge type {name} does not exist"),
            })?;
        next.validate_ref()?;
        self.txn.working.meta.bound_type = Some(Arc::new(next));
        self.txn.changes.push(Change::SchemaChanged {
            graph: self.txn.working.graph_id(),
            change: SchemaChange::EdgeTypeDropped {
                graph_type: implicit_graph_type_id(),
                name,
            },
        });
        Ok(())
    }

    fn current_graph_type(&self) -> GraphResult<GraphTypeDef> {
        self.txn
            .working
            .meta
            .bound_type
            .as_deref()
            .cloned()
            .ok_or_else(|| GraphError::Inconsistent {
                reason: OPEN_GRAPH_CATALOG_DDL.to_owned(),
            })
    }
}

/// Return the implicit graph type ID used while v1.0 has one bound type per graph.
///
/// Future multi-type-bound graph work should replace this sentinel with a real
/// graph-type allocator and preserve the ID across WAL replay.
fn implicit_graph_type_id() -> GraphTypeId {
    GraphTypeId::new(1).expect("implicit graph type id")
}

fn core_node_type_def(node_type: &NodeTypeDef) -> selene_core::NodeTypeDef {
    selene_core::NodeTypeDef {
        labels: node_type.key_labels.clone(),
        properties: core_node_properties(&node_type.properties),
        key: None,
    }
}

fn core_edge_type_def(
    graph_type: &GraphTypeDef,
    edge_type: &EdgeTypeDef,
) -> GraphResult<selene_core::EdgeTypeDef> {
    let source = graph_type
        .node_types
        .get(edge_type.source_node_type as usize)
        .and_then(|node_type| node_type.key_labels.iter().next().copied())
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!(
                "edge type {} references invalid source node type {}",
                edge_type.name, edge_type.source_node_type
            ),
        })?;
    let target = graph_type
        .node_types
        .get(edge_type.target_node_type as usize)
        .and_then(|node_type| node_type.key_labels.iter().next().copied())
        .ok_or_else(|| GraphError::Inconsistent {
            reason: format!(
                "edge type {} references invalid target node type {}",
                edge_type.name, edge_type.target_node_type
            ),
        })?;
    Ok(selene_core::EdgeTypeDef {
        label: edge_type.label,
        source_node_type: NodeTypeRef(source),
        target_node_type: NodeTypeRef(target),
        properties: core_edge_properties(&edge_type.properties),
    })
}

fn core_node_properties(properties: &[PropertyTypeDef]) -> SmallVec<[PropertyDef; 8]> {
    let mut out = SmallVec::new();
    out.extend(properties.iter().map(|property| PropertyDef {
        name: property.name,
        value_type: core_value_type(property.value_type, property.required),
        nullable: !property.required,
        default: None,
    }));
    out
}

fn core_edge_properties(properties: &[PropertyTypeDef]) -> SmallVec<[PropertyDef; 4]> {
    let mut out = SmallVec::new();
    out.extend(properties.iter().map(|property| PropertyDef {
        name: property.name,
        value_type: core_value_type(property.value_type, property.required),
        nullable: !property.required,
        default: None,
    }));
    out
}

fn core_value_type(value_type: PropertyValueType, required: bool) -> ValueType {
    let predefined = match value_type {
        PropertyValueType::Bool => Some(PredefinedValueType::Bool),
        PropertyValueType::Int => Some(PredefinedValueType::Int),
        PropertyValueType::Uint => Some(PredefinedValueType::Uint),
        PropertyValueType::Int128 => Some(PredefinedValueType::Int128),
        PropertyValueType::Uint128 => Some(PredefinedValueType::Uint128),
        PropertyValueType::Float => Some(PredefinedValueType::Float),
        PropertyValueType::Float32 => Some(PredefinedValueType::Float32),
        PropertyValueType::Decimal => Some(PredefinedValueType::Decimal),
        PropertyValueType::String => Some(PredefinedValueType::String),
        PropertyValueType::Bytes => Some(PredefinedValueType::Bytes),
        PropertyValueType::Path => Some(PredefinedValueType::Path),
        PropertyValueType::NodeRef => Some(PredefinedValueType::NodeRef),
        PropertyValueType::EdgeRef => Some(PredefinedValueType::EdgeRef),
        PropertyValueType::GraphRef => Some(PredefinedValueType::GraphRef),
        PropertyValueType::TableRef => Some(PredefinedValueType::TableRef),
        PropertyValueType::ZonedDateTime => Some(PredefinedValueType::ZonedDateTime),
        PropertyValueType::LocalDateTime => Some(PredefinedValueType::LocalDateTime),
        PropertyValueType::Date => Some(PredefinedValueType::Date),
        PropertyValueType::ZonedTime => Some(PredefinedValueType::ZonedTime),
        PropertyValueType::LocalTime => Some(PredefinedValueType::LocalTime),
        PropertyValueType::Duration => Some(PredefinedValueType::Duration),
        PropertyValueType::Uuid => Some(PredefinedValueType::Uuid),
        PropertyValueType::List
        | PropertyValueType::Record
        | PropertyValueType::RecordTyped
        | PropertyValueType::Null => None,
    };
    ValueType {
        predefined,
        union: None,
        list_of: None,
        record: None,
        not_null: required,
        cardinality: selene_core::ValueTypeCardinality::ExactlyOne,
    }
}

#[cfg(test)]
mod tests;
