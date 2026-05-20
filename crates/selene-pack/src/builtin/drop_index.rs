//! `selene.drop_index` built-in.

use selene_core::IStr;
use selene_gql::{
    GqlType, MutationContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
    Value,
};

use crate::builtin::{
    BuiltInMetadata, MutationProcedureBuiltIn, StaticOutputColumn, StaticParameter,
};

static DROP_INDEX_PARAMS: [StaticParameter; 2] = [
    StaticParameter::new("label", GqlType::String, false).with_description("Node label."),
    StaticParameter::new("property", GqlType::String, false).with_description("Property name."),
];

static DROP_INDEX_OUTPUTS: [StaticOutputColumn; 0] = [];

/// Built-in mutation-tier property-index drop procedure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeleneDropIndex;

impl BuiltInMetadata for SeleneDropIndex {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "drop_index"]
    }

    fn description(&self) -> &'static str {
        "Drop a property index."
    }

    fn tier(&self) -> ProcedureTier {
        ProcedureTier::Mutation
    }

    fn mutability(&self) -> ProcedureMutability {
        ProcedureMutability::SchemaWrite
    }

    fn signature_static(&self) -> &'static [StaticParameter] {
        &DROP_INDEX_PARAMS
    }

    fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
        &DROP_INDEX_OUTPUTS
    }
}

impl MutationProcedureBuiltIn for SeleneDropIndex {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        if args.len() != 2 {
            return Err(invalid_arg("selene.drop_index expects exactly 2 arguments"));
        }
        let label = string_arg(&args[0], "label")?;
        let property = string_arg(&args[1], "property")?;

        ctx.mutator()
            .drop_property_index(label, property)
            .map_err(|source| ProcedureError::Internal {
                detail: format!("unexpected graph error during index drop: {source}"),
            })?;
        Ok(unit_result())
    }
}

fn string_arg(value: &Value, name: &'static str) -> Result<IStr, ProcedureError> {
    let Value::String(value) = value else {
        return Err(invalid_arg(format!(
            "selene.drop_index {name} must be a non-empty STRING"
        )));
    };
    if value.as_str().is_empty() {
        return Err(invalid_arg(format!(
            "selene.drop_index {name} must be a non-empty STRING"
        )));
    }
    Ok(*value)
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
