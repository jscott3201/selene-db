//! Catalog DDL pipeline operator.

use selene_core::{IStr, LabelSet, PropertyValueType, Value, intern_with_admission};
use selene_graph::{
    EdgeTypeDef, GraphError, GraphTypeDef, NodeTypeDef, PropertyTypeDef, TypedIndexKind,
};

use crate::{
    AnalyzedType, BindingTableColumn, BindingTableSchema, CatalogOp, EdgeEndpointSpec, GqlType,
    PlannedTypePropertyConstraint, PlannedTypePropertyDef, ProcedureMetadata, ProcedureMutability,
    ProcedureSignature, ProcedureTier, SourceSpan,
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
        CatalogOp::CreateGraph { .. } | CatalogOp::DropGraph { .. } => {
            Err(ExecutorError::ImplementationDefined {
                detail: GRAPH_LEVEL_CATALOG_DETAIL,
            })
        }
        CatalogOp::CreateNodeType {
            label,
            or_replace,
            if_not_exists,
            extends,
            properties,
            validation_mode,
            span,
        } => {
            reject_create_flags(*or_replace, *if_not_exists, extends, *validation_mode)?;
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            let indexes = inline_index_specs(properties)?;
            validate_index_name_collisions(*label, &indexes, ctx.snapshot())?;
            let properties = property_defs(properties)?;
            {
                let mut mutator =
                    ctx.mutator_with_span("catalog op invoked without write transaction", *span)?;
                mutator
                    .create_node_type(*label, LabelSet::single(*label), properties)
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
            endpoints,
            properties,
            validation_mode,
            span,
        } => {
            reject_create_flags(*or_replace, *if_not_exists, &None, *validation_mode)?;
            ctx.ensure_write_txn("catalog op invoked without write transaction", *span)?;
            let endpoints = endpoints
                .as_ref()
                .ok_or(ExecutorError::ImplementationDefined {
                    detail: "create edge type without endpoints requires open graph (GG01)",
                })?;
            let graph_type = closed_graph_type(ctx.snapshot(), *span)?;
            let (source, target) = resolve_endpoints(endpoints, &graph_type, *span)?;
            let properties = property_defs(properties)?;
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .create_edge_type(*label, *label, source, target, properties)
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::DropNodeType {
            label,
            if_exists,
            span,
        } => {
            reject_if_exists(*if_exists)?;
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .drop_node_type(*label)
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::DropEdgeType {
            label,
            if_exists,
            span,
        } => {
            reject_if_exists(*if_exists)?;
            ctx.mutator_with_span("catalog op invoked without write transaction", *span)?
                .drop_edge_type(*label)
                .map_err(|source| catalog_graph_error(source, *span))?;
            Ok(table)
        }
        CatalogOp::ShowNodeTypes(_) => show_node_types(ctx),
        CatalogOp::ShowEdgeTypes(_) => show_edge_types(ctx),
        CatalogOp::ShowIndexes(_) => show_indexes(ctx),
        CatalogOp::ShowProcedures(_) => show_procedures(ctx),
    }
}

fn reject_create_flags(
    or_replace: bool,
    if_not_exists: bool,
    extends: &Option<IStr>,
    validation_mode: Option<crate::ValidationMode>,
) -> Result<(), ExecutorError> {
    if or_replace {
        return Err(ExecutorError::ImplementationDefined {
            detail: "OR REPLACE not implemented for catalog DDL",
        });
    }
    if if_not_exists {
        return Err(ExecutorError::ImplementationDefined {
            detail: "IF NOT EXISTS not implemented for catalog DDL",
        });
    }
    if extends.is_some() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "EXTENDS not implemented for catalog DDL",
        });
    }
    if validation_mode.is_some() {
        return Err(ExecutorError::ImplementationDefined {
            detail: "VALIDATION_MODE not implemented for catalog DDL",
        });
    }
    Ok(())
}

fn reject_if_exists(if_exists: bool) -> Result<(), ExecutorError> {
    if if_exists {
        return Err(ExecutorError::ImplementationDefined {
            detail: "IF EXISTS not implemented for catalog DDL",
        });
    }
    Ok(())
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

fn resolve_endpoints(
    spec: &EdgeEndpointSpec,
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<(u32, u32), ExecutorError> {
    Ok((
        single_endpoint(&spec.from_labels, graph_type, span)?,
        single_endpoint(&spec.to_labels, graph_type, span)?,
    ))
}

fn single_endpoint(
    labels: &[IStr],
    graph_type: &GraphTypeDef,
    span: SourceSpan,
) -> Result<u32, ExecutorError> {
    let [label] = labels else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "multi-label edge endpoint not supported (Phase A: single label per endpoint)",
        });
    };
    graph_type
        .find_node_type_index(&LabelSet::single(*label))
        .ok_or_else(|| ExecutorError::GraphTypeViolation {
            message: format!("edge endpoint references unknown node type label {label}"),
            span,
        })
}

fn property_defs(
    properties: &[PlannedTypePropertyDef],
) -> Result<Vec<PropertyTypeDef>, ExecutorError> {
    properties.iter().map(property_def).collect()
}

fn property_def(property: &PlannedTypePropertyDef) -> Result<PropertyTypeDef, ExecutorError> {
    let mut required = false;
    for constraint in &property.constraints {
        match constraint {
            PlannedTypePropertyConstraint::NotNull(_) => required = true,
            PlannedTypePropertyConstraint::Default(_, _)
            | PlannedTypePropertyConstraint::Immutable(_)
            | PlannedTypePropertyConstraint::Unique(_)
            | PlannedTypePropertyConstraint::Searchable(_)
            | PlannedTypePropertyConstraint::Dictionary(_)
            | PlannedTypePropertyConstraint::Fill(_, _)
            | PlannedTypePropertyConstraint::Interval(_, _)
            | PlannedTypePropertyConstraint::Encoding(_, _) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "type property constraint not implemented (Phase A: NOT NULL only)",
                });
            }
            PlannedTypePropertyConstraint::Indexed { .. } => {}
        }
    }
    Ok(PropertyTypeDef {
        name: property.name,
        value_type: gql_type_to_property_value_type(&property.gql_type)?,
        required,
    })
}

struct InlineIndexSpec {
    property: IStr,
    kind: TypedIndexKind,
    name: Option<IStr>,
    span: SourceSpan,
}
fn inline_index_specs(
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
fn validate_index_name_collisions(
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
fn gql_type_to_index_kind(
    gql_type: &GqlType,
    span: SourceSpan,
) -> Result<TypedIndexKind, ExecutorError> {
    match gql_type {
        GqlType::String => Ok(TypedIndexKind::String),
        GqlType::Integer
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::SmallInt
        | GqlType::BigInt => Ok(TypedIndexKind::I64),
        GqlType::Float | GqlType::Float64 => Ok(TypedIndexKind::F64),
        GqlType::Date => Ok(TypedIndexKind::Date),
        GqlType::LocalDateTime => Ok(TypedIndexKind::LocalDateTime),
        _ => Err(ExecutorError::FeatureNotInV1_1 {
            feature: "inline INDEXED for this GQL type",
            span,
        }),
    }
}

fn gql_type_to_property_value_type(gql_type: &GqlType) -> Result<PropertyValueType, ExecutorError> {
    Ok(match gql_type {
        GqlType::String => PropertyValueType::String,
        GqlType::Boolean => PropertyValueType::Bool,
        GqlType::Integer
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::SmallInt
        | GqlType::BigInt => PropertyValueType::Int,
        GqlType::Uint8 | GqlType::Uint16 | GqlType::Uint32 | GqlType::Uint64 => {
            PropertyValueType::Uint
        }
        GqlType::Int128 => PropertyValueType::Int128,
        GqlType::Uint128 => PropertyValueType::Uint128,
        GqlType::Float | GqlType::Float64 => PropertyValueType::Float,
        GqlType::Float32 => PropertyValueType::Float32,
        GqlType::Decimal => PropertyValueType::Decimal,
        GqlType::Bytes | GqlType::Binary | GqlType::VarBinary => PropertyValueType::Bytes,
        GqlType::ZonedDateTime => PropertyValueType::ZonedDateTime,
        GqlType::LocalDateTime => PropertyValueType::LocalDateTime,
        GqlType::Date => PropertyValueType::Date,
        GqlType::ZonedTime => PropertyValueType::ZonedTime,
        GqlType::LocalTime => PropertyValueType::LocalTime,
        GqlType::Duration => PropertyValueType::Duration,
        GqlType::Path => PropertyValueType::Path,
        GqlType::GraphRef => PropertyValueType::GraphRef,
        GqlType::NodeRef => PropertyValueType::NodeRef,
        GqlType::EdgeRef => PropertyValueType::EdgeRef,
        GqlType::TableRef => PropertyValueType::TableRef,
        GqlType::Null => PropertyValueType::Null,
        GqlType::Record(_) | GqlType::List(_) | GqlType::Nothing => {
            return Err(ExecutorError::ImplementationDefined {
                detail: "type property GQL type not supported as property value type (Phase A)",
            });
        }
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

fn render_index_name(label: IStr, property: IStr, explicit: Option<IStr>) -> String {
    explicit
        .map(|name| name.as_str().to_owned())
        .unwrap_or_else(|| format!("{}_{}_idx", label.as_str(), property.as_str()))
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

fn intern_runtime(value: &str) -> Result<IStr, ExecutorError> {
    intern_with_admission(value)
        .map(|(value, _was_new)| value)
        .map_err(|_err| ExecutorError::ImplementationDefined {
            detail: "interner cap exhausted during catalog rendering",
        })
}

fn render_index_kind(kind: TypedIndexKind) -> &'static str {
    match kind {
        TypedIndexKind::I64 => "i64",
        TypedIndexKind::F64 => "f64",
        TypedIndexKind::String => "string",
        TypedIndexKind::Date => "date",
        TypedIndexKind::LocalDateTime => "local_datetime",
        TypedIndexKind::Uuid => "uuid",
    }
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
    let source = render_endpoint(graph_type, edge_type.source_node_type);
    let target = render_endpoint(graph_type, edge_type.target_node_type);
    let properties = render_properties(&edge_type.properties);
    if properties.is_empty() {
        format!(
            "CREATE EDGE TYPE :{} (FROM {} TO {})",
            edge_type.label, source, target
        )
    } else {
        format!(
            "CREATE EDGE TYPE :{} (FROM {} TO {}, {})",
            edge_type.label, source, target, properties
        )
    }
}

fn render_endpoint(graph_type: &GraphTypeDef, index: u32) -> String {
    render_node_label_set(&graph_type.node_types[index as usize].key_labels)
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
            format!(
                "{} :: {}{}",
                property.name,
                render_property_value_type(property.value_type),
                nullability
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_property_value_type(value_type: PropertyValueType) -> &'static str {
    match value_type {
        PropertyValueType::Bool => "BOOLEAN",
        PropertyValueType::Int => "INTEGER",
        PropertyValueType::Uint => "UINT64",
        PropertyValueType::Int128 => "INT128",
        PropertyValueType::Uint128 => "UINT128",
        PropertyValueType::Float => "FLOAT",
        PropertyValueType::Float32 => "FLOAT32",
        PropertyValueType::Decimal => "DECIMAL",
        PropertyValueType::String => "STRING",
        PropertyValueType::Bytes => "BYTES",
        PropertyValueType::List => "LIST",
        PropertyValueType::Record | PropertyValueType::RecordTyped => "RECORD",
        PropertyValueType::Path => "PATH",
        PropertyValueType::NodeRef => "NODE",
        PropertyValueType::EdgeRef => "EDGE",
        PropertyValueType::GraphRef => "GRAPH",
        PropertyValueType::TableRef => "TABLE",
        PropertyValueType::ZonedDateTime => "ZONED DATETIME",
        PropertyValueType::LocalDateTime => "LOCAL DATETIME",
        PropertyValueType::Date => "DATE",
        PropertyValueType::ZonedTime => "ZONED TIME",
        PropertyValueType::LocalTime => "LOCAL TIME",
        PropertyValueType::Duration => "DURATION",
        PropertyValueType::Null => "NULL",
        PropertyValueType::Uuid => "UUID",
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
