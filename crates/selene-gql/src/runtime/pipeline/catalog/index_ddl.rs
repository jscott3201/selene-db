//! Runtime helpers for named property-index DDL.

use selene_core::{IStr, PropertyValueType};
use selene_graph::{GraphTypeDef, NodeTypeDef, TypedIndexKind};

use super::intern_runtime;
use crate::{
    SourceSpan,
    runtime::{ExecutorError, TxContext},
};

use super::super::catalog_index::{
    lookup_index_entries, render_index_name, resolve_drop_index_matches,
};

pub(super) enum IndexPath {
    Single {
        property: IStr,
        kind: TypedIndexKind,
    },
    Composite {
        properties: Vec<IStr>,
        kinds: Vec<TypedIndexKind>,
    },
}

pub(super) fn create_index_plan(
    ctx: &TxContext<'_, '_>,
    name: IStr,
    label: IStr,
    properties: &[IStr],
    if_not_exists: bool,
    span: SourceSpan,
) -> Result<Option<IndexPath>, ExecutorError> {
    let graph = ctx.snapshot();
    let graph_type = graph
        .meta
        .bound_type
        .as_deref()
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message:
                "CREATE INDEX requires a bound graph type; use CALL selene.create_index(...) on open graphs"
                    .to_owned(),
            span,
        })?;
    let node_type = index_node_type(graph_type, label, span)?;
    let path = dispatch_index_properties(node_type, label, properties, span)?;
    match path {
        IndexPath::Single { property, kind } => {
            create_single_index_plan(graph, name, label, property, kind, if_not_exists, span)
        }
        IndexPath::Composite { properties, kinds } => {
            create_composite_index_plan(name, label, properties, kinds, if_not_exists, span)
        }
    }
}

fn create_single_index_plan(
    graph: &selene_graph::SeleneGraph,
    name: IStr,
    label: IStr,
    property: IStr,
    kind: TypedIndexKind,
    if_not_exists: bool,
    span: SourceSpan,
) -> Result<Option<IndexPath>, ExecutorError> {
    let report = lookup_index_entries(graph, name, label, property);
    if !report.other_name_matches.is_empty() {
        return Err(ExecutorError::DuplicateObject {
            kind: "index",
            name,
            span,
        });
    }
    if let Some(existing_name) = report.same_pair {
        if if_not_exists {
            return Ok(None);
        }
        let existing = render_index_name(label, property, existing_name);
        return Err(ExecutorError::DuplicateObject {
            kind: "index",
            name: intern_runtime(&existing)?,
            span,
        });
    }
    Ok(Some(IndexPath::Single { property, kind }))
}

fn dispatch_index_properties(
    node_type: &NodeTypeDef,
    label: IStr,
    properties: &[IStr],
    span: SourceSpan,
) -> Result<IndexPath, ExecutorError> {
    match properties {
        [] => Err(ExecutorError::GraphTypeViolation {
            message: "CREATE INDEX requires at least one property".to_owned(),
            span,
        }),
        [property] => Ok(IndexPath::Single {
            property: *property,
            kind: index_kind_for_property(node_type, label, *property, span)?,
        }),
        _ => {
            let kinds = properties
                .iter()
                .map(|property| index_kind_for_property(node_type, label, *property, span))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IndexPath::Composite {
                properties: properties.to_vec(),
                kinds,
            })
        }
    }
}

fn create_composite_index_plan(
    _name: IStr,
    _label: IStr,
    properties: Vec<IStr>,
    _kinds: Vec<TypedIndexKind>,
    _if_not_exists: bool,
    span: SourceSpan,
) -> Result<Option<IndexPath>, ExecutorError> {
    Err(ExecutorError::GraphTypeViolation {
        message: format!(
            "CREATE INDEX on {} properties -- composite path not yet wired; C5 lands the executor",
            properties.len()
        ),
        span,
    })
}

fn index_node_type(
    graph_type: &GraphTypeDef,
    label: IStr,
    span: SourceSpan,
) -> Result<&NodeTypeDef, ExecutorError> {
    if let Some(index) = graph_type.node_type_index_for(label) {
        return Ok(&graph_type.node_types[index as usize]);
    }
    if graph_type.edge_type_index_for(label).is_some() {
        return Err(ExecutorError::GraphTypeViolation {
            message: format!(
                "CREATE INDEX on edge label ':{}' -- edge-property indexes ship in BRIEF-140c",
                label.as_str()
            ),
            span,
        });
    }
    Err(ExecutorError::GraphTypeViolation {
        message: format!("node type ':{}' is not declared", label.as_str()),
        span,
    })
}

fn index_kind_for_property(
    node_type: &NodeTypeDef,
    label: IStr,
    property: IStr,
    span: SourceSpan,
) -> Result<TypedIndexKind, ExecutorError> {
    let property_def = node_type
        .properties
        .iter()
        .find(|candidate| candidate.name == property)
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message: format!(
                "property '{}' is not declared on type ':{}'",
                property.as_str(),
                label.as_str()
            ),
            span,
        })?;
    match property_def.value_type {
        PropertyValueType::Int => Ok(TypedIndexKind::I64),
        PropertyValueType::Float => Ok(TypedIndexKind::F64),
        PropertyValueType::String => Ok(TypedIndexKind::String),
        PropertyValueType::Date => Ok(TypedIndexKind::Date),
        PropertyValueType::LocalDateTime => Ok(TypedIndexKind::LocalDateTime),
        PropertyValueType::Uuid => Ok(TypedIndexKind::Uuid),
        value_type => Err(ExecutorError::GraphTypeViolation {
            message: format!(
                "property kind {} is not supported for property indexes",
                value_type.name()
            ),
            span,
        }),
    }
}

pub(super) fn resolve_drop_index(
    graph: &selene_graph::SeleneGraph,
    name: IStr,
    if_exists: bool,
    span: SourceSpan,
) -> Result<Option<(IStr, IStr)>, ExecutorError> {
    let matches = resolve_drop_index_matches(graph, name);
    match matches.as_slice() {
        [] if if_exists => Ok(None),
        [] => Err(ExecutorError::GraphTypeViolation {
            message: format!("index '{}' does not exist", name.as_str()),
            span,
        }),
        [pair] => Ok(Some(*pair)),
        pairs => Err(ExecutorError::GraphTypeViolation {
            message: format!(
                "index '{}' is ambiguous: matches {} entries across pairs {}",
                name.as_str(),
                pairs.len(),
                render_index_pair_list(pairs)
            ),
            span,
        }),
    }
}

fn render_index_pair_list(pairs: &[(IStr, IStr)]) -> String {
    pairs
        .iter()
        .map(|(label, property)| format!(":{}({})", label.as_str(), property.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}
