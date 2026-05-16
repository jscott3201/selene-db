//! `selene.pack.history` built-in.

use std::sync::Arc;

use selene_core::{Change, IStr, PackLifecycleEvent, SchemaChange, Value, intern};
use selene_gql::{
    GqlType, GraphContext, ProcedureError, ProcedureMutability, ProcedureResult, ProcedureTier,
};
use selene_persist::PersistError;

use crate::builtin::{BuiltInMetadata, GraphProcedureBuiltIn, StaticOutputColumn, StaticParameter};
use crate::history::PackHistorySource;

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
    intern_value: impl FnMut(&str) -> Result<IStr, ProcedureError>,
) -> Result<Vec<Value>, ProcedureError> {
    event.to_history_row_with(intern_value)
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
