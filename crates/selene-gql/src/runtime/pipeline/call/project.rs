use selene_core::Value;

use crate::{GqlType, PlannedCall, ProcedureError, YieldKind, runtime::ExecutorError};

use super::context::procedure_error;

pub(super) fn project_yield_row(
    call: &PlannedCall,
    output_row: Vec<Value>,
) -> Result<Vec<Value>, ExecutorError> {
    validate_output_row(call, &output_row)?;
    if call.yield_cols.is_empty() {
        return Ok(Vec::new());
    }

    let mut projected = Vec::with_capacity(call.yield_schema.len());
    if call
        .yield_cols
        .iter()
        .any(|item| matches!(item.column, YieldKind::Star))
    {
        projected.extend(output_row.iter().cloned());
    }

    for item in &call.yield_cols {
        let YieldKind::Named(ref name) = item.column else {
            continue;
        };
        let Some(index) = output_column_index(call, name.clone()) else {
            return Err(procedure_error(
                ProcedureError::Internal {
                    detail: "planned yield column not in procedure output schema".to_owned(),
                },
                item.span,
                None,
            ));
        };
        projected.push(output_row[index].clone());
    }
    Ok(projected)
}

fn validate_output_row(call: &PlannedCall, row: &[Value]) -> Result<(), ExecutorError> {
    if row.len() != call.output_schema.columns.len() {
        return Err(procedure_error(
            ProcedureError::Internal {
                detail: "registry returned row with wrong column count".to_owned(),
            },
            call.span,
            None,
        ));
    }

    for (index, (value, column)) in row.iter().zip(&call.output_schema.columns).enumerate() {
        if !matches_gql_type(value, &column.ty) {
            return Err(procedure_error(
                ProcedureError::Internal {
                    detail: format!("registry returned value with wrong type for column {index}"),
                },
                call.span,
                None,
            ));
        }
    }
    Ok(())
}

fn output_column_index(call: &PlannedCall, name: selene_core::IStr) -> Option<usize> {
    call.output_schema
        .columns
        .iter()
        .position(|column| column.name == name)
}

fn matches_gql_type(value: &Value, ty: &GqlType) -> bool {
    if matches!(ty, GqlType::Nothing) {
        return false;
    }
    if matches!(value, Value::Null) {
        return true;
    }

    match ty {
        GqlType::String => matches!(value, Value::String(_) | Value::ExternalString(_)),
        GqlType::Uuid => matches!(value, Value::Uuid(_)),
        GqlType::Boolean => matches!(value, Value::Bool(_)),
        GqlType::Integer
        | GqlType::Int8
        | GqlType::Int16
        | GqlType::Int32
        | GqlType::Int64
        | GqlType::SmallInt
        | GqlType::BigInt => matches!(value, Value::Int(_)),
        GqlType::Int128 => matches!(value, Value::Int128(_)),
        GqlType::Uint8 | GqlType::Uint16 | GqlType::Uint32 | GqlType::Uint64 => {
            matches!(value, Value::Uint(_))
        }
        GqlType::Uint128 => matches!(value, Value::Uint128(_)),
        GqlType::Float | GqlType::Float64 => matches!(value, Value::Float(_)),
        GqlType::Float32 => matches!(value, Value::Float32(_)),
        GqlType::Decimal => matches!(value, Value::Decimal(_)),
        GqlType::Bytes => matches!(value, Value::Bytes(_)),
        GqlType::ZonedDateTime => matches!(value, Value::ZonedDateTime(_)),
        GqlType::LocalDateTime => matches!(value, Value::LocalDateTime(_)),
        GqlType::Date => matches!(value, Value::Date(_)),
        GqlType::ZonedTime => matches!(value, Value::ZonedTime(_)),
        GqlType::LocalTime => matches!(value, Value::LocalTime(_)),
        GqlType::Duration => matches!(value, Value::Duration(_)),
        GqlType::Record(_) => matches!(value, Value::Record(_) | Value::RecordTyped(_)),
        GqlType::List(inner) => {
            let Value::List(values) = value else {
                return false;
            };
            values.iter().all(|value| matches_gql_type(value, inner))
        }
        GqlType::Path => matches!(value, Value::Path(_)),
        GqlType::GraphRef => matches!(value, Value::GraphRef(_)),
        GqlType::NodeRef => matches!(value, Value::NodeRef(_)),
        GqlType::EdgeRef => matches!(value, Value::EdgeRef(_)),
        GqlType::TableRef => matches!(value, Value::TableRef(_)),
        GqlType::Null => matches!(value, Value::Null),
        GqlType::Nothing => false,
    }
}
