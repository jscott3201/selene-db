//! `selene.feature_status` native built-in.
//!
//! Walks `SUPPORTED_FEATURES` and `REFERENCED_FEATURES` from
//! `selene_core::feature_register` and emits one row per known feature. Ported
//! verbatim from `selene-pack/src/builtin/feature_status.rs`.

use std::collections::BTreeMap;

use selene_core::{
    Value,
    feature_register::{
        FeatureId, REFERENCED_FEATURES, SUPPORTED_FEATURES, is_supported, name_of,
        non_supported_rationale,
    },
    intern_with_admission,
};

use super::meta::{StaticOutputColumn, StaticParameter};
use crate::procedure_registry::ProcedureError;
use crate::{GqlType, GraphContext, ProcedureOutputColumn, ProcedureParameter, ProcedureResult};

static FEATURE_STATUS_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn::new("feature_id", GqlType::String)
        .with_description("ISO GQL feature identifier."),
    StaticOutputColumn::new("status", GqlType::String).with_description("Feature support state."),
    StaticOutputColumn::new("rationale", GqlType::String)
        .with_description("Support rationale or feature name."),
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

    let mut features = BTreeMap::<FeatureId, &'static str>::new();
    for (id, name) in REFERENCED_FEATURES {
        features.insert(*id, *name);
    }
    for id in SUPPORTED_FEATURES {
        features
            .entry(*id)
            .or_insert_with(|| name_of(*id).unwrap_or(""));
    }

    let rows = features
        .into_iter()
        .map(|(id, display)| {
            let (status, rationale) = feature_status(id, display);
            Ok(vec![
                string(id.as_str())?,
                string(status)?,
                string(rationale)?,
            ])
        })
        .collect::<Result<Vec<_>, ProcedureError>>()?;
    Ok(ProcedureResult { rows })
}

fn feature_status(id: FeatureId, display: &'static str) -> (&'static str, &'static str) {
    if is_supported(id) {
        ("supported", display)
    } else if let Some(rationale) = non_supported_rationale(id) {
        ("unsupported", rationale)
    } else {
        ("referenced", display)
    }
}

fn string(value: &str) -> Result<Value, ProcedureError> {
    intern_with_admission(value)
        .map(|(value, _was_new)| Value::String(value))
        .map_err(|_err| ProcedureError::Internal {
            detail: "interner cap exhausted during selene.feature_status".to_owned(),
        })
}
