//! `selene.create_text_index` native built-in.
//!
//! Mutation-tier procedure creating a maintained BM25 text index. Every write
//! routes through [`MutationContext::mutator`] → `Mutator::create_text_index_named`,
//! which emits `SchemaChange::TextIndexCreated` through the single mutation
//! funnel (Hard Rule 11).

use selene_core::{DbString, Value};
use selene_graph::GraphError;

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, MutationContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.create_text_index";

static CREATE_TEXT_INDEX_PARAMS: [StaticParameter; 3] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Text property."),
    StaticParameter::new("name", GqlType::String, true)
        .with_description("Optional catalog name.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
];

static CREATE_TEXT_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    CREATE_TEXT_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    CREATE_TEXT_INDEX_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &mut MutationContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(2..=3).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 2 to 3 arguments")));
    }
    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;
    let name = args.get(2).map(name_arg).transpose()?.flatten();

    match ctx
        .mutator()
        .create_text_index_named(label.clone(), property.clone(), name)
    {
        Ok(()) => Ok(unit_result()),
        Err(GraphError::TextIndexAlreadyExists { .. }) => Err(invalid_arg(format!(
            "text index for ({label}, {property}) already exists"
        ))),
        Err(other) => Err(ProcedureError::Internal {
            detail: format!("unexpected graph error during text index creation: {other}"),
        }),
    }
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

fn name_arg(value: &Value) -> Result<Option<DbString>, ProcedureError> {
    match value {
        Value::Null => Ok(None),
        Value::String(value) if !value.as_str().is_empty() => Ok(Some(value.clone())),
        Value::String(_) => Err(invalid_arg(format!(
            "{PROC_NAME} name must be NULL or a non-empty STRING"
        ))),
        _ => Err(invalid_arg(format!(
            "{PROC_NAME} name must be NULL or a non-empty STRING"
        ))),
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
