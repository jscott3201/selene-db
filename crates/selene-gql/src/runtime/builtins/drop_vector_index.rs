//! `selene.drop_vector_index` native built-in.
//!
//! Mutation-tier procedure dropping a vector index. The drop routes through
//! [`MutationContext::mutator`] → `Mutator::drop_vector_index`, which emits
//! `SchemaChange::VectorIndexDropped` through the single mutation funnel (Hard
//! Rule 11). It never bypasses the funnel and never re-enters `begin_write`.

use selene_core::{DbString, Value};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, MutationContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.drop_vector_index";

static DROP_VECTOR_INDEX_PARAMS: [StaticParameter; 2] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Vector property."),
];

static DROP_VECTOR_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    DROP_VECTOR_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    DROP_VECTOR_INDEX_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &mut MutationContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if args.len() != 2 {
        return Err(invalid_arg(format!(
            "{PROC_NAME} expects exactly 2 arguments"
        )));
    }
    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;

    ctx.mutator()
        .drop_vector_index(label, property)
        .map_err(|source| ProcedureError::Internal {
            detail: format!("unexpected graph error during vector index drop: {source}"),
        })?;
    Ok(unit_result())
}

fn string_arg(value: &Value, name: &'static str) -> Result<DbString, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    };
    if value.as_str().is_empty() {
        return Err(invalid_arg(format!(
            "{PROC_NAME} {name} must be a non-empty STRING"
        )));
    }
    Ok(value.clone())
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
