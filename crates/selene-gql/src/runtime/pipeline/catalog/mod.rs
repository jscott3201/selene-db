//! Catalog DDL pipeline operator.

mod compose;
mod drop_cascade;
mod drop_graph;
mod endpoints;
mod index_ddl;
mod property;

use selene_core::{IStr, LabelSet, Value, intern_with_admission};
use selene_graph::{
    EdgeEndpointDef, EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef,
    TypedIndexKind, ValidationMode as GraphValidationMode,
};
use smallvec::SmallVec;

use self::{
    compose::{compose_edge_properties, compose_node_properties},
    endpoints::resolve_endpoints,
    index_ddl::{IndexPath, create_index_plan, resolve_drop_index},
    property::{property_defs, render_property_value_type},
};
use super::catalog_index::{
    DropTarget, inline_index_specs, render_index_kind, render_index_name,
    validate_index_name_collisions,
};
use crate::{
    AnalyzedType, BindingTableColumn, BindingTableSchema, CatalogOp, GqlType, ProcedureMetadata,
    ProcedureMutability, ProcedureSignature, ProcedureTier, SourceSpan,
    runtime::{Binding, BindingTable, ExecutorError, TxContext},
};

const GRAPH_LEVEL_CATALOG_DETAIL: &str =
    "graph-level catalog ops not in v1.0 (D1 single-graph embeddable)";
const OPEN_GRAPH_CATALOG_DDL: &str =
    "open graph (GG01) does not support catalog type DDL -- use a closed graph (GG02)";

pub(super) fn execute(
    op: &CatalogOp,
    table: BindingTable,
    ctx: &mut TxContext<'_, '_>,
) -> Result<BindingTable, ExecutorError> {
    match op {
        // CREATE GRAPH stays rejected under D1 single-graph (cannot create a
        // second graph). DROP GRAPH is the IM_DROP_GRAPH factory-reset
        // (BRIEF-152, audit Item 10), handled by the drop_graph submodule.
        CatalogOp::CreateGraph { .. } => Err(ExecutorError::ImplementationDefined {
            detail: GRAPH_LEVEL_CATALOG_DETAIL,
        }),
        CatalogOp::DropGraph {
            name,
            if_exists,
            span,
        } => drop_graph::execute_drop_graph(*name, *if_exists, *span, table, ctx),
        CatalogOp::CreateNodeType {
            label,
            or_replace,
            if_not_exists,
            extends,
            properties,
            validation_mode,
            span,
        } => {
            reject_or_replace(*or_replace)?;
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            if node_type_exists(ctx.snapshot().meta.bound_type.as_deref(), *label) {
                if *if_not_exists {
                    return Ok(table);
                }
                return Err(ExecutorError::DuplicateObject {
                    kind: "node type",
                    name: *label,
                    span: *span,
                });
            }
            let indexes = inline_index_specs(properties)?;
            validate_index_name_collisions(*label, &indexes, ctx.snapshot())?;
            let properties = property_defs(properties, true)?;
            let properties = if let Some(parent) = extends {
                compose_node_properties(
                    &closed_graph_type(ctx.snapshot(), *span)?,
                    *label,
                    *parent,
                    properties,
                    *span,
                )?
            } else {
                properties
            };
            {
                let mut mutator =
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?;
                mutator
                    .create_node_type(
                        *label,
                        LabelSet::single(*label),
                        properties,
                        graph_validation_mode(*validation_mode),
                    )
                    .map_err(|source| catalog_graph_error(source, *span))?;
                for index in indexes {
                    mutator
                        .create_property_index_named(*label, index.property, index.kind, index.name)
                        .map_err(|source| catalog_graph_error(source, index.span))?;
                }
            }
            Ok(table)
        }
        CatalogOp::CreateEdgeType {
            label,
            or_replace,
            if_not_exists,
            extends,
            endpoints,
            properties,
            validation_mode,
            span,
        } => {
            reject_or_replace(*or_replace)?;
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            if edge_type_exists(ctx.snapshot().meta.bound_type.as_deref(), *label) {
                if *if_not_exists {
                    return Ok(table);
                }
                return Err(ExecutorError::DuplicateObject {
                    kind: "edge type",
                    name: *label,
                    span: *span,
                });
            }
            let graph_type = closed_graph_type(ctx.snapshot(), *span)?;
            let (source, target) = endpoints
                .as_ref()
                .map(|endpoints| resolve_endpoints(endpoints, &graph_type, *span))
                .transpose()?
                .unwrap_or((EdgeEndpointDef::Any, EdgeEndpointDef::Any));
            let properties = property_defs(properties, false)?;
            let properties = if let Some(parent) = extends {
                compose_edge_properties(&graph_type, *label, *parent, properties, *span)?
            } else {
                properties
            };
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .create_edge_type(
                    *label,
                    *label,
                    source,
                    target,
                    properties,
                    graph_validation_mode(*validation_mode),
                )
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::DropNodeType {
            label,
            if_exists,
            behavior,
            span,
        } => drop_cascade::execute_drop_node_type(*label, *if_exists, *behavior, *span, table, ctx),
        CatalogOp::DropEdgeType {
            label,
            if_exists,
            behavior,
            span,
        } => drop_cascade::execute_drop_edge_type(*label, *if_exists, *behavior, *span, table, ctx),
        CatalogOp::TruncateNodeType { label, span } => {
            // TRUNCATE removes instances by label (IM_TRUNCATE, audit Item 11).
            // It is valid on both open (GG01) and closed (GG02) graphs — unlike
            // DROP NODE TYPE it never touches the catalog, so no closed-graph
            // gate. An absent label is a clean no-op inside the mutator.
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .truncate_node_type(*label)
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::TruncateEdgeType { label, span } => {
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .truncate_edge_type(*label)
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::CreateIndex {
            name,
            label,
            properties,
            if_not_exists,
            span,
        } => {
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            let Some(index_path) =
                create_index_plan(ctx, *name, *label, properties, *if_not_exists, *span)?
            else {
                return Ok(table);
            };
            match index_path {
                IndexPath::Single { property, kind } => {
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                        .create_property_index_named(*label, property, kind, Some(*name))
                        .map_err(|source| catalog_graph_error(source, *span))?;
                }
                IndexPath::Composite { properties, kinds } => {
                    let properties = properties.into_iter().collect::<SmallVec<[IStr; 4]>>();
                    let kinds = kinds.into_iter().collect::<SmallVec<[TypedIndexKind; 4]>>();
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                        .create_composite_property_index_named(
                            *label,
                            properties,
                            kinds,
                            Some(*name),
                        )
                        .map_err(|source| catalog_graph_error(source, *span))?;
                }
            }
            Ok(table)
        }
        CatalogOp::DropIndex {
            name,
            if_exists,
            span,
        } => {
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            let Some(target) = resolve_drop_index(ctx.snapshot(), *name, *if_exists, *span)? else {
                return Ok(table);
            };
            match target {
                DropTarget::Single { label, property } => {
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                        .drop_property_index(label, property)
                        .map_err(|source| catalog_graph_error(source, *span))?;
                }
                DropTarget::Composite { label, properties } => {
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                        .drop_composite_property_index(label, properties)
                        .map_err(|source| catalog_graph_error(source, *span))?;
                }
            }
            Ok(table)
        }
        CatalogOp::ShowNodeTypes(_) => show_node_types(ctx),
        CatalogOp::ShowEdgeTypes(_) => show_edge_types(ctx),
        CatalogOp::ShowIndexes(_) => show_indexes(ctx),
        CatalogOp::ShowProcedures(_) => show_procedures(ctx),
    }
}

fn reject_or_replace(or_replace: bool) -> Result<(), ExecutorError> {
    if or_replace {
        return Err(ExecutorError::ImplementationDefined {
            detail: "OR REPLACE not implemented for catalog DDL",
        });
    }
    Ok(())
}

const fn graph_validation_mode(mode: Option<crate::ValidationMode>) -> GraphValidationMode {
    match mode {
        Some(crate::ValidationMode::Warn) => GraphValidationMode::Warn,
        Some(crate::ValidationMode::Strict) | None => GraphValidationMode::Strict,
    }
}

fn node_type_exists(graph_type: Option<&GraphTypeDef>, label: IStr) -> bool {
    graph_type
        .map(|graph_type| {
            graph_type
                .node_types
                .iter()
                .any(|node_type| node_type.name == label)
        })
        .unwrap_or(false)
}

fn edge_type_exists(graph_type: Option<&GraphTypeDef>, label: IStr) -> bool {
    graph_type
        .map(|graph_type| {
            graph_type
                .edge_types
                .iter()
                .any(|edge_type| edge_type.name == label)
        })
        .unwrap_or(false)
}

fn closed_graph_type(
    graph: &selene_graph::SeleneGraph,
    span: SourceSpan,
) -> Result<GraphTypeDef, ExecutorError> {
    graph
        .meta
        .bound_type
        .as_deref()
        .cloned()
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message: OPEN_GRAPH_CATALOG_DDL.to_owned(),
            span,
        })
}

fn show_node_types(ctx: &TxContext<'_, '_>) -> Result<BindingTable, ExecutorError> {
    let rows = ctx
        .snapshot()
        .meta
        .bound_type
        .as_deref()
        .map(|graph_type| {
            graph_type
                .node_types
                .iter()
                .map(|node_type| {
                    let label = render_node_label_name(&node_type.key_labels);
                    show_row(&label, &render_node_type_def(node_type))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(BindingTable::new(show_schema()?, rows))
}

fn show_edge_types(ctx: &TxContext<'_, '_>) -> Result<BindingTable, ExecutorError> {
    let rows = ctx
        .snapshot()
        .meta
        .bound_type
        .as_deref()
        .map(|graph_type| {
            graph_type
                .edge_types
                .iter()
                .map(|edge_type| {
                    show_row(
                        edge_type.label.as_str(),
                        &render_edge_type_def(graph_type, edge_type),
                    )
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(BindingTable::new(show_schema()?, rows))
}

fn show_indexes(ctx: &TxContext<'_, '_>) -> Result<BindingTable, ExecutorError> {
    let mut indexes = ctx
        .snapshot()
        .iter_property_index_entries()
        .collect::<Vec<_>>();
    indexes.sort_by(
        |(left_label, left_property, _, _), (right_label, right_property, _, _)| {
            left_label
                .as_str()
                .cmp(right_label.as_str())
                .then_with(|| left_property.as_str().cmp(right_property.as_str()))
        },
    );
    let rows = indexes
        .into_iter()
        .map(|(label, property, kind, name)| {
            let name = render_index_name(label, property, name);
            Ok(Binding::new([
                Value::String(intern_runtime(&name)?),
                Value::String(label),
                Value::String(property),
                Value::String(intern_runtime(render_index_kind(kind))?),
            ]))
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTable::new(
        string_schema(&["name", "label", "property", "kind"])?,
        rows,
    ))
}

fn show_procedures(ctx: &TxContext<'_, '_>) -> Result<BindingTable, ExecutorError> {
    let mut procedures = ctx.registry().iter_handles().collect::<Vec<_>>();
    procedures.sort_by(|(left, _), (right, _)| {
        render_procedure_name(left).cmp(&render_procedure_name(right))
    });
    let rows = procedures
        .into_iter()
        .map(|(name, metadata)| procedure_row(&name, &metadata))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BindingTable::new(
        string_schema(&[
            "name",
            "tier",
            "mutability",
            "signature",
            "description",
            "since_version",
            "capability_required",
        ])?,
        rows,
    ))
}

fn show_row(label: &str, definition: &str) -> Result<Binding, ExecutorError> {
    Ok(Binding::new([
        Value::String(intern_runtime(label)?),
        Value::String(intern_runtime(definition)?),
    ]))
}

fn show_schema() -> Result<BindingTableSchema, ExecutorError> {
    Ok(BindingTableSchema {
        columns: vec![
            BindingTableColumn {
                name: Some(intern_runtime("label")?),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            },
            BindingTableColumn {
                name: Some(intern_runtime("definition")?),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
        ],
    })
}

fn string_schema(names: &[&str]) -> Result<BindingTableSchema, ExecutorError> {
    let columns = names
        .iter()
        .map(|name| {
            Ok(BindingTableColumn {
                name: Some(intern_runtime(name)?),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            })
        })
        .collect::<Result<Vec<_>, ExecutorError>>()?;
    Ok(BindingTableSchema { columns })
}

pub(super) fn intern_runtime(value: &str) -> Result<IStr, ExecutorError> {
    intern_with_admission(value)
        .map(|(value, _was_new)| value)
        .map_err(|_err| ExecutorError::ImplementationDefined {
            detail: "interner cap exhausted during catalog rendering",
        })
}

fn procedure_row(name: &[IStr], metadata: &ProcedureMetadata) -> Result<Binding, ExecutorError> {
    let name = render_procedure_name(name);
    Ok(Binding::new([
        Value::String(intern_runtime(&name)?),
        Value::String(intern_runtime(render_tier(metadata.tier))?),
        Value::String(intern_runtime(render_mutability(metadata.mutability))?),
        Value::String(intern_runtime(&render_signature(
            &name,
            &metadata.signature,
        ))?),
        Value::String(intern_runtime(metadata.description)?),
        Value::String(intern_runtime(metadata.signature.since_version)?),
        Value::String(intern_runtime(
            metadata.capability_required.as_deref().unwrap_or(""),
        )?),
    ]))
}

fn render_procedure_name(name: &[IStr]) -> String {
    name.iter()
        .map(|part| part.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn render_tier(tier: ProcedureTier) -> &'static str {
    match tier {
        ProcedureTier::Graph => "graph",
        ProcedureTier::Mutation => "mutation",
        ProcedureTier::Persist => "persist",
    }
}

fn render_mutability(mutability: ProcedureMutability) -> &'static str {
    match mutability {
        ProcedureMutability::Read => "read",
        ProcedureMutability::GraphWrite => "graph_write",
        ProcedureMutability::SchemaWrite => "schema_write",
        ProcedureMutability::Admin => "admin",
    }
}

fn render_signature(name: &str, signature: &ProcedureSignature) -> String {
    let params = signature
        .parameters
        .iter()
        .map(|parameter| {
            let nullable = if parameter.nullable { "?" } else { "" };
            format!(
                "{}: {}{}",
                parameter.name,
                render_gql_type(&parameter.ty),
                nullable
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({params})")
}

fn render_gql_type(ty: &GqlType) -> String {
    match ty {
        GqlType::String => "STRING".to_owned(),
        GqlType::Boolean => "BOOLEAN".to_owned(),
        GqlType::Integer => "INTEGER".to_owned(),
        GqlType::Float => "FLOAT".to_owned(),
        GqlType::Int8 => "INT8".to_owned(),
        GqlType::Int16 => "INT16".to_owned(),
        GqlType::Int32 => "INT32".to_owned(),
        GqlType::Int64 => "INT64".to_owned(),
        GqlType::Int128 => "INT128".to_owned(),
        GqlType::Uint8 => "UINT8".to_owned(),
        GqlType::Uint16 => "UINT16".to_owned(),
        GqlType::Uint32 => "UINT32".to_owned(),
        GqlType::Uint64 => "UINT64".to_owned(),
        GqlType::Uint128 => "UINT128".to_owned(),
        GqlType::SmallInt => "SMALLINT".to_owned(),
        GqlType::BigInt => "BIGINT".to_owned(),
        GqlType::Decimal => "DECIMAL".to_owned(),
        GqlType::Float32 => "FLOAT32".to_owned(),
        GqlType::Float64 => "FLOAT64".to_owned(),
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => "BYTES".to_owned(),
        GqlType::Uuid => "UUID".to_owned(),
        GqlType::ZonedDateTime => "ZONED DATETIME".to_owned(),
        GqlType::LocalDateTime => "LOCAL DATETIME".to_owned(),
        GqlType::Date => "DATE".to_owned(),
        GqlType::ZonedTime => "ZONED TIME".to_owned(),
        GqlType::LocalTime => "LOCAL TIME".to_owned(),
        GqlType::Duration => "DURATION".to_owned(),
        GqlType::Record(_) => "RECORD".to_owned(),
        GqlType::List(inner) => format!("LIST<{}>", render_gql_type(inner)),
        GqlType::Path => "PATH".to_owned(),
        GqlType::GraphRef => "GRAPH".to_owned(),
        GqlType::NodeRef => "NODE".to_owned(),
        GqlType::EdgeRef => "EDGE".to_owned(),
        GqlType::TableRef => "TABLE".to_owned(),
        GqlType::Null => "NULL".to_owned(),
        GqlType::Nothing => "NOTHING".to_owned(),
    }
}

fn render_node_type_def(node_type: &NodeTypeDef) -> String {
    format!(
        "CREATE NODE TYPE {} ({})",
        render_node_label_set(&node_type.key_labels),
        render_properties(&node_type.properties)
    )
}

fn render_edge_type_def(graph_type: &GraphTypeDef, edge_type: &EdgeTypeDef) -> String {
    let endpoint_clause = render_edge_endpoint_clause(graph_type, edge_type);
    let properties = render_properties(&edge_type.properties);
    let body = match (endpoint_clause.is_empty(), properties.is_empty()) {
        (true, true) => String::new(),
        (true, false) => properties,
        (false, true) => endpoint_clause,
        (false, false) => format!("{endpoint_clause}, {properties}"),
    };
    format!("CREATE EDGE TYPE :{} ({body})", edge_type.label)
}

fn render_edge_endpoint_clause(graph_type: &GraphTypeDef, edge_type: &EdgeTypeDef) -> String {
    if edge_type.source_node_type == EdgeEndpointDef::Any
        || edge_type.target_node_type == EdgeEndpointDef::Any
    {
        // Partial Any has no DDL syntax; keep SHOW output parseable.
        // OneOf endpoints ARE renderable (their member labels form a valid
        // enumerated-FROM/TO clause), so they fall through to render_endpoint.
        return String::new();
    }
    let source = render_endpoint(graph_type, &edge_type.source_node_type);
    let target = render_endpoint(graph_type, &edge_type.target_node_type);
    format!("FROM {source} TO {target}")
}

fn render_endpoint(graph_type: &GraphTypeDef, endpoint: &EdgeEndpointDef) -> String {
    match endpoint {
        EdgeEndpointDef::Any => "ANY".to_owned(),
        EdgeEndpointDef::NodeType(index) => {
            render_endpoint_label_set(&graph_type.node_types[*index as usize].key_labels)
        }
        EdgeEndpointDef::OneOf(indices) => indices
            .iter()
            .map(|index| {
                render_endpoint_label_set(&graph_type.node_types[*index as usize].key_labels)
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}

fn render_endpoint_label_set(labels: &LabelSet) -> String {
    labels
        .iter()
        .map(|label| format!(":{label}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn render_node_label_set(labels: &LabelSet) -> String {
    let label = render_node_label_name(labels);
    if label.is_empty() {
        String::new()
    } else {
        format!(":{label}")
    }
}

fn render_node_label_name(labels: &LabelSet) -> String {
    labels
        .iter()
        .map(|label| label.as_str())
        .collect::<Vec<_>>()
        .join(":")
}

fn render_properties(properties: &[PropertyTypeDef]) -> String {
    properties
        .iter()
        .map(|property| {
            let nullability = if property.required { " NOT NULL" } else { "" };
            let default = property
                .default
                .as_ref()
                .map(render_default_value)
                .map(|value| format!(" DEFAULT {value}"))
                .unwrap_or_default();
            let immutable = if property.immutable { " IMMUTABLE" } else { "" };
            format!(
                "{} :: {}{}{}{}",
                property.name,
                render_property_value_type(
                    property.value_type,
                    property.list_element_type.as_ref()
                ),
                nullability,
                default,
                immutable
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_default_value(default: &selene_graph::PropertyDefaultValue) -> String {
    match default {
        selene_graph::PropertyDefaultValue::Null => "NULL".to_owned(),
        selene_graph::PropertyDefaultValue::Boolean(value) => value.to_string().to_uppercase(),
        selene_graph::PropertyDefaultValue::Integer(value) => value.to_string(),
        selene_graph::PropertyDefaultValue::String(value) => format!("'{}'", value.as_str()),
        _ => "<unsupported-default>".to_owned(),
    }
}

fn catalog_graph_error(source: GraphError, span: SourceSpan) -> ExecutorError {
    match source {
        GraphError::Inconsistent { reason } => ExecutorError::GraphTypeViolation {
            message: reason,
            span,
        },
        source => ExecutorError::GraphMutation { source, span },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn istr(value: &str) -> IStr {
        intern_with_admission(value).expect("test label admits").0
    }

    #[test]
    fn render_partial_any_edge_endpoint_as_endpoint_less_ddl() {
        let person = istr("Person");
        let graph_type = GraphTypeDef {
            name: istr("catalog.partial.any.graph"),
            node_types: vec![NodeTypeDef {
                name: person,
                key_labels: LabelSet::single(person),
                properties: Vec::new(),
                validation_mode: GraphValidationMode::Strict,
            }],
            edge_types: Vec::new(),
        };
        let knows = istr("KNOWS");

        for (source_node_type, target_node_type) in [
            (EdgeEndpointDef::Any, EdgeEndpointDef::NodeType(0)),
            (EdgeEndpointDef::NodeType(0), EdgeEndpointDef::Any),
        ] {
            let edge_type = EdgeTypeDef {
                name: knows,
                label: knows,
                source_node_type,
                target_node_type,
                properties: Vec::new(),
                validation_mode: GraphValidationMode::Strict,
            };
            let rendered = render_edge_type_def(&graph_type, &edge_type);
            assert_eq!(rendered, "CREATE EDGE TYPE :KNOWS ()");
            crate::parse(&rendered).expect("rendered edge type DDL parses");
        }
    }
}
