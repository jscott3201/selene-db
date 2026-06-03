//! Inline property-index helpers for catalog DDL.

use selene_core::IStr;
use selene_graph::{TypedIndexKind, VectorIndexKind};
use smallvec::SmallVec;

use super::catalog::intern_runtime;
use crate::{
    ExecutorError, GqlType, PlannedTypePropertyConstraint, PlannedTypePropertyDef, SourceSpan,
};

pub(super) struct InlineIndexSpec {
    pub(super) property: IStr,
    pub(super) kind: TypedIndexKind,
    pub(super) name: Option<IStr>,
    pub(super) span: SourceSpan,
}

pub(super) struct IndexConflictReport {
    pub(super) same_pair_name: Option<String>,
    pub(super) other_name_matches: Vec<DropTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DropTarget {
    Single {
        label: IStr,
        property: IStr,
    },
    Composite {
        label: IStr,
        properties: SmallVec<[IStr; 4]>,
    },
}

pub(super) fn inline_index_specs(
    properties: &[PlannedTypePropertyDef],
) -> Result<Vec<InlineIndexSpec>, ExecutorError> {
    let mut indexes = Vec::new();
    for property in properties {
        for constraint in &property.constraints {
            if let PlannedTypePropertyConstraint::Indexed { name, span } = constraint {
                indexes.push(InlineIndexSpec {
                    property: property.name.clone(),
                    kind: gql_type_to_index_kind(&property.gql_type, *span)?,
                    name: name.clone(),
                    span: *span,
                });
            }
        }
    }
    Ok(indexes)
}

pub(super) fn validate_index_name_collisions(
    label: IStr,
    indexes: &[InlineIndexSpec],
    graph: &selene_graph::SeleneGraph,
) -> Result<(), ExecutorError> {
    let mut used = graph
        .iter_property_index_entries()
        .map(|(label, property, _, name)| render_index_name(label, property, name))
        .collect::<Vec<_>>();
    used.extend(
        graph
            .iter_composite_property_index_entries()
            .map(|(label, properties, _, name)| {
                render_composite_index_name(label, &properties, name)
            }),
    );
    used.extend(
        graph
            .iter_vector_index_entries()
            .map(|(label, property, _, _, name)| render_vector_index_name(label, property, name)),
    );
    for index in indexes {
        let rendered = render_index_name(label.clone(), index.property.clone(), index.name.clone());
        if used.iter().any(|name| name == &rendered) {
            let name = index.name.clone().unwrap_or(intern_runtime(&rendered)?);
            return Err(ExecutorError::DuplicateObject {
                kind: "index",
                name,
                span: index.span,
            });
        }
        used.push(rendered);
    }
    Ok(())
}

pub(super) fn lookup_index_entries(
    graph: &selene_graph::SeleneGraph,
    ident: IStr,
    label: IStr,
    properties: &[IStr],
) -> IndexConflictReport {
    let mut same_pair_name = None;
    let mut other_name_matches = Vec::new();
    for (entry_label, entry_property, _, entry_name) in graph.iter_property_index_entries() {
        if entry_label == label && properties == [entry_property.clone()] {
            same_pair_name = Some(render_index_name(entry_label, entry_property, entry_name));
            continue;
        }
        if render_index_name(entry_label.clone(), entry_property.clone(), entry_name)
            == ident.as_str()
        {
            other_name_matches.push(DropTarget::Single {
                label: entry_label,
                property: entry_property,
            });
        }
    }
    for (entry_label, entry_properties, _, entry_name) in
        graph.iter_composite_property_index_entries()
    {
        if entry_label == label && same_property_set(&entry_properties, properties) {
            same_pair_name = Some(render_composite_index_name(
                entry_label,
                &entry_properties,
                entry_name,
            ));
            continue;
        }
        if render_composite_index_name(entry_label.clone(), &entry_properties, entry_name)
            == ident.as_str()
        {
            other_name_matches.push(DropTarget::Composite {
                label: entry_label,
                properties: entry_properties,
            });
        }
    }
    IndexConflictReport {
        same_pair_name,
        other_name_matches,
    }
}

pub(super) fn resolve_drop_index_matches(
    graph: &selene_graph::SeleneGraph,
    ident: IStr,
) -> Vec<DropTarget> {
    let mut matches = graph
        .iter_property_index_entries()
        .filter_map(|(label, property, _, name)| {
            (render_index_name(label.clone(), property.clone(), name) == ident.as_str())
                .then_some(DropTarget::Single { label, property })
        })
        .collect::<Vec<_>>();
    matches.extend(graph.iter_composite_property_index_entries().filter_map(
        |(label, properties, _, name)| {
            (render_composite_index_name(label.clone(), &properties, name) == ident.as_str())
                .then_some(DropTarget::Composite { label, properties })
        },
    ));
    matches.sort_by_key(render_drop_target);
    matches
}

fn gql_type_to_index_kind(
    gql_type: &GqlType,
    span: SourceSpan,
) -> Result<TypedIndexKind, ExecutorError> {
    match gql_type {
        GqlType::String => Ok(TypedIndexKind::String),
        GqlType::Uuid => Ok(TypedIndexKind::Uuid),
        GqlType::Integer
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::SmallInt
        | GqlType::BigInt => Ok(TypedIndexKind::I64),
        GqlType::Float64 => Ok(TypedIndexKind::F64),
        GqlType::Date => Ok(TypedIndexKind::Date),
        GqlType::LocalDateTime => Ok(TypedIndexKind::LocalDateTime),
        _ => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "inline INDEXED for this GQL type",
            span,
        }),
    }
}

pub(super) fn render_index_name(label: IStr, property: IStr, explicit: Option<IStr>) -> String {
    explicit
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| render_auto_index_name(label, property))
}

pub(super) fn render_composite_index_name(
    label: IStr,
    properties: &[IStr],
    explicit: Option<IStr>,
) -> String {
    explicit
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| render_composite_auto_index_name(label, properties))
}

pub(super) fn render_vector_index_name(
    label: IStr,
    property: IStr,
    explicit: Option<IStr>,
) -> String {
    explicit
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| render_vector_auto_index_name(label, property))
}

fn render_auto_index_name(label: IStr, property: IStr) -> String {
    let label = label.as_str();
    let property = property.as_str();
    format!(
        "idx:{}:{}:{}:{}",
        label.len(),
        label,
        property.len(),
        property
    )
}

fn render_vector_auto_index_name(label: IStr, property: IStr) -> String {
    let label = label.as_str();
    let property = property.as_str();
    format!(
        "vidx:{}:{}:{}:{}",
        label.len(),
        label,
        property.len(),
        property
    )
}

fn render_composite_auto_index_name(label: IStr, properties: &[IStr]) -> String {
    let label = label.as_str();
    let mut rendered = format!("idx:{}:{}:c{}", label.len(), label, properties.len());
    for property in properties {
        let property = property.as_str();
        rendered.push_str(&format!(":{}:{}", property.len(), property));
    }
    rendered
}

pub(super) fn render_drop_target(target: &DropTarget) -> String {
    match target {
        DropTarget::Single { label, property } => {
            format!(":{}({})", label.as_str(), property.as_str())
        }
        DropTarget::Composite { label, properties } => format!(
            ":{}({})",
            label.as_str(),
            properties
                .iter()
                .map(|property| property.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn same_property_set(lhs: &[IStr], rhs: &[IStr]) -> bool {
    if lhs.len() != rhs.len() {
        return false;
    }
    let mut lhs = lhs.to_vec();
    let mut rhs = rhs.to_vec();
    lhs.sort_unstable();
    rhs.sort_unstable();
    lhs == rhs
}

pub(super) fn render_index_kind(kind: TypedIndexKind) -> &'static str {
    match kind {
        TypedIndexKind::I64 => "i64",
        TypedIndexKind::F64 => "f64",
        TypedIndexKind::String => "string",
        TypedIndexKind::Date => "date",
        TypedIndexKind::LocalDateTime => "local_datetime",
        TypedIndexKind::Uuid => "uuid",
    }
}

pub(super) fn render_vector_index_kind(kind: VectorIndexKind, dimension: u32) -> String {
    match kind {
        VectorIndexKind::Flat => format!("vector_flat({dimension})"),
    }
}
