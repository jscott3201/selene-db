//! Property-definition helpers for catalog DDL.

use selene_core::PropertyValueType;
use selene_graph::PropertyTypeDef;

use crate::{ExecutorError, GqlType, PlannedTypePropertyConstraint, PlannedTypePropertyDef};

pub(super) fn property_defs(
    properties: &[PlannedTypePropertyDef],
    allow_inline_indexed: bool,
) -> Result<Vec<PropertyTypeDef>, ExecutorError> {
    properties
        .iter()
        .map(|property| property_def(property, allow_inline_indexed))
        .collect()
}

fn property_def(
    property: &PlannedTypePropertyDef,
    allow_inline_indexed: bool,
) -> Result<PropertyTypeDef, ExecutorError> {
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
            PlannedTypePropertyConstraint::Indexed { span, .. } if !allow_inline_indexed => {
                return Err(ExecutorError::FeatureNotInV1_1 {
                    feature: "inline INDEXED on edge properties",
                    span: *span,
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

pub(super) fn render_property_value_type(value_type: PropertyValueType) -> &'static str {
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
