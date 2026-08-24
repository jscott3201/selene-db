//! Facade-owned closed graph-type definitions.

use std::collections::BTreeSet;

use selene_core::{LabelSet, db_string};
use selene_graph::{GraphTypeDef, NodeTypeDef, ValidationMode};

use crate::{Error, PathSegment, Result};

/// A named node type with one or more defining labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeTypeDefinition {
    name: PathSegment,
    labels: Vec<PathSegment>,
}

impl NodeTypeDefinition {
    /// Construct a node type from validated logical names.
    ///
    /// # Errors
    ///
    /// Returns an invalid-definition error when `labels` is empty or contains
    /// the same canonical label more than once.
    pub fn new(name: PathSegment, labels: Vec<PathSegment>) -> Result<Self> {
        if labels.is_empty() {
            return Err(Error::invalid_graph_type(
                "a node type requires at least one defining label",
            ));
        }
        let unique = labels.iter().collect::<BTreeSet<_>>();
        if unique.len() != labels.len() {
            return Err(Error::invalid_graph_type(
                "a node type cannot repeat a canonical label",
            ));
        }
        Ok(Self { name, labels })
    }
}

/// Property-free closed graph-type definition for catalog lifecycle use.
///
/// The builder is intentionally extensible: later work can add facade-owned
/// property and edge definitions without exposing lower graph schema types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphTypeDefinition {
    node_types: Vec<NodeTypeDefinition>,
}

impl GraphTypeDefinition {
    /// Start a graph-type definition.
    #[must_use]
    pub fn builder() -> GraphTypeBuilder {
        GraphTypeBuilder::default()
    }

    pub(crate) fn into_runtime(self, name: &PathSegment) -> Result<GraphTypeDef> {
        let runtime = GraphTypeDef {
            name: db_string(name.display()).map_err(Error::invalid_graph_type_source)?,
            node_types: self
                .node_types
                .into_iter()
                .map(|node| {
                    let labels = node
                        .labels
                        .into_iter()
                        .map(|label| db_string(label.display()))
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    Ok(NodeTypeDef {
                        name: db_string(node.name.display())?,
                        key_labels: LabelSet::from_iter(labels),
                        properties: Vec::new(),
                        validation_mode: ValidationMode::Strict,
                    })
                })
                .collect::<std::result::Result<Vec<_>, selene_core::CoreError>>()
                .map_err(Error::invalid_graph_type_source)?,
            edge_types: Vec::new(),
        };
        runtime.validate().map_err(Error::invalid_graph_type_source)
    }
}

/// Builder for a facade-owned closed graph type.
#[derive(Clone, Debug, Default)]
pub struct GraphTypeBuilder {
    node_types: Vec<NodeTypeDefinition>,
}

impl GraphTypeBuilder {
    /// Append one node type.
    #[must_use]
    pub fn with_node_type(mut self, node_type: NodeTypeDefinition) -> Self {
        self.node_types.push(node_type);
        self
    }

    /// Validate and finish the definition.
    ///
    /// # Errors
    ///
    /// Returns an invalid-definition error when no node type was declared or
    /// when node names or defining label sets are duplicated.
    pub fn build(self) -> Result<GraphTypeDefinition> {
        if self.node_types.is_empty() {
            return Err(Error::invalid_graph_type(
                "a catalog graph type requires at least one node type",
            ));
        }
        let definition = GraphTypeDefinition {
            node_types: self.node_types,
        };
        definition.clone().into_runtime(
            &PathSegment::regular("validation")
                .expect("static validation graph-type name is valid"),
        )?;
        Ok(definition)
    }
}
