//! Property-definition helpers for catalog DDL.

mod defaults;

pub(super) use defaults::render_property_default_value;
use defaults::{coerce_property_default_value, property_default_value, validate_default_value};
use selene_core::PropertyValueType;
use selene_graph::{
    PropertyElementType, PropertyTypeDef, RecordFieldType, RecordFieldTypeDef, RecordFieldTypes,
};

use crate::{
    ExecutorError, GqlType, PlannedTypePropertyConstraint, PlannedTypePropertyDef, RecordType,
    parser::MAX_NESTING_DEPTH,
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
    let (value_type, list_element_type, record_field_types) =
        gql_type_to_property_value_type(&property.gql_type)?;
    let mut required = false;
    let mut default = None;
    let mut default_span = property.span;
    let mut immutable = false;
    let mut unique = false;
    for constraint in &property.constraints {
        match constraint {
            PlannedTypePropertyConstraint::NotNull(_) => required = true,
            PlannedTypePropertyConstraint::Default(project, span) => {
                default = Some(property_default_value(
                    project,
                    value_type,
                    list_element_type.as_ref(),
                    record_field_types.as_ref(),
                    *span,
                )?);
                default_span = *span;
            }
            PlannedTypePropertyConstraint::Immutable(_) => immutable = true,
            PlannedTypePropertyConstraint::Unique(_) => unique = true,
            PlannedTypePropertyConstraint::Indexed { span, .. } if !allow_inline_indexed => {
                return Err(ExecutorError::FeatureNotSupportedYet {
                    feature: "inline INDEXED on edge properties",
                    span: *span,
                });
            }
            PlannedTypePropertyConstraint::Indexed { .. } => {}
        }
    }
    let default = default
        .map(|default| coerce_property_default_value(value_type, default, default_span))
        .transpose()?;
    if let Some(default) = &default {
        validate_default_value(
            property.name.clone(),
            value_type,
            list_element_type.as_ref(),
            record_field_types.as_ref(),
            required,
            default,
            default_span,
        )?;
    }
    Ok(PropertyTypeDef {
        name: property.name.clone(),
        value_type,
        list_element_type,
        required,
        default,
        immutable,
        unique,
        record_field_types,
    })
}

type LoweredPropertyType = (
    PropertyValueType,
    Option<PropertyElementType>,
    Option<RecordFieldTypes>,
);

fn gql_type_to_property_value_type(
    gql_type: &GqlType,
) -> Result<LoweredPropertyType, ExecutorError> {
    match gql_type {
        GqlType::List(inner) => {
            let element_type = gql_type_to_property_element_type(inner, 1)?;
            Ok((PropertyValueType::List, Some(element_type), None))
        }
        // A top-level `RECORD` declaration lowers to the RecordTyped tag. A closed
        // `RECORD { .. }` carries its field-type descriptor; an open/bare `RECORD` carries
        // `None` (permissive, accepts any record value).
        GqlType::Record(record_type) => {
            let record_field_types = gql_record_to_record_field_types(record_type, 1)?;
            Ok((PropertyValueType::RecordTyped, None, record_field_types))
        }
        _ => gql_type_to_scalar_property_value_type(gql_type)
            .map(|value_type| (value_type, None, None)),
    }
}

/// Lower a record TYPE into the catalog descriptor. `RecordType::Open` (bare `RECORD`)
/// yields `None` (permissive); `RecordType::Closed` yields the field-type list.
///
/// The grammar does not (yet) capture a per-field `<not null>`, so every declared field is
/// required-present per ISO 39075:2024 §4.15.4 (a closed record value has the same
/// field-name set as the descriptor). The descriptor's optional-field capability is latent
/// for a future `NOT NULL` field-type extension.
fn gql_record_to_record_field_types(
    record_type: &RecordType,
    depth: u32,
) -> Result<Option<RecordFieldTypes>, ExecutorError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ExecutorError::ImplementationDefined {
            detail: "nested RECORD property type exceeds parser nesting limit",
        });
    }
    match record_type {
        RecordType::Open => Ok(None),
        RecordType::Closed(fields) => {
            let defs = fields
                .iter()
                .map(|(name, gql_type)| {
                    Ok(RecordFieldTypeDef {
                        name: name.clone(),
                        field_type: gql_type_to_record_field_type(gql_type, depth)?,
                        required: true,
                    })
                })
                .collect::<Result<Vec<_>, ExecutorError>>()?;
            Ok(Some(RecordFieldTypes(defs)))
        }
    }
}

fn gql_type_to_record_field_type(
    gql_type: &GqlType,
    depth: u32,
) -> Result<RecordFieldType, ExecutorError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ExecutorError::ImplementationDefined {
            detail: "nested RECORD field type exceeds parser nesting limit",
        });
    }
    match gql_type {
        GqlType::List(inner) => Ok(RecordFieldType::List(Box::new(
            gql_type_to_record_field_type(inner, depth + 1)?,
        ))),
        GqlType::Record(record_type) => {
            match gql_record_to_record_field_types(record_type, depth + 1)? {
                Some(fields) => Ok(RecordFieldType::Record(Box::new(fields))),
                None => Ok(RecordFieldType::OpenRecord),
            }
        }
        _ => gql_type_to_scalar_property_value_type(gql_type).map(RecordFieldType::Scalar),
    }
}

fn gql_type_to_property_element_type(
    gql_type: &GqlType,
    depth: u32,
) -> Result<PropertyElementType, ExecutorError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ExecutorError::ImplementationDefined {
            detail: "nested LIST property type exceeds parser nesting limit",
        });
    }
    match gql_type {
        GqlType::List(inner) => Ok(PropertyElementType::List(Box::new(
            gql_type_to_property_element_type(inner, depth + 1)?,
        ))),
        _ => gql_type_to_scalar_property_value_type(gql_type).map(PropertyElementType::Scalar),
    }
}

fn gql_type_to_scalar_property_value_type(
    gql_type: &GqlType,
) -> Result<PropertyValueType, ExecutorError> {
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
        GqlType::Bytes => PropertyValueType::Bytes,
        GqlType::Uuid => PropertyValueType::Uuid,
        GqlType::Json => PropertyValueType::Json,
        GqlType::ZonedDateTime => PropertyValueType::ZonedDateTime,
        GqlType::LocalDateTime => PropertyValueType::LocalDateTime,
        GqlType::Date => PropertyValueType::Date,
        GqlType::ZonedTime => PropertyValueType::ZonedTime,
        GqlType::LocalTime => PropertyValueType::LocalTime,
        GqlType::Duration => PropertyValueType::Duration,
        GqlType::DurationYearToMonth => PropertyValueType::DurationYearToMonth,
        GqlType::DurationDayToSecond => PropertyValueType::DurationDayToSecond,
        GqlType::Vector => PropertyValueType::Vector,
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

pub(super) fn render_property_value_type(
    value_type: PropertyValueType,
    list_element_type: Option<&PropertyElementType>,
    record_field_types: Option<&RecordFieldTypes>,
) -> String {
    if value_type == PropertyValueType::List
        && let Some(element_type) = list_element_type
    {
        return format!("LIST<{}>", render_property_element_type(element_type));
    }
    // A closed RECORD carries its field-type descriptor; render the structure
    // so SHOW round-trips `RECORD { name :: TYPE, ... }` rather than a bare
    // `RECORD` that loses the open-vs-closed distinction. An open/bare RECORD
    // (no descriptor) stays `RECORD`.
    if value_type == PropertyValueType::RecordTyped
        && let Some(fields) = record_field_types
    {
        return render_record_field_types(fields);
    }
    scalar_property_value_type_name(value_type).to_owned()
}

fn render_record_field_types(fields: &RecordFieldTypes) -> String {
    let rendered = fields
        .0
        .iter()
        .map(|field| {
            format!(
                "{} :: {}",
                field.name,
                render_record_field_type(&field.field_type)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("RECORD {{ {rendered} }}")
}

fn render_record_field_type(field_type: &RecordFieldType) -> String {
    match field_type {
        RecordFieldType::Scalar(value_type) => {
            scalar_property_value_type_name(*value_type).to_owned()
        }
        RecordFieldType::List(inner) => format!("LIST<{}>", render_record_field_type(inner)),
        RecordFieldType::OpenRecord => "RECORD".to_owned(),
        RecordFieldType::Record(inner) => render_record_field_types(inner),
        _ => "<unsupported-record-field>".to_owned(),
    }
}

fn render_property_element_type(element_type: &PropertyElementType) -> String {
    match element_type {
        PropertyElementType::Scalar(value_type) => {
            scalar_property_value_type_name(*value_type).to_owned()
        }
        PropertyElementType::List(inner) => {
            format!("LIST<{}>", render_property_element_type(inner))
        }
        _ => "<unsupported-element>".to_owned(),
    }
}

fn scalar_property_value_type_name(value_type: PropertyValueType) -> &'static str {
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
        PropertyValueType::DurationYearToMonth => "DURATION (YEAR TO MONTH)",
        PropertyValueType::DurationDayToSecond => "DURATION (DAY TO SECOND)",
        PropertyValueType::Null => "NULL",
        PropertyValueType::Uuid => "UUID",
        PropertyValueType::Vector => "VECTOR",
        PropertyValueType::Json => "JSON",
    }
}
