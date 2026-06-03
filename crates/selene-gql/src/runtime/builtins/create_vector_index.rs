//! `selene.create_vector_index` native built-in.
//!
//! Mutation-tier procedure creating a vector index. Every write routes through
//! [`MutationContext::mutator`] → `Mutator::create_vector_index_named`, which
//! emits `SchemaChange::VectorIndexCreated` through the single mutation funnel
//! (Hard Rule 11). It never bypasses the funnel and never re-enters
//! `begin_write`.

use selene_core::{IStr, Value};
use selene_graph::{GraphError, VectorIndexKind};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{
    GqlType, MutationContext, ProcedureDefaultValue, ProcedureOutputColumn, ProcedureParameter,
    ProcedureResult,
};

const PROC_NAME: &str = "selene.create_vector_index";

static CREATE_VECTOR_INDEX_PARAMS: [StaticParameter; 5] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Vector property."),
    StaticParameter::new("dimension", GqlType::Integer, false)
        .with_description("Required vector dimensionality."),
    StaticParameter::new("kind", GqlType::String, false)
        .with_description("Vector index algorithm kind.")
        .with_default_doc("flat")
        .with_default(ProcedureDefaultValue::String("flat")),
    StaticParameter::new("name", GqlType::String, true)
        .with_description("Optional catalog name.")
        .with_default_doc("NULL")
        .with_default(ProcedureDefaultValue::Null),
];

static CREATE_VECTOR_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    CREATE_VECTOR_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    CREATE_VECTOR_INDEX_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &mut MutationContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !(3..=5).contains(&args.len()) {
        return Err(invalid_arg(format!("{PROC_NAME} expects 3 to 5 arguments")));
    }
    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;
    let dimension = dimension_arg(&args[2])?;
    let kind = args
        .get(3)
        .map(kind_arg)
        .transpose()?
        .unwrap_or(VectorIndexKind::Flat);
    let name = args.get(4).map(name_arg).transpose()?.flatten();

    match ctx.mutator().create_vector_index_named(
        label.clone(),
        property.clone(),
        kind,
        dimension,
        name,
    ) {
        Ok(()) => Ok(unit_result()),
        Err(GraphError::VectorIndexAlreadyExists { .. }) => Err(invalid_arg(format!(
            "vector index for ({label}, {property}) already exists"
        ))),
        Err(GraphError::VectorIndexInvalidDimension { .. }) => Err(invalid_arg(
            "vector index dimension must be greater than zero",
        )),
        Err(GraphError::VectorIndexValueRejected { observed, .. }) => Err(invalid_arg(format!(
            "existing nodes contain values incompatible with the requested vector index: {observed}"
        ))),
        Err(other) => Err(ProcedureError::Internal {
            detail: format!("unexpected graph error during vector index creation: {other}"),
        }),
    }
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

fn dimension_arg(value: &Value) -> Result<u32, ProcedureError> {
    let dimension = match value {
        Value::Int(value) => u32::try_from(*value).ok(),
        Value::Uint(value) => u32::try_from(*value).ok(),
        _ => None,
    }
    .ok_or_else(|| invalid_arg(format!("{PROC_NAME} dimension must be a positive INTEGER")))?;
    if dimension == 0 {
        return Err(invalid_arg(format!(
            "{PROC_NAME} dimension must be a positive INTEGER"
        )));
    }
    Ok(dimension)
}

fn kind_arg(value: &Value) -> Result<VectorIndexKind, ProcedureError> {
    let raw = string_arg(value, "kind")?;
    match raw.as_str().to_ascii_lowercase().as_str() {
        "flat" => Ok(VectorIndexKind::Flat),
        other => Err(invalid_arg(format!(
            "unknown vector index kind '{other}'; expected flat"
        ))),
    }
}

fn name_arg(value: &Value) -> Result<Option<IStr>, ProcedureError> {
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
