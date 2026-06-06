//! SHOW PROCEDURES row rendering.

use selene_core::{DbString, Value};

use crate::{
    Binding, ExecutorError, ProcedureMetadata, ProcedureMutability, ProcedureSignature,
    ProcedureTier,
};

use super::{render_gql_type, runtime_db_string};

pub(super) fn procedure_row(
    name: &[DbString],
    metadata: &ProcedureMetadata,
) -> Result<Binding, ExecutorError> {
    let name = render_procedure_name(name);
    Ok(Binding::new([
        Value::String(runtime_db_string(&name)?),
        Value::String(runtime_db_string(render_tier(metadata.tier))?),
        Value::String(runtime_db_string(render_mutability(metadata.mutability))?),
        Value::String(runtime_db_string(&render_signature(
            &name,
            &metadata.signature,
        ))?),
        Value::String(runtime_db_string(metadata.description)?),
        Value::String(runtime_db_string(metadata.signature.since_version)?),
    ]))
}

pub(super) fn render_procedure_name(name: &[DbString]) -> String {
    name.iter()
        .map(|part| part.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn render_tier(tier: ProcedureTier) -> &'static str {
    match tier {
        ProcedureTier::Graph => "graph",
        ProcedureTier::Mutation => "mutation",
        ProcedureTier::Maintenance => "maintenance",
    }
}

fn render_mutability(mutability: ProcedureMutability) -> &'static str {
    match mutability {
        ProcedureMutability::Read => "read",
        ProcedureMutability::SchemaWrite => "schema_write",
        ProcedureMutability::MaintenanceWrite => "maintenance_write",
    }
}

fn render_signature(name: &str, signature: &ProcedureSignature) -> String {
    let params = signature
        .parameters
        .iter()
        .map(|parameter| {
            let nullable = if parameter.nullable { "?" } else { "" };
            format!(
                "{}: {}{}",
                parameter.name,
                render_gql_type(&parameter.ty),
                nullable
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({params})")
}
