//! `selene.drop_text_index` native built-in.
//!
//! Mutation-tier procedure dropping a maintained BM25 text index through the
//! single mutation funnel.

use selene_core::{IStr, Value};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, MutationContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

const PROC_NAME: &str = "selene.drop_text_index";

static DROP_TEXT_INDEX_PARAMS: [StaticParameter; 2] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Text property."),
];

static DROP_TEXT_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    DROP_TEXT_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    DROP_TEXT_INDEX_OUTPUTS
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
        .drop_text_index(label, property)
        .map_err(|source| ProcedureError::Internal {
            detail: format!("unexpected graph error during text index drop: {source}"),
        })?;
    Ok(unit_result())
}

fn string_arg(value: &Value, name: &'static str) -> Result<IStr, ProcedureError> {
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
