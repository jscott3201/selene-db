//! `selene.create_index` built-in.

use selene_core::IStr;
use selene_gql::{
    GqlType, MutationContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
    Value,
};
use selene_graph::{GraphError, TypedIndexKind};

use crate::builtin::{
    BuiltInMetadata, MutationProcedureBuiltIn, StaticOutputColumn, StaticParameter,
};

static CREATE_INDEX_PARAMS: [StaticParameter; 3] = [
    StaticParameter::new("label", GqlType::String, false),
    StaticParameter::new("property", GqlType::String, false),
    StaticParameter::new("kind", GqlType::String, false),
];

static CREATE_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

/// Built-in mutation-tier property-index creation procedure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeleneCreateIndex;

impl BuiltInMetadata for SeleneCreateIndex {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "create_index"]
    }

    fn tier(&self) -> ProcedureTier {
        ProcedureTier::Mutation
    }

    fn mutability(&self) -> ProcedureMutability {
        ProcedureMutability::SchemaWrite
    }

    fn signature_static(&self) -> &'static [StaticParameter] {
        &CREATE_INDEX_PARAMS
    }

    fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
        &CREATE_INDEX_OUTPUTS
    }
}

impl MutationProcedureBuiltIn for SeleneCreateIndex {
    fn execute(
        &self,
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

        match ctx.mutator().create_property_index(label, property, kind) {
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
    Ok(*value)
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

fn unit_result() -> ProcedureResult {
    ProcedureResult {
        rows: vec![Vec::new()],
    }
}
