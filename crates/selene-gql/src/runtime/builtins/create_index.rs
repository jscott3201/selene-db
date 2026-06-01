//! `selene.create_index` native built-in.
//!
//! Mutation-tier procedure creating a property index. Ported verbatim from
//! the historical procedure-pack `create_index` built-in. Every write routes through
//! [`MutationContext::mutator`] → `Mutator::create_property_index`, which emits
//! `SchemaChange::PropertyIndexCreated` through the single mutation funnel (Hard
//! Rule 11). It never bypasses the funnel and never re-enters `begin_write`.

use selene_core::{IStr, Value};
use selene_graph::{GraphError, TypedIndexKind};

use super::meta::{StaticOutputColumn, StaticParameter};
use super::unit_result;
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, MutationContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

static CREATE_INDEX_PARAMS: [StaticParameter; 3] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
    StaticParameter::new("kind", GqlType::String, false).with_description("Index value kind."),
];

static CREATE_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    CREATE_INDEX_PARAMS
        .iter()
        .cloned()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    CREATE_INDEX_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    ctx: &mut MutationContext<'_, '_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if args.len() != 3 {
        return Err(invalid_arg(
            "selene.create_index expects exactly 3 arguments",
        ));
    }
    let label = string_arg(&args[0], "label")?;
    let property = string_arg(&args[1], "property")?;
    let kind = parse_kind(string_arg(&args[2], "kind")?)?;

    match ctx
        .mutator()
        .create_property_index(label.clone(), property.clone(), kind)
    {
        Ok(()) => Ok(unit_result()),
        Err(GraphError::PropertyIndexAlreadyExists { .. }) => Err(invalid_arg(format!(
            "index for ({label}, {property}) already exists"
        ))),
        Err(GraphError::IndexValueRejected { .. }) => Err(invalid_arg(
            "existing nodes contain values incompatible with the requested index kind",
        )),
        Err(other) => Err(ProcedureError::Internal {
            detail: format!("unexpected graph error during index creation: {other}"),
        }),
    }
}

fn string_arg(value: &Value, name: &'static str) -> Result<IStr, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!(
            "selene.create_index {name} must be a non-empty STRING"
        )));
    };
    if value.as_str().is_empty() {
        return Err(invalid_arg(format!(
            "selene.create_index {name} must be a non-empty STRING"
        )));
    }
    Ok(value.clone())
}

fn parse_kind(value: IStr) -> Result<TypedIndexKind, ProcedureError> {
    let raw = value.as_str();
    match raw.to_ascii_lowercase().as_str() {
        "i64" | "integer" | "int" => Ok(TypedIndexKind::I64),
        "f64" | "float" => Ok(TypedIndexKind::F64),
        "string" => Ok(TypedIndexKind::String),
        "date" => Ok(TypedIndexKind::Date),
        "local_datetime" | "localdatetime" => Ok(TypedIndexKind::LocalDateTime),
        "uuid" => Ok(TypedIndexKind::Uuid),
        _ => Err(invalid_arg(format!(
            "unknown index kind '{raw}'; expected one of i64, f64, string, date, local_datetime, uuid"
        ))),
    }
}

fn invalid_arg(detail: impl Into<String>) -> ProcedureError {
    ProcedureError::InvalidArgument {
        detail: detail.into(),
    }
}
