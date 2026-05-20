//! `selene.feature_status` built-in.
//!
//! Walks `SUPPORTED_FEATURES` and `REFERENCED_FEATURES` from
//! `selene_core::feature_register` and emits one row per known feature.

use std::collections::BTreeMap;

use selene_core::{
    Value,
    feature_register::{
        FeatureId, REFERENCED_FEATURES, SUPPORTED_FEATURES, is_supported, name_of,
        non_supported_rationale,
    },
    intern_with_admission,
};
use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
};

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};

static FEATURE_STATUS_OUTPUTS: [StaticOutputColumn; 3] = [
    StaticOutputColumn {
        name: "feature_id",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "status",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "rationale",
        ty: GqlType::String,
    },
];

/// Built-in read-only feature status procedure.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeleneFeatureStatus;

impl BuiltInMetadata for SeleneFeatureStatus {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "feature_status"]
    }

    fn tier(&self) -> ProcedureTier {
        ProcedureTier::Graph
    }

    fn mutability(&self) -> ProcedureMutability {
        ProcedureMutability::Read
    }

    fn signature_static(&self) -> &'static [StaticParameter] {
        &[]
    }

    fn output_columns_static(&self) -> &'static [StaticOutputColumn] {
        &FEATURE_STATUS_OUTPUTS
    }
}

impl GraphProcedureBuiltIn for SeleneFeatureStatus {
    fn execute(
        &self,
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
