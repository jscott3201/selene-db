//! Property-definition helpers for catalog DDL.

use selene_core::PropertyValueType;
use selene_graph::{PropertyDefaultValue, PropertyTypeDef};

use crate::{
    DataExceptionSubclass, ExecutorError, GqlType, Literal, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, ProjectExpr, ValueExpr,
};

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
    let mut default = None;
    let mut default_span = property.span;
    let mut immutable = false;
    for constraint in &property.constraints {
        match constraint {
            PlannedTypePropertyConstraint::NotNull(_) => required = true,
            PlannedTypePropertyConstraint::Default(project, span) => {
                default = Some(property_default_value(project, *span)?);
                default_span = *span;
            }
            PlannedTypePropertyConstraint::Immutable(_) => immutable = true,
            PlannedTypePropertyConstraint::Unique(_)
            | PlannedTypePropertyConstraint::Searchable(_)
            | PlannedTypePropertyConstraint::Dictionary(_)
            | PlannedTypePropertyConstraint::Fill(_, _)
            | PlannedTypePropertyConstraint::Interval(_, _)
            | PlannedTypePropertyConstraint::Encoding(_, _) => {
                return Err(ExecutorError::ImplementationDefined {
                    detail: "type property constraint not implemented",
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
    let value_type = gql_type_to_property_value_type(&property.gql_type)?;
    if let Some(default) = &default {
        validate_default_value(property.name, value_type, required, default, default_span)?;
    }
    Ok(PropertyTypeDef {
        name: property.name,
        value_type,
        list_element_type: None,
        required,
        default,
        immutable,
    })
}

fn property_default_value(
    project: &ProjectExpr,
    span: crate::SourceSpan,
) -> Result<PropertyDefaultValue, ExecutorError> {
    let ValueExpr::Literal(literal) = &project.expr else {
        return Err(ExecutorError::ImplementationDefined {
            detail: "DEFAULT constraint must lower to a literal expression",
        });
    };
    match literal {
        Literal::Null(_) => Ok(PropertyDefaultValue::Null),
        Literal::Bool(value, _) => Ok(PropertyDefaultValue::Boolean(*value)),
        Literal::Integer(value, _) => Ok(PropertyDefaultValue::Integer(*value)),
        Literal::String(value, _) => Ok(PropertyDefaultValue::String(*value)),
        Literal::Float(_, _) => Err(ExecutorError::FeatureNotInV1_1 {
            feature: "floating-point DEFAULT literals",
            span,
        }),
    }
}

fn validate_default_value(
    property: selene_core::IStr,
    value_type: PropertyValueType,
    required: bool,
    default: &PropertyDefaultValue,
    span: crate::SourceSpan,
) -> Result<(), ExecutorError> {
    let value = default.to_value();
    if matches!(value, selene_core::Value::Null) {
        if required {
            return Err(default_type_error(
                property,
                value_type,
                "Null",
                "NOT NULL property cannot default to NULL",
                span,
            ));
        }
        return Ok(());
    }
    if value_type.matches(&value) {
        return Ok(());
    }
    Err(default_type_error(
        property,
        value_type,
        PropertyValueType::observed_name(&value),
        "DEFAULT literal is not assignable to property type",
        span,
    ))
}

fn default_type_error(
    property: selene_core::IStr,
    expected: PropertyValueType,
    observed: &'static str,
    reason: &'static str,
    span: crate::SourceSpan,
) -> ExecutorError {
    ExecutorError::data_exception(
        DataExceptionSubclass::InvalidValueType,
        format!(
            "{reason}: property {property} expects {}, default is {observed}",
            expected.name()
        ),
        span,
    )
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
