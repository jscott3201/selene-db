//! `selene.feature_status` native built-in.

use selene_core::{Value, db_string};
use selene_profile::{CapabilityRecord, CapabilityStatus, capabilities, current_profile_identity};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

static FEATURE_STATUS_OUTPUTS: [StaticOutputColumn; 10] = [
    StaticOutputColumn::new("feature_id", GqlType::String)
        .with_description("ISO or implementation feature identifier."),
    StaticOutputColumn::new("status", GqlType::String).with_description("Runtime support state."),
    StaticOutputColumn::new("rationale", GqlType::String)
        .with_description("Non-support rationale or feature name."),
    StaticOutputColumn::new("feature_name", GqlType::String)
        .with_description("Canonical feature display name."),
    StaticOutputColumn::new("surface", GqlType::String)
        .with_description("ISO or namespaced extension surface."),
    StaticOutputColumn::new("profile_relation", GqlType::String)
        .with_description("Direct, implied, unselected, or extension profile relation."),
    StaticOutputColumn::new("claim_state", GqlType::String)
        .with_description("Generated conformance claim state."),
    StaticOutputColumn::new("evidence_status", GqlType::String)
        .with_description("Whether registered evidence references are present."),
    StaticOutputColumn::new("evidence_count", GqlType::Uint64)
        .with_description("Number of registered evidence references."),
    StaticOutputColumn::new("profile_hash", GqlType::String)
        .with_description("Canonical generated profile hash."),
];

pub(super) fn signature() -> Vec<ProcedureParameter> {
    let params: [StaticParameter; 0] = [];
    params
        .into_iter()
        .map(StaticParameter::into_parameter)
        .collect()
}

pub(super) fn output_columns() -> Vec<ProcedureOutputColumn> {
    FEATURE_STATUS_OUTPUTS
        .iter()
        .cloned()
        .map(StaticOutputColumn::into_output_column)
        .collect()
}

pub(super) fn execute(
    _ctx: &GraphContext<'_>,
    args: &[Value],
) -> Result<ProcedureResult, ProcedureError> {
    if !args.is_empty() {
        return Err(ProcedureError::InvalidArgument {
            detail: "selene.feature_status expects zero arguments".to_owned(),
        });
    }

    let profile_hash = current_profile_identity().canonical_hash();
    let rows = capabilities()
        .iter()
        .map(|record| row(record, profile_hash))
        .collect::<Result<Vec<_>, ProcedureError>>()?;
    Ok(ProcedureResult { rows })
}

fn row(
    record: &CapabilityRecord,
    profile_hash: &'static str,
) -> Result<Vec<Value>, ProcedureError> {
    let rationale = if record.status != CapabilityStatus::Supported {
        record.non_support_rationale
    } else {
        record.name
    };
    Ok(vec![
        string(record.id.as_str())?,
        string(record.status.as_str())?,
        string(rationale)?,
        string(record.name)?,
        string(record.surface.as_str())?,
        string(record.profile_relation.as_str())?,
        string(record.claim_state.as_str())?,
        string(record.evidence_status.as_str())?,
        Value::Uint(u64::try_from(record.evidence_count).unwrap_or(u64::MAX)),
        string(profile_hash)?,
    ])
}

fn string(value: &str) -> Result<Value, ProcedureError> {
    db_string(value)
        .map(Value::String)
        .map_err(|_err| ProcedureError::Internal {
            detail: "string construction failed during selene.feature_status".to_owned(),
        })
}
