//! Property-definition helpers for catalog DDL.

use selene_core::PropertyValueType;
use selene_graph::{
    PropertyDefaultValue, PropertyElementType, PropertyTypeDef, RecordFieldType,
    RecordFieldTypeDef, RecordFieldTypes,
};

use crate::{
    DataExceptionSubclass, ExecutorError, GqlType, Literal, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, ProjectExpr, RecordType, ValueExpr, parser::MAX_NESTING_DEPTH,
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
            // UNIQUE is an ISO-relevant property constraint (ISO/IEC 39075:2024
            // §18) but enforcement of property uniqueness is not yet implemented;
            // surface it as an honest capability-gap deferral (42N01) rather than
            // a generic internal error, mirroring the inline-INDEXED-on-edge path.
            PlannedTypePropertyConstraint::Unique(span) => {
                return Err(ExecutorError::FeatureNotSupportedYet {
                    feature: "UNIQUE property constraint",
                    span: *span,
                });
            }
            PlannedTypePropertyConstraint::Indexed { span, .. } if !allow_inline_indexed => {
                return Err(ExecutorError::FeatureNotSupportedYet {
                    feature: "inline INDEXED on edge properties",
                    span: *span,
                });
            }
            PlannedTypePropertyConstraint::Indexed { .. } => {}
        }
    }
    let (value_type, list_element_type, record_field_types) =
        gql_type_to_property_value_type(&property.gql_type)?;
    if let Some(default) = &default {
        validate_default_value(
            property.name.clone(),
            value_type,
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
        record_field_types,
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
        Literal::String(value, _) => Ok(PropertyDefaultValue::String(value.clone())),
        Literal::Float(_, _) => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "floating-point DEFAULT literals",
            span,
        }),
        Literal::Uuid(_, _) => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "UUID DEFAULT literals",
            span,
        }),
        Literal::ZonedDateTime(_, _)
        | Literal::LocalDateTime(_, _)
        | Literal::Date(_, _)
        | Literal::ZonedTime(_, _)
        | Literal::LocalTime(_, _)
        | Literal::Duration(_, _) => Err(ExecutorError::FeatureNotSupportedYet {
            feature: "temporal DEFAULT literals",
            span,
        }),
    }
}

fn validate_default_value(
    property: selene_core::DbString,
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
    property: selene_core::DbString,
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
                // An open/bare nested `RECORD` field has no closed structure to persist; defer
                // it (consistent with deferring top-level `LIST<RECORD>`). Declare its fields.
                None => Err(ExecutorError::ImplementationDefined {
                    detail: "open/bare RECORD is not supported as a nested record field type; declare its fields",
                }),
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
        GqlType::ZonedDateTime => PropertyValueType::ZonedDateTime,
        GqlType::LocalDateTime => PropertyValueType::LocalDateTime,
        GqlType::Date => PropertyValueType::Date,
        GqlType::ZonedTime => PropertyValueType::ZonedTime,
        GqlType::LocalTime => PropertyValueType::LocalTime,
        GqlType::Duration => PropertyValueType::Duration,
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
        PropertyValueType::Null => "NULL",
        PropertyValueType::Uuid => "UUID",
        PropertyValueType::Vector => "VECTOR",
    }
}
