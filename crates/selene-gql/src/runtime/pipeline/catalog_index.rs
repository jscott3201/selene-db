//! Inline property-index helpers for catalog DDL.

use selene_core::IStr;
use selene_graph::TypedIndexKind;

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
    pub(super) same_pair: Option<Option<IStr>>,
    pub(super) other_name_matches: Vec<(IStr, IStr)>,
}

pub(super) fn inline_index_specs(
    properties: &[PlannedTypePropertyDef],
) -> Result<Vec<InlineIndexSpec>, ExecutorError> {
    let mut indexes = Vec::new();
    for property in properties {
        for constraint in &property.constraints {
            if let PlannedTypePropertyConstraint::Indexed { name, span } = constraint {
                indexes.push(InlineIndexSpec {
                    property: property.name,
                    kind: gql_type_to_index_kind(&property.gql_type, *span)?,
                    name: *name,
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
    for index in indexes {
        let rendered = render_index_name(label, index.property, index.name);
        if used.iter().any(|name| name == &rendered) {
            let name = index.name.unwrap_or(intern_runtime(&rendered)?);
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
    property: IStr,
) -> IndexConflictReport {
    let mut same_pair = None;
    let mut other_name_matches = Vec::new();
    for (entry_label, entry_property, _, entry_name) in graph.iter_property_index_entries() {
        if entry_label == label && entry_property == property {
            same_pair = Some(entry_name);
            continue;
        }
        if render_index_name(entry_label, entry_property, entry_name) == ident.as_str() {
            other_name_matches.push((entry_label, entry_property));
        }
    }
    IndexConflictReport {
        same_pair,
        other_name_matches,
    }
}

pub(super) fn resolve_drop_index_matches(
    graph: &selene_graph::SeleneGraph,
    ident: IStr,
) -> Vec<(IStr, IStr)> {
    graph
        .iter_property_index_entries()
        .filter_map(|(label, property, _, name)| {
            (render_index_name(label, property, name) == ident.as_str())
                .then_some((label, property))
        })
        .collect()
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
        _ => Err(ExecutorError::FeatureNotInV1_1 {
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
