//! `selene.pack.history` built-in.

use std::sync::Arc;

use selene_core::{Change, IStr, PackLifecycleEvent, SchemaChange, Value, intern};
use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
};
use selene_persist::PersistError;

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};
use crate::history::PackHistorySource;
use crate::manifest::hex_lower;

// Static output schema. Nullability is implicit (per ProcedureOutputColumn contract):
//   - kind         : always populated
//   - pack_name    : NULL on ValidationFailed when manifest parse failed
//   - content_hash : "blake3:<64-hex>" on Staged/Activated, NULL on
//                    Deprecated/Disabled (no hash in payload per BRIEF-46 E63)
//                    and on ValidationFailed (no hash before validation succeeds)
//   - principal    : always populated
//   - reason       : NULL except on Deprecated
//   - error        : NULL except on ValidationFailed
//   - at           : always populated
static PACK_HISTORY_OUTPUTS: [StaticOutputColumn; 7] = [
    StaticOutputColumn {
        name: "kind",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "pack_name",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "content_hash",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "principal",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "reason",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "error",
        ty: GqlType::String,
    },
    StaticOutputColumn {
        name: "at",
        ty: GqlType::ZonedDateTime,
    },
];

/// Built-in read-only pack-history procedure.
#[derive(Clone)]
pub(crate) struct SelenePackHistory {
    source: Arc<dyn PackHistorySource>,
}

impl SelenePackHistory {
    pub(crate) fn new(source: Arc<dyn PackHistorySource>) -> Self {
        Self { source }
    }
}

impl BuiltInMetadata for SelenePackHistory {
    fn name(&self) -> &'static [&'static str] {
        &["selene", "pack", "history"]
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
        &PACK_HISTORY_OUTPUTS
    }
}

impl GraphProcedureBuiltIn for SelenePackHistory {
    fn execute(
        &self,
        ctx: &GraphContext<'_>,
        args: &[Value],
    ) -> Result<ProcedureResult, ProcedureError> {
        if !args.is_empty() {
            return Err(ProcedureError::InvalidArgument {
                detail: "selene.pack.history expects zero arguments".to_owned(),
            });
        }

        let current_graph = ctx.snapshot().graph_id();
        let reader = self.source.open_wal_reader()?;
        let stream = reader
            .iterate(|_header| true)
            .map_err(map_persist_err("pack history WAL iterate failed"))?;
        let mut rows = Vec::new();
        for view in stream {
            let view = view.map_err(map_persist_err("pack history WAL iterate failed"))?;
            let entry = view
                .into_entry()
                .map_err(map_persist_err("pack history WAL entry decode failed"))?;
            for change in entry.changes {
                if let Change::SchemaChanged {
                    graph,
                    change: SchemaChange::ProcedurePackLifecycle { event },
                    ..
                } = change
                    && graph == current_graph
                {
                    rows.push(event_to_row(&event)?);
                }
            }
        }

        Ok(ProcedureResult { rows })
    }
}

fn map_persist_err(detail: &'static str) -> impl FnOnce(PersistError) -> ProcedureError {
    move |source| ProcedureError::Internal {
        detail: format!("{detail}: {source}"),
    }
}

fn event_to_row(event: &PackLifecycleEvent) -> Result<Vec<Value>, ProcedureError> {
    event_to_row_with(event, intern_row_value)
}

pub(crate) fn event_to_row_with(
    event: &PackLifecycleEvent,
    mut intern_value: impl FnMut(&str) -> Result<IStr, ProcedureError>,
) -> Result<Vec<Value>, ProcedureError> {
    match event {
        PackLifecycleEvent::ValidationFailed {
            pack_name,
            principal,
            error,
            at,
        } => Ok(vec![
            string_value("validation_failed", &mut intern_value)?,
            optional_string_value(*pack_name),
            Value::Null,
            Value::String(*principal),
            Value::Null,
            external_string_value(error),
            timestamp_value(*at),
        ]),
        PackLifecycleEvent::Staged {
            pack_name,
            content_hash,
            principal,
            at,
        } => Ok(vec![
            string_value("staged", &mut intern_value)?,
            Value::String(*pack_name),
            external_string_value(&content_hash_string(content_hash)),
            Value::String(*principal),
            Value::Null,
            Value::Null,
            timestamp_value(*at),
        ]),
        PackLifecycleEvent::Activated {
            pack_name,
            content_hash,
            principal,
            at,
        } => Ok(vec![
            string_value("activated", &mut intern_value)?,
            Value::String(*pack_name),
            external_string_value(&content_hash_string(content_hash)),
            Value::String(*principal),
            Value::Null,
            Value::Null,
            timestamp_value(*at),
        ]),
        PackLifecycleEvent::Deprecated {
            pack_name,
            reason,
            principal,
            at,
        } => Ok(vec![
            string_value("deprecated", &mut intern_value)?,
            Value::String(*pack_name),
            Value::Null,
            Value::String(*principal),
            Value::String(*reason),
            Value::Null,
            timestamp_value(*at),
        ]),
        PackLifecycleEvent::Disabled {
            pack_name,
            principal,
            at,
        } => Ok(vec![
            string_value("disabled", &mut intern_value)?,
            Value::String(*pack_name),
            Value::Null,
            Value::String(*principal),
            Value::Null,
            Value::Null,
            timestamp_value(*at),
        ]),
        _ => Err(ProcedureError::Internal {
            detail: "unknown pack lifecycle event".to_owned(),
        }),
    }
}

fn optional_string_value(value: Option<IStr>) -> Value {
    value.map_or(Value::Null, Value::String)
}

fn string_value(
    value: &str,
    intern_value: &mut impl FnMut(&str) -> Result<IStr, ProcedureError>,
) -> Result<Value, ProcedureError> {
    intern_value(value).map(Value::String)
}

fn external_string_value(value: &str) -> Value {
    Value::ExternalString(Arc::from(value))
}

fn timestamp_value(at: jiff::Timestamp) -> Value {
    Value::ZonedDateTime(at.to_zoned(jiff::tz::TimeZone::UTC))
}

fn content_hash_string(hash: &[u8; 32]) -> String {
    let mut out = String::with_capacity("blake3:".len() + hash.len() * 2);
    out.push_str("blake3:");
    out.push_str(&hex_lower(hash));
    out
}

fn intern_row_value(value: &str) -> Result<IStr, ProcedureError> {
    intern(value).map_err(|_| ProcedureError::Internal {
        detail: "pack history row string interner cap exhausted".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use selene_core::intern;

    use super::*;

    #[test]
    fn event_to_row_intern_failure_surfaces_as_internal() {
        let event = PackLifecycleEvent::ValidationFailed {
            pack_name: None,
            principal: intern("pack_history.test.principal").unwrap(),
            error: "too many strings".to_owned(),
            at: Timestamp::from_second(1).unwrap(),
        };

        let err = event_to_row_with(&event, |_| {
            Err(ProcedureError::Internal {
                detail: "pack history row string interner cap exhausted".to_owned(),
            })
        })
        .expect_err("injected interner failure must fail projection");

        assert!(matches!(
            err,
            ProcedureError::Internal { detail }
                if detail == "pack history row string interner cap exhausted"
        ));
    }
}
