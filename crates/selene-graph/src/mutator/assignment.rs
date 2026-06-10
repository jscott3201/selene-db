//! Store-assignment conversion for closed graph property writes.

use std::sync::Arc;

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use selene_core::{
    ByteStringType, CharacterStringCoercionError, CharacterStringType, DbString, DecimalType,
    LabelSet, NodeId, PropertyDiff, PropertyMap, PropertyValueType, Value,
    coerce_character_string_to_type, db_string, round_decimal_to_type,
};

use crate::error::{GraphError, GraphResult, StoreAssignmentError, StoreAssignmentException};
use crate::graph_types::{
    GraphTypeDef, PropertyElementType, PropertyTypeDef, RecordFieldType, RecordFieldTypes,
};

pub(crate) fn coerce_node_properties(
    graph: &crate::SeleneGraph,
    labels: &LabelSet,
    props: &mut PropertyMap,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(node_type) = graph_type.find_node_type(labels) else {
        return Ok(());
    };
    coerce_property_map(&node_type.properties, props)
}

pub(crate) fn coerce_edge_properties(
    graph: &crate::SeleneGraph,
    label: DbString,
    source: NodeId,
    target: NodeId,
    props: &mut PropertyMap,
) -> GraphResult<()> {
    let Some(edge_type) = edge_type(graph, label, source, target) else {
        return Ok(());
    };
    coerce_property_map(&edge_type.properties, props)
}

pub(crate) fn coerce_node_property_diff(
    graph: &crate::SeleneGraph,
    labels: &LabelSet,
    diff: &mut PropertyDiff,
) -> GraphResult<()> {
    let Some(graph_type) = graph.meta.bound_type.as_deref() else {
        return Ok(());
    };
    let Some(node_type) = graph_type.find_node_type(labels) else {
        return Ok(());
    };
    coerce_property_diff(&node_type.properties, diff)
}

pub(crate) fn coerce_edge_property_diff(
    graph: &crate::SeleneGraph,
    edge: selene_core::EdgeId,
    diff: &mut PropertyDiff,
) -> GraphResult<()> {
    let Some(label) = graph.edge_label(edge).cloned() else {
        return Ok(());
    };
    let Some((source, target)) = graph.edge_endpoints(edge) else {
        return Ok(());
    };
    let Some(edge_type) = edge_type(graph, label, source, target) else {
        return Ok(());
    };
    coerce_property_diff(&edge_type.properties, diff)
}

fn edge_type(
    graph: &crate::SeleneGraph,
    label: DbString,
    source: NodeId,
    target: NodeId,
) -> Option<&crate::graph_types::EdgeTypeDef> {
    let graph_type = graph.meta.bound_type.as_deref()?;
    let source_type = node_type_index_for_node(graph, graph_type, source)?;
    let target_type = node_type_index_for_node(graph, graph_type, target)?;
    graph_type.find_edge_type(label, source_type, target_type)
}

fn node_type_index_for_node(
    graph: &crate::SeleneGraph,
    graph_type: &GraphTypeDef,
    node: NodeId,
) -> Option<u32> {
    let labels = graph.node_labels(node)?;
    graph_type.find_node_type_index(labels)
}

fn coerce_property_map(
    declarations: &[PropertyTypeDef],
    props: &mut PropertyMap,
) -> GraphResult<()> {
    for declaration in declarations {
        let Some(value) = props.get(&declaration.name).cloned() else {
            continue;
        };
        let value = coerce_property_value(declaration, value)?;
        props.set(declaration.name.clone(), value)?;
    }
    Ok(())
}

fn coerce_property_diff(
    declarations: &[PropertyTypeDef],
    diff: &mut PropertyDiff,
) -> GraphResult<()> {
    for (key, value) in &mut diff.set {
        let Some(declaration) = declarations.iter().find(|decl| decl.name == *key) else {
            continue;
        };
        *value = coerce_property_value(declaration, value.clone())?;
    }
    Ok(())
}

fn coerce_property_value(declaration: &PropertyTypeDef, value: Value) -> GraphResult<Value> {
    if matches!(value, Value::Null) {
        return Ok(value);
    }
    match declaration.value_type {
        PropertyValueType::Decimal => {
            coerce_decimal_property(value, declaration.decimal_type, declaration.name.clone())
        }
        PropertyValueType::String => match declaration.character_string_type {
            Some(target) => coerce_character_string(value, target, declaration.name.clone()),
            None => Ok(value),
        },
        PropertyValueType::Bytes => match declaration.byte_string_type {
            Some(target) => coerce_byte_string(value, target, declaration.name.clone()),
            None => Ok(value),
        },
        PropertyValueType::List => match (&declaration.list_element_type, value) {
            (Some(element_type), Value::List(values)) => values
                .into_iter()
                .map(|value| coerce_element_value(element_type, value, &declaration.name))
                .collect::<GraphResult<Vec<_>>>()
                .map(Value::List),
            (_, value) => Ok(value),
        },
        PropertyValueType::Record | PropertyValueType::RecordTyped => {
            match (&declaration.record_field_types, value) {
                (Some(fields), value @ (Value::Record(_) | Value::RecordTyped(_))) => {
                    coerce_record_value(fields, value, &declaration.name)
                }
                (_, value) => Ok(value),
            }
        }
        _ => Ok(value),
    }
}

fn coerce_element_value(
    element_type: &PropertyElementType,
    value: Value,
    property: &DbString,
) -> GraphResult<Value> {
    if matches!(value, Value::Null) {
        return Ok(value);
    }
    match element_type {
        PropertyElementType::NotNull(inner) => coerce_element_value(inner, value, property),
        PropertyElementType::Scalar(PropertyValueType::Decimal) => {
            coerce_decimal_property(value, None, property.clone())
        }
        PropertyElementType::Scalar(_) => Ok(value),
        PropertyElementType::CharacterString(target) => {
            coerce_character_string(value, *target, property.clone())
        }
        PropertyElementType::Decimal(target) => {
            coerce_decimal_property(value, Some(*target), property.clone())
        }
        PropertyElementType::ByteString(target) => {
            coerce_byte_string(value, *target, property.clone())
        }
        PropertyElementType::List(inner) => match value {
            Value::List(values) => values
                .into_iter()
                .map(|value| coerce_element_value(inner, value, property))
                .collect::<GraphResult<Vec<_>>>()
                .map(Value::List),
            value => Ok(value),
        },
    }
}

fn coerce_record_field_value(
    field_type: &RecordFieldType,
    value: Value,
    property: &DbString,
) -> GraphResult<Value> {
    if matches!(value, Value::Null) {
        return Ok(value);
    }
    match field_type {
        RecordFieldType::NotNull(inner) => coerce_record_field_value(inner, value, property),
        RecordFieldType::Scalar(PropertyValueType::Decimal) => {
            coerce_decimal_property(value, None, property.clone())
        }
        RecordFieldType::Scalar(_) | RecordFieldType::OpenRecord => Ok(value),
        RecordFieldType::CharacterString(target) => {
            coerce_character_string(value, *target, property.clone())
        }
        RecordFieldType::Decimal(target) => {
            coerce_decimal_property(value, Some(*target), property.clone())
        }
        RecordFieldType::ByteString(target) => coerce_byte_string(value, *target, property.clone()),
        RecordFieldType::List(inner) => match value {
            Value::List(values) => values
                .into_iter()
                .map(|value| coerce_record_field_value(inner, value, property))
                .collect::<GraphResult<Vec<_>>>()
                .map(Value::List),
            value => Ok(value),
        },
        RecordFieldType::Record(fields) => coerce_record_value(fields, value, property),
    }
}

fn coerce_record_value(
    fields: &RecordFieldTypes,
    value: Value,
    property: &DbString,
) -> GraphResult<Value> {
    match value {
        Value::Record(record) => match *record {
            selene_core::Record::Open(mut values) => {
                for (name, field_value) in &mut values {
                    let Some(field) = fields.0.iter().find(|field| field.name == *name) else {
                        continue;
                    };
                    *field_value = coerce_record_field_value(
                        &field.field_type,
                        field_value.clone(),
                        property,
                    )?;
                }
                Ok(Value::Record(Box::new(selene_core::Record::Open(values))))
            }
            other => Ok(Value::Record(Box::new(other))),
        },
        Value::RecordTyped(mut record) => {
            for (field, slot) in fields.0.iter().zip(record.values.iter_mut()) {
                let Some(value) = slot.take() else {
                    continue;
                };
                *slot = Some(coerce_record_field_value(
                    &field.field_type,
                    value,
                    property,
                )?);
            }
            Ok(Value::RecordTyped(record))
        }
        value => Ok(value),
    }
}

fn coerce_character_string(
    value: Value,
    target: CharacterStringType,
    property: DbString,
) -> GraphResult<Value> {
    let Value::String(value) = value else {
        return Ok(value);
    };
    // The shared core coercion keeps store assignment on the same IV023
    // space-only truncation policy as character CAST and DEFAULT descriptor
    // coercion.
    let coerced = coerce_character_string_to_type(value.as_str(), target).map_err(|err| {
        let detail = match err {
            CharacterStringCoercionError::SourceLengthOverflow => {
                "character string source length exceeds supported range"
            }
            CharacterStringCoercionError::TargetMinOverflow => {
                "character string target minimum length exceeds supported range"
            }
            CharacterStringCoercionError::TargetMaxOverflow => {
                "character string target maximum length exceeds supported range"
            }
            CharacterStringCoercionError::NonSpaceTruncation => {
                "character string assignment would truncate non-space trailing characters"
            }
        };
        assignment_error(
            property,
            StoreAssignmentException::StringDataRightTruncation,
            detail,
        )
    })?;
    db_string(&coerced)
        .map(Value::String)
        .map_err(GraphError::Core)
}

fn coerce_byte_string(
    value: Value,
    target: ByteStringType,
    property: DbString,
) -> GraphResult<Value> {
    let Value::Bytes(value) = value else {
        return Ok(value);
    };
    let len = u64::try_from(value.len()).map_err(|_| {
        assignment_error(
            property.clone(),
            StoreAssignmentException::StringDataRightTruncation,
            "byte string source length exceeds supported range",
        )
    })?;
    let mut bytes = value.as_ref().to_vec();
    if len > target.max_len {
        let max_len = usize::try_from(target.max_len).map_err(|_| {
            assignment_error(
                property.clone(),
                StoreAssignmentException::StringDataRightTruncation,
                "byte string target maximum length exceeds supported range",
            )
        })?;
        if bytes[max_len..].iter().any(|byte| *byte != 0) {
            return Err(assignment_error(
                property,
                StoreAssignmentException::StringDataRightTruncation,
                "byte string assignment would truncate non-zero trailing bytes",
            ));
        }
        bytes.truncate(max_len);
    } else if len < target.min_len {
        let min_len = usize::try_from(target.min_len).map_err(|_| {
            assignment_error(
                property.clone(),
                StoreAssignmentException::StringDataRightTruncation,
                "byte string target minimum length exceeds supported range",
            )
        })?;
        bytes.resize(min_len, 0);
    }
    Ok(Value::Bytes(Arc::from(bytes)))
}

fn coerce_decimal_property(
    value: Value,
    target: Option<DecimalType>,
    property: DbString,
) -> GraphResult<Value> {
    let decimal = numeric_to_decimal(value, property.clone())?;
    let Some(target) = target else {
        return Ok(Value::Decimal(decimal));
    };
    round_decimal_to_type(decimal, target)
        .map(Value::Decimal)
        .ok_or_else(|| {
            assignment_error(
                property,
                StoreAssignmentException::NumericValueOutOfRange,
                "numeric assignment cannot be represented by declared DECIMAL precision/scale",
            )
        })
}

fn numeric_to_decimal(value: Value, property: DbString) -> GraphResult<Decimal> {
    match value {
        Value::Int(value) => Ok(Decimal::from(value)),
        Value::Uint(value) => Ok(Decimal::from(value)),
        Value::Int128(value) => Decimal::try_from_i128_with_scale(value, 0).map_err(|_| {
            assignment_error(
                property,
                StoreAssignmentException::NumericValueOutOfRange,
                "INT128 assignment exceeds DECIMAL range",
            )
        }),
        Value::Uint128(value) => Decimal::from_u128(value).ok_or_else(|| {
            assignment_error(
                property,
                StoreAssignmentException::NumericValueOutOfRange,
                "UINT128 assignment exceeds DECIMAL range",
            )
        }),
        Value::Float(value) => Decimal::from_f64(value).ok_or_else(|| {
            assignment_error(
                property,
                StoreAssignmentException::NumericValueOutOfRange,
                "FLOAT assignment has no DECIMAL image",
            )
        }),
        Value::Float32(value) => Decimal::from_f32(value).ok_or_else(|| {
            assignment_error(
                property,
                StoreAssignmentException::NumericValueOutOfRange,
                "FLOAT32 assignment has no DECIMAL image",
            )
        }),
        Value::Decimal(value) => Ok(value),
        value => Err(assignment_error(
            property,
            StoreAssignmentException::NumericValueOutOfRange,
            format!("{} is not assignable to DECIMAL", value.variant_name()),
        )),
    }
}

fn assignment_error(
    property: DbString,
    exception: StoreAssignmentException,
    reason: impl Into<String>,
) -> GraphError {
    GraphError::StoreAssignment(Box::new(StoreAssignmentError {
        property,
        exception,
        reason: reason.into(),
    }))
}
