//! `vector.drop_index` mutation procedure adapter.
//!
//! The optional `if_exists` argument defaults to `FALSE` through the analyzer's
//! trailing procedure-argument default materialization.

use std::sync::Arc;

use selene_core::Value;
use selene_gql::{
    GqlType, MutationContext, ProcedureDefaultValue, ProcedureError, ProcedureResult,
};
use selene_pack::{
    ExternalMutationProcedure, ExternalOutputColumn, ExternalParameter, ExternalProcedureMetadata,
};
use selene_vector::{DropIndexOutcome, VectorDropIndexV1};

use crate::{
    args::{expect_arity, required_string},
    error::invalid_argument,
    provider::{LIFECYCLE_PROVIDER_NAME, catalog_from_mutation, emit_payload_bytes},
    state::VectorPackState,
};

static DROP_INDEX_NAME: [&str; 2] = ["vector", "drop_index"];
const DROP_INDEX_PROC: &str = "vector.drop_index";

pub(crate) fn procedure(state: Arc<VectorPackState>) -> Arc<dyn ExternalMutationProcedure> {
    Arc::new(DropIndexProcedure { state })
}

struct DropIndexProcedure {
    state: Arc<VectorPackState>,
}

impl ExternalProcedureMetadata for DropIndexProcedure {
    fn name(&self) -> &'static [&'static str] {
        &DROP_INDEX_NAME
    }

    fn description(&self) -> &'static str {
        "Drop a vector index."
    }

    fn since_version(&self) -> &'static str {
        "1.1.0"
    }

    fn signature(&self) -> Vec<ExternalParameter> {
        vec![
            parameter("name", GqlType::String, false),
            parameter("if_exists", GqlType::Boolean, false)
                .with_default_doc("FALSE")
                .with_default(ProcedureDefaultValue::Boolean(false)),
        ]
    }

    fn output_columns(&self) -> Vec<ExternalOutputColumn> {
        Vec::new()
    }
}

impl ExternalMutationProcedure for DropIndexProcedure {
    fn execute(
        &self,
        ctx: &mut MutationContext<'_, '_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        let _state = &self.state;
        expect_arity(DROP_INDEX_PROC, args, 2)?;
        let name = required_string(DROP_INDEX_PROC, args, 0, "name")?;
        let if_exists = match &args[1] {
            Value::Bool(value) => *value,
            other => {
                return Err(invalid_argument(format!(
                    "{DROP_INDEX_PROC}: expected if_exists to be BOOLEAN, got {other:?}"
                )));
            }
        };
        let catalog = catalog_from_mutation(ctx, DROP_INDEX_PROC)?;
        match catalog.drop_index(&name)? {
            DropIndexOutcome::Hnsw | DropIndexOutcome::Ivf => {
                let bytes = VectorDropIndexV1.encode();
                emit_payload_bytes(ctx, DROP_INDEX_PROC, LIFECYCLE_PROVIDER_NAME, &name, bytes)?;
                Ok(ProcedureResult { rows: Vec::new() })
            }
            DropIndexOutcome::Missing if if_exists => Ok(ProcedureResult { rows: Vec::new() }),
            DropIndexOutcome::Missing => Err(invalid_argument(format!(
                "{DROP_INDEX_PROC}: vector index '{name}' does not exist"
            ))),
        }
    }
}

fn parameter(name: &'static str, ty: GqlType, nullable: bool) -> ExternalParameter {
    let parameter =
        ExternalParameter::new(name, ty, nullable).with_description("Procedure parameter.");
    if nullable {
        parameter.with_default_doc("NULL (use procedure default)")
    } else {
        parameter
    }
}
