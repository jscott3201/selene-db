use selene_core::Value;

use crate::{
    GqlType, PlannedCall, ProcedureError, YieldKind,
    runtime::{ExecutorError, value_type_match::value_matches_gql_type},
};

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

fn output_column_index(call: &PlannedCall, name: selene_core::DbString) -> Option<usize> {
    call.output_schema
        .columns
        .iter()
        .position(|column| column.name == name)
}

fn matches_gql_type(value: &Value, ty: &GqlType) -> bool {
    if matches!(ty, GqlType::Nothing) {
        return false;
    }
    // Output schemas do not model nullability yet, and native metadata procedures use
    // NULL in typed columns for optional fields.
    if matches!(value, Value::Null) {
        return true;
    }

    value_matches_gql_type(value, ty)
}
